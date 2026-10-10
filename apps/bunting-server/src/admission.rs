//! Latency-ordered admission for the venue (ADR 0035).
//!
//! Each connection's reader stamps inbound bytes the moment they arrive (so
//! the team's real network delay is inside the stamp), the session maps and
//! admits them without waiting, and one sequencer thread releases admitted
//! work in `(release, arrival)` order at `t_rx + L(p, v)`: the team's
//! virtual distance to the addressed venue. It is the only thread that
//! commits, and it returns the result to the session, which
//! sends every venue message `L(v, p)` after the venue produced it.
//! Built-in agents submit through the same path from their own location
//! (`scenario.rs`).

use crate::config::AdmissionConfig;
use crate::distributor::PublishingOrigin;
use crate::run_clock::RunClock;
use crate::wake::Waker;
use bunting_admission_sequencer::{
    AdmissionError, AdmissionRecord, DelayEstimator, Endpoint, LatencyModel, RttSource, Sequencer,
};
use bunting_market_types::ParticipantId;
use serde::Serialize;
use simfix_wire::FixMessage;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::SyncSender;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Monotonic venue clock: microseconds since server start. It times the
/// network (arrivals, releases, deliveries); the run's own logical clock is
/// [`RunClock`]'s mapping of it (ADR 0037).
#[derive(Clone, Copy, Debug)]
pub(crate) struct VenueClock {
    origin: Instant,
}

impl VenueClock {
    pub(crate) fn start() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    pub(crate) fn now_us(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_micros()).unwrap_or(u64::MAX)
    }
}

/// Everything a released job may use. The job runs on the sequencer thread,
/// the venue's only committer.
pub(crate) struct JobContext<'a> {
    pub(crate) origin: &'a PublishingOrigin,
    pub(crate) clock: &'a VenueClock,
    pub(crate) admission: &'a AdmissionRecord,
    /// Stamps the job with its run time; see [`JobContext::run_time`].
    pub(crate) run_clock: &'a RunClock,
}

/// A session's work, producing messages in its protocol (`M`: FIX or BNP).
pub(crate) type JobWork<M = FixMessage> =
    Box<dyn FnOnce(&JobContext<'_>) -> Result<Vec<M>, String> + Send>;

/// Released work: runs on the sequencer thread and delivers its own result
/// without blocking.
pub(crate) type Task = Box<dyn FnOnce(&JobContext<'_>) + Send>;

/// A session's work: its reply goes back on `reply`, from `source`.
pub(crate) fn reply_task<M: Send + 'static>(
    work: JobWork<M>,
    reply: SyncSender<Reply<M>>,
    waker: Waker,
    source: Endpoint,
    feed: Option<String>,
) -> Task {
    Box::new(move |context| {
        let result = work(context);
        // A full or closed reply channel means the session already
        // disconnected; its committed effects stand.
        let _ = reply.try_send(Reply {
            result,
            completed_us: context.clock.now_us(),
            source,
            published_before: context.origin.distributor().published(),
            feed,
        });
        waker.wake();
    })
}

/// One inbound unit of work as the session saw it.
pub(crate) struct Inbound<'a> {
    pub(crate) received_us: u64,
    pub(crate) estimator: &'a DelayEstimator,
    pub(crate) participant: ParticipantId,
    /// A venue, or the hub for organizer requests.
    pub(crate) destination: Endpoint,
    /// The connection's previous release to the same destination: one path
    /// delivers in order, different venues' paths do not.
    pub(crate) floor_us: u64,
}

/// The outcome of one released job, returned to its session.
pub(crate) struct Reply<M = FixMessage> {
    pub(crate) result: Result<Vec<M>, String>,
    /// Venue time at which the job finished (the commit time of any command).
    pub(crate) completed_us: u64,
    /// Where the work ran (a venue, or the hub): its response travels back
    /// over the virtual path from there.
    pub(crate) source: Endpoint,
    /// Committed batches published before the job finished (the sequencer
    /// thread is the only publisher): a snapshot in the reply reflects
    /// exactly those, so a feed continues from this ordinal.
    pub(crate) published_before: u64,
    /// The market-data subscription this reply's snapshot starts.
    pub(crate) feed: Option<String>,
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
    queue: Sequencer<Task>,
}

pub(crate) struct AdmissionService {
    clock: VenueClock,
    run_clock: RunClock,
    config: AdmissionConfig,
    inner: Mutex<Inner>,
    wake: Condvar,
    connections: Mutex<BTreeMap<u64, ConnectionHealth>>,
    next_connection: AtomicU64,
    /// Participants with a live session, per interface; see
    /// [`Self::claim_participant`].
    sessions: Mutex<BTreeSet<(Interface, ParticipantId)>>,
    session_ended: Condvar,
}

