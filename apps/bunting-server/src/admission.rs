//! Real-life latency admission for the venue (ADR 0035).
//!
//! Each connection's reader stamps inbound bytes the moment they arrive (so
//! the team's real network delay is inside the stamp), the session maps and
//! admits them without waiting, and one sequencer thread releases admitted
//! work in `(release, arrival)` order at `t_rx + L(p, v)`: the team's
//! virtual distance to the addressed venue. It executes under the
//! authoritative writer gate and returns the result to the session, which
//! sends every venue message `L(v, p)` after the venue produced it.

use crate::config::AdmissionConfig;
use crate::distributor::PublishingOrigin;
use crate::wake::Waker;
use crate::writer::AuthoritativeWriter;
use bunting_admission_sequencer::{
    AdmissionError, AdmissionRecord, DelayEstimator, LatencyModel, RttSource, Sequencer,
};
use bunting_market_types::{LogicalTimeNs, ParticipantId, VenueId};
use serde::Serialize;
use simfix_wire::FixMessage;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Monotonic venue clock: microseconds since server start, mapped to epoch
/// nanoseconds for logical time.
#[derive(Clone, Copy, Debug)]
pub(crate) struct VenueClock {
    origin: Instant,
    epoch_ns_at_origin: u64,
}

impl VenueClock {
    pub(crate) fn start() -> Self {
        let epoch_ns_at_origin = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
            });
        Self {
            origin: Instant::now(),
            epoch_ns_at_origin,
        }
    }

    pub(crate) fn now_us(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    /// Logical time of a venue instant: epoch nanoseconds, monotonic.
    pub(crate) fn logical_time(&self, venue_us: u64) -> LogicalTimeNs {
        LogicalTimeNs::new(
            self.epoch_ns_at_origin
                .saturating_add(venue_us.saturating_mul(1_000)),
        )
    }
}

/// Everything a released job may use. The job runs on the sequencer thread
/// while it holds the writer gate.
pub(crate) struct JobContext<'a> {
    pub(crate) origin: &'a PublishingOrigin,
    pub(crate) admission: &'a AdmissionRecord,
    /// Logical time of the release: the command's venue time.
    pub(crate) logical_time: LogicalTimeNs,
}

pub(crate) type JobWork =
    Box<dyn FnOnce(&JobContext<'_>) -> Result<Vec<FixMessage>, String> + Send>;

/// One inbound unit of work as the session saw it.
pub(crate) struct Inbound<'a> {
    pub(crate) received_us: u64,
    pub(crate) estimator: &'a DelayEstimator,
    pub(crate) participant: ParticipantId,
    pub(crate) venue: Option<VenueId>,
    /// The connection's previous release to the same venue: one path
    /// delivers in order, different venues' paths do not.
    pub(crate) floor_us: u64,
}

/// The outcome of one released job, returned to its session.
pub(crate) struct Reply {
    pub(crate) result: Result<Vec<FixMessage>, String>,
    /// Venue time at which the job finished (the commit time of any command).
    pub(crate) completed_us: u64,
    /// The venue the work addressed: its response travels back over that
    /// venue's simulated path (ADR 0030 §4).
    pub(crate) venue: Option<VenueId>,
}

struct Job {
    work: JobWork,
    reply: SyncSender<Reply>,
    waker: Waker,
    venue: Option<VenueId>,
}

/// One connection's published access latency (ADR 0035 §2). Teams add
/// the published virtual table to it for each venue.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConnectionHealth {
    pub(crate) participant_id: String,
    pub(crate) rtt_source: RttSource,
    pub(crate) kernel_min_rtt_us: Option<u64>,
    pub(crate) probe_min_rtt_us: Option<u64>,
    pub(crate) probe_samples: u64,
    /// Measured one-way access latency to the server.
    pub(crate) access_one_way_us: Option<u64>,
}

struct Inner {
    model: LatencyModel,
    queue: Sequencer<Job>,
}

pub(crate) struct AdmissionService {
    clock: VenueClock,
    config: AdmissionConfig,
    inner: Mutex<Inner>,
    wake: Condvar,
    connections: Mutex<BTreeMap<u64, ConnectionHealth>>,
    next_connection: AtomicU64,
    /// Participants with a live FIX session; see [`Self::claim_participant`].
    sessions: Mutex<BTreeSet<ParticipantId>>,
    session_ended: Condvar,
}

/// Exclusive ownership of one participant's FIX session state, released
/// when the session (and its final persisted snapshot) is done.
pub(crate) struct ParticipantClaim<'a> {
    admission: &'a AdmissionService,
    participant: ParticipantId,
}

impl Drop for ParticipantClaim<'_> {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.admission.sessions.lock() {
            sessions.remove(&self.participant);
        }
        self.admission.session_ended.notify_all();
    }
}

