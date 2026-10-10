//! Latency-modeled admission for the venue (ADR 0030 as amended by ADR 0034).
//!
//! Sessions stamp each inbound read with the venue clock, map it, and admit
//! it here without waiting. One sequencer thread releases admitted work in
//! `(release, arrival)` order when the venue clock reaches its release time,
//! executes it under the authoritative writer gate and returns the result
//! to the session, which holds every outbound message until
//! `commit + (D − d̂) + L` so all participants see it together.

use crate::config::{AdmissionConfig, RttSources};
use crate::distributor::PublishingOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_admission_sequencer::{
    AdmissionError, AdmissionRecord, DelayEstimator, LatencyModel, RttSource, Sequencer,
};
use bunting_market_types::{LogicalTimeNs, ParticipantId, VenueId};
use serde::Serialize;
use simfix_wire::FixMessage;
use std::collections::BTreeMap;
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
    /// The connection's previous release: one session stays in TCP order.
    pub(crate) floor_us: u64,
}

/// The outcome of one released job, returned to its session.
pub(crate) struct Reply {
    pub(crate) result: Result<Vec<FixMessage>, String>,
    /// Venue time at which the job finished (the commit time of any command).
    pub(crate) completed_us: u64,
}

struct Job {
    work: JobWork,
    reply: SyncSender<Reply>,
}

/// One connection's published latency figures (ADR 0034 operational impact).
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ConnectionHealth {
    pub(crate) participant_id: String,
    pub(crate) rtt_source: RttSource,
    pub(crate) kernel_min_rtt_us: Option<u64>,
    pub(crate) probe_min_rtt_us: Option<u64>,
    pub(crate) probe_samples: u64,
    pub(crate) one_way_delay_us: u64,
    /// `d̂ ≥ D`: this participant is farther than the venue compensates.
    pub(crate) beyond_max_one_way_delay: bool,
    /// Probe RTT far above kernel RTT: slow probe replies, or an
    /// unconfigured TCP-terminating proxy.
    pub(crate) probe_far_above_kernel: bool,
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
        })
    }

    pub(crate) const fn clock(&self) -> &VenueClock {
        &self.clock
    }

    pub(crate) const fn config(&self) -> &AdmissionConfig {
        &self.config
    }

    pub(crate) fn uses_kernel_rtt(&self) -> bool {
        self.config.rtt_sources == RttSources::KernelAndProbe
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
        let record = inner
            .queue
            .admit(self.clock.now_us(), record, Job { work, reply })?;
        drop(inner);
        self.wake.notify_all();
        Ok(record)
    }

    /// `(D − d̂) + L(v, p)` for one outbound message to `participant`.
    pub(crate) fn outbound_hold_us(
        &self,
        estimator: &DelayEstimator,
        participant: ParticipantId,
        venue: Option<VenueId>,
    ) -> Result<u64, String> {
        self.inner
            .lock()
            .map_err(|_| "admission lock poisoned".to_owned())?
            .model
            .outbound_hold_us(estimator, participant, venue)
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
            });
        }
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
            "policy": self.config.policy,
            "rttSources": self.config.rtt_sources,
            "queued": queued,
            "connections": connections,
        })
    }
}