/// A participant interface. Each allows one live session per participant.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum Interface {
    Fix,
    Bnp,
}

impl Interface {
    const fn name(self) -> &'static str {
        match self {
            Self::Fix => "FIX",
            Self::Bnp => "BNP",
        }
    }
}

/// Exclusive ownership of one participant's session on one interface,
/// released when the session (and its final persisted snapshot) is done.
pub(crate) struct ParticipantClaim<'a> {
    admission: &'a AdmissionService,
    key: (Interface, ParticipantId),
}

impl Drop for ParticipantClaim<'_> {
    fn drop(&mut self) {
        if let Ok(mut sessions) = self.admission.sessions.lock() {
            sessions.remove(&self.key);
        }
        self.admission.session_ended.notify_all();
    }
}

impl AdmissionService {
    pub(crate) fn new(clock: VenueClock, config: AdmissionConfig) -> Result<Self, String> {
        let model = LatencyModel::new(config.map.clone())
            .map_err(|error| format!("invalid latency map: {error}"))?;
        let queue = Sequencer::new(config.max_admission_queue);
        Ok(Self {
            clock,
            run_clock: RunClock::default(),
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

    pub(crate) const fn run_clock(&self) -> &RunClock {
        &self.run_clock
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
        task: Task,
    ) -> Result<AdmissionRecord, AdmissionError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AdmissionError::InvalidPolicy("admission lock poisoned"))?;
        let mut record = inner.model.decide(
            inbound.received_us,
            inbound.estimator,
            inbound.participant,
            inbound.destination,
        )?;
        record.release_us = record.release_us.max(inbound.floor_us);
        let record = inner.queue.admit(self.clock.now_us(), record, task)?;
        drop(inner);
        self.wake.notify_all();
        Ok(record)
    }

    /// Queues the venue's own work (the timer's clock tick) for release at
    /// venue instant `at_us`. It travels no path: the timer is at the venue.
    ///
    /// # Errors
    /// Returns the queue limit when admission is refused.
    pub(crate) fn schedule(
        &self,
        at_us: u64,
        task: Task,
    ) -> Result<AdmissionRecord, AdmissionError> {
        let record = AdmissionRecord {
            received_us: at_us,
            measured_one_way_us: None,
            rtt_source: RttSource::None,
            destination: Endpoint::Hub,
            path_latency_us: 0,
            jitter_position: None,
            release_us: at_us,
            arrival_sequence: 0,
        };
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| AdmissionError::InvalidPolicy("admission lock poisoned"))?;
        let record = inner.queue.admit(self.clock.now_us(), record, task)?;
        drop(inner);
        self.wake.notify_all();
        Ok(record)
    }

    /// `L(source, participant)`: how long a message from a venue, the hub or
    /// another team travels the virtual path before it is written to
    /// `participant`'s real connection.
    pub(crate) fn delay_to_us(
        &self,
        source: Endpoint,
        participant: ParticipantId,
    ) -> Result<u64, String> {
        self.delay_us(source, Endpoint::Participant(participant))
    }

    /// `L(from, to)` between any two endpoints, e.g. a venue's path to the
    /// consolidated tape's processor at the hub.
    pub(crate) fn delay_us(&self, from: Endpoint, to: Endpoint) -> Result<u64, String> {
        self.inner
            .lock()
            .map_err(|_| "admission lock poisoned".to_owned())?
            .model
            .delay_us(from, to)
            .map_err(|error| error.to_string())
    }

    /// Releases due work forever. Only one thread may run this.
    pub(crate) fn run_sequencer(&self, origin: &PublishingOrigin) -> Result<(), String> {
        loop {
            let (record, task) = {
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
            task(&JobContext {
                origin,
                clock: &self.clock,
                admission: &record,
                run_clock: &self.run_clock,
            });
        }
    }

    /// One live session per participant and interface, as on a real
    /// venue: a new logon waits up to `wait` for the previous session to
    /// finish closing (so its last persisted sequence numbers are the ones
    /// restored), and is refused while that session is still connected.
    pub(crate) fn claim_participant(
        &self,
        interface: Interface,
        participant: ParticipantId,
        wait: Duration,
    ) -> Result<ParticipantClaim<'_>, String> {
        let key = (interface, participant);
        let deadline = Instant::now() + wait;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "session registry lock poisoned".to_owned())?;
        while sessions.contains(&key) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(format!(
                    "participant {participant} already has an active {} session",
                    interface.name()
                ));
            }
            sessions = self
                .session_ended
                .wait_timeout(sessions, remaining)
                .map_err(|_| "session registry lock poisoned".to_owned())?
                .0;
        }
        sessions.insert(key);
        Ok(ParticipantClaim {
            admission: self,
            key,
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
            "latencyMap": self.config.map,
            "queued": queued,
            "connections": connections,
        })
    }
}