impl AdmissionService {
    pub(crate) fn new(clock: VenueClock, config: AdmissionConfig) -> Result<Self, String> {
        let model = LatencyModel::new(config.policy.clone())
            .map_err(|error| format!("invalid admission policy: {error}"))?;
        let queue = Sequencer::new(config.max_admission_queue);
        Ok(Self {
            clock,
            config,
            inner: Mutex::new(Inner { model, queue }),
            wake: Condvar::new(),
            connections: Mutex::new(BTreeMap::new()),
            next_connection: AtomicU64::new(1),
            sessions: Mutex::new(BTreeSet::new()),
            session_ended: Condvar::new(),
        })
    }

    pub(crate) const fn clock(&self) -> &VenueClock {
        &self.clock
    }

    pub(crate) const fn config(&self) -> &AdmissionConfig {
        &self.config
    }

    /// Decides and queues one inbound unit of work received at
    /// `received_us`. `floor_us` keeps one connection's releases in arrival
    /// order (TCP delivers in order; jitter must not reorder a session).
    ///
    /// # Errors
    /// Returns the named queue or stream limit when admission is refused.
    pub(crate) fn admit(
        &self,
        inbound: &Inbound<'_>,
        work: JobWork,
        reply: SyncSender<Reply>,
        waker: Waker,
    ) -> Result<AdmissionRecord, AdmissionError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AdmissionError::InvalidPolicy("admission lock poisoned"))?;
        let mut record = inner.model.decide(
            inbound.received_us,
            inbound.estimator,
            inbound.participant,
            inbound.venue,
        )?;
        record.release_us = record.release_us.max(inbound.floor_us);
        let record = inner.queue.admit(
            self.clock.now_us(),
            record,
            Job {
                work,
                reply,
                waker,
                venue: inbound.venue,
            },
        )?;
        drop(inner);
        self.wake.notify_all();
        Ok(record)
    }

    /// `L(v, p)`: how long a venue's message to `participant` travels the
    /// virtual path before it is written to the team's real connection.
    pub(crate) fn outbound_delay_us(
        &self,
        participant: ParticipantId,
        venue: Option<VenueId>,
    ) -> Result<u64, String> {
        self.inner
            .lock()
            .map_err(|_| "admission lock poisoned".to_owned())?
            .model
            .outbound_delay_us(participant, venue)
            .map_err(|error| error.to_string())
    }

    /// Releases due work forever. Only one thread may run this.
    pub(crate) fn run_sequencer(
        &self,
        origin: &PublishingOrigin,
        writer: &AuthoritativeWriter,
    ) -> Result<(), String> {
        loop {
            let (record, job) = {
                let mut inner = self
                    .inner
                    .lock()
                    .map_err(|_| "admission lock poisoned".to_owned())?;
                loop {
                    if let Some(due) = inner.queue.pop_due(self.clock.now_us()) {
                        break due;
                    }
                    let wait = inner.queue.next_release_us().map_or(
                        Duration::from_millis(100),
                        |release| {
                            Duration::from_micros(release.saturating_sub(self.clock.now_us()))
                        },
                    );
                    inner = self
                        .wake
                        .wait_timeout(inner, wait.max(Duration::from_micros(50)))
                        .map_err(|_| "admission lock poisoned".to_owned())?
                        .0;
                }
            };
            let result = {
                let _gate = writer.lock()?;
                (job.work)(&JobContext {
                    origin,
                    admission: &record,
                    logical_time: self.clock.logical_time(record.release_us),
                })
            };
            // A full or closed reply channel means the session already
            // disconnected; its committed effects stand.
            let _ = job.reply.try_send(Reply {
                result,
                completed_us: self.clock.now_us(),
                venue: job.venue,
            });
            job.waker.wake();
        }
    }

    /// One live session per participant, as on a real FIX venue: a new
    /// logon waits up to `wait` for the previous session to finish closing
    /// (so its last persisted sequence numbers are the ones restored), and
    /// is refused while that session is still connected.
    pub(crate) fn claim_participant(
        &self,
        participant: ParticipantId,
        wait: Duration,
    ) -> Result<ParticipantClaim<'_>, String> {
        let deadline = Instant::now() + wait;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "session registry lock poisoned".to_owned())?;
        while sessions.contains(&participant) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "participant {participant} already has an active FIX session"
                ));
            }
            sessions = self
                .session_ended
                .wait_timeout(sessions, remaining)
                .map_err(|_| "session registry lock poisoned".to_owned())?
                .0;
        }
        sessions.insert(participant);
        Ok(ParticipantClaim {
            admission: self,
            participant,
        })
    }

    pub(crate) fn register_connection(&self) -> u64 {
        self.next_connection.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn report_connection(&self, connection: u64, health: ConnectionHealth) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.insert(connection, health);
        }
    }

    pub(crate) fn remove_connection(&self, connection: u64) {
        if let Ok(mut connections) = self.connections.lock() {
            connections.remove(&connection);
        }
    }

    /// Health snapshot for the admin endpoint.
    pub(crate) fn health(&self) -> serde_json::Value {
        let connections = self
            .connections
            .lock()
            .map(|connections| connections.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let queued = self.inner.lock().map_or(0, |inner| inner.queue.len());
        serde_json::json!({
            "virtualLatency": self.config.policy,
            "queued": queued,
            "connections": connections,
        })
    }
}
