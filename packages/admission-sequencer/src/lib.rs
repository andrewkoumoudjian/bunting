#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Latency for venue admission (ADR 0035), sans-I/O and clock-free.
//!
//! Nothing is equalized. A team's real network delay to the server (its
//! connection method, TCP stack, uplink and distance) counts in full, as it
//! does for a trading desk, and the scenario adds each team's virtual
//! distance to each venue on top, in both directions:
//!
//! ```text
//! inbound:   release  = t_rx + L(p, v)      # t_rx already includes real delay
//! outbound:  send_at  = commit + L(v, p)    # real delay then applies on the wire
//! ```
//!
//! `L(p, v)` is a configured path latency plus a seeded jitter draw. Real
//! delay is still measured ([`DelayEstimator`]) and published as each team's
//! access latency; it never changes ordering, so inflating it can only hurt.
//! The [`Sequencer`] releases work in `(release, arrival_sequence)` order
//! and never before its release time. All times are microseconds on the
//! host's monotonic clock.

use bunting_market_types::{ParticipantId, VenueId};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};
use std::fmt;

/// Upper bound for any configured delay, so all arithmetic stays far from
/// overflow and a misconfiguration cannot park commands for hours.
pub const MAX_CONFIGURED_DELAY_US: u64 = 10_000_000;
/// Upper bound for distinct `(direction, participant, venue)` jitter streams.
pub const MAX_JITTER_STREAMS: usize = 65_536;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdmissionError {
    /// The bounded release queue is full; carries the configured limit.
    QueueFull {
        limit: usize,
    },
    /// Too many distinct jitter streams; carries the configured limit.
    TooManyStreams {
        limit: usize,
    },
    InvalidPolicy(&'static str),
}

impl fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::QueueFull { limit } => write!(formatter, "max_admission_queue limit {limit}"),
            Self::TooManyStreams { limit } => {
                write!(formatter, "jitter stream limit {limit}")
            }
            Self::InvalidPolicy(reason) => write!(formatter, "invalid latency policy: {reason}"),
        }
    }
}

impl std::error::Error for AdmissionError {}

/// Which measurements a team's access-latency figure came from.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RttSource {
    /// No sample yet.
    None,
    /// Kernel TCP minimum RTT only (handshake and ACK timing).
    Kernel,
    /// Application probes only (FIX `TestRequest`, BNP `Ping`).
    Probe,
    /// Both; the figure is the smaller.
    Both,
}

/// One connection's measured real network delay (ADR 0035 §2): half the
/// minimum of the kernel TCP RTT and the probe RTT over the connection's
/// lifetime. Published as the team's access latency; it never changes
/// admission order.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DelayEstimator {
    kernel_min_us: Option<u64>,
    probe_min_us: Option<u64>,
    probe_samples: u64,
}

impl DelayEstimator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            kernel_min_us: None,
            probe_min_us: None,
            probe_samples: 0,
        }
    }

    /// Records one application probe round trip.
    pub fn record_probe(&mut self, rtt_us: u64) {
        self.probe_min_us = Some(self.probe_min_us.map_or(rtt_us, |min| min.min(rtt_us)));
        self.probe_samples = self.probe_samples.saturating_add(1);
    }

    /// Records the kernel's current minimum RTT for the socket. A zero
    /// reading means "no sample yet" on Linux and is ignored.
    pub fn record_kernel_min(&mut self, min_rtt_us: u64) {
        if min_rtt_us == 0 {
            return;
        }
        self.kernel_min_us = Some(
            self.kernel_min_us
                .map_or(min_rtt_us, |min| min.min(min_rtt_us)),
        );
    }

    #[must_use]
    pub const fn kernel_min_rtt_us(&self) -> Option<u64> {
        self.kernel_min_us
    }

    #[must_use]
    pub const fn probe_min_rtt_us(&self) -> Option<u64> {
        self.probe_min_us
    }

    #[must_use]
    pub const fn probe_samples(&self) -> u64 {
        self.probe_samples
    }

    #[must_use]
    pub const fn source(&self) -> RttSource {
        match (self.kernel_min_us, self.probe_min_us) {
            (None, None) => RttSource::None,
            (Some(_), None) => RttSource::Kernel,
            (None, Some(_)) => RttSource::Probe,
            (Some(_), Some(_)) => RttSource::Both,
        }
    }

    #[must_use]
    pub fn min_rtt_us(&self) -> Option<u64> {
        match (self.kernel_min_us, self.probe_min_us) {
            (Some(kernel), Some(probe)) => Some(kernel.min(probe)),
            (kernel, probe) => kernel.or(probe),
        }
    }

    /// Measured one-way access latency, if any sample exists.
    #[must_use]
    pub fn one_way_us(&self) -> Option<u64> {
        self.min_rtt_us().map(|rtt| rtt / 2)
    }
}

/// Simulated latency of one path: a fixed part plus uniform jitter in
/// `[0, jitter_us]`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathLatency {
    pub latency_us: u64,
    #[serde(default)]
    pub jitter_us: u64,
}

/// One row of the scenario's latency table: the virtual distance between a
/// team's location and a venue, used in both directions. `venue_id: None`
/// is the team's default for every venue and for requests without a venue.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathEntry {
    pub participant_id: ParticipantId,
    #[serde(default)]
    pub venue_id: Option<VenueId>,
    #[serde(flatten)]
    pub path: PathLatency,
}

/// Published per run before it starts; must not change mid-round.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyPolicy {
    /// Seeds every jitter stream; recorded with the run.
    #[serde(default)]
    pub jitter_seed: u64,
    #[serde(default)]
    pub default_path: PathLatency,
    #[serde(default)]
    pub paths: Vec<PathEntry>,
}

impl LatencyPolicy {
    pub fn validate(&self) -> Result<(), AdmissionError> {
        let bounded = |path: &PathLatency| {
            path.latency_us <= MAX_CONFIGURED_DELAY_US && path.jitter_us <= MAX_CONFIGURED_DELAY_US
        };
        if !bounded(&self.default_path) || !self.paths.iter().all(|entry| bounded(&entry.path)) {
            return Err(AdmissionError::InvalidPolicy("path latency exceeds 10 s"));
        }
        if self.paths.len() > MAX_JITTER_STREAMS {
            return Err(AdmissionError::InvalidPolicy("too many latency paths"));
        }
        let mut keys = self
            .paths
            .iter()
            .map(|entry| (entry.participant_id, entry.venue_id))
            .collect::<Vec<_>>();
        keys.sort_unstable();
        if keys.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(AdmissionError::InvalidPolicy("duplicate latency path"));
        }
        Ok(())
    }

    /// The configured path for a team and venue.
    #[must_use]
    pub fn path(&self, participant: ParticipantId, venue: Option<VenueId>) -> PathLatency {
        let find = |venue: Option<VenueId>| {
            self.paths
                .iter()
                .find(|entry| entry.participant_id == participant && entry.venue_id == venue)
                .map(|entry| entry.path)
        };
        venue
            .and_then(|venue| find(Some(venue)))
            .or_else(|| find(None))
            .unwrap_or(self.default_path)
    }
}

/// Every input and output of one admission decision; journaled with the
/// command so replay never re-measures the network.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionRecord {
    /// Server monotonic receive time; includes the team's real delay.
    pub received_us: u64,
    /// The team's measured one-way access latency at the time (published,
    /// not used for ordering).
    pub measured_one_way_us: Option<u64>,
    /// Measurements behind `measured_one_way_us`.
    pub rtt_source: RttSource,
    /// `L(p, v)` applied, jitter included.
    pub path_latency_us: u64,
    /// Position in the `(participant, venue)` inbound jitter stream, when a
    /// jitter draw was made.
    pub jitter_position: Option<u64>,
    /// When the command reaches the venue's book.
    pub release_us: u64,
    /// Tie-breaker among equal releases: global admission order.
    pub arrival_sequence: u64,
}

/// Separates inbound and outbound jitter streams, so the number of outbound
/// messages never shifts the inbound draws recorded in the journal.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum Direction {
    Inbound = 0,
    Outbound = 1,
}

/// Computes virtual path delays from the policy and per-path jitter streams.
#[derive(Clone, Debug)]
pub struct LatencyModel {
    policy: LatencyPolicy,
    streams: BTreeMap<(Direction, ParticipantId, Option<VenueId>), u64>,
}

impl LatencyModel {
    pub fn new(policy: LatencyPolicy) -> Result<Self, AdmissionError> {
        policy.validate()?;
        Ok(Self {
            policy,
            streams: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn policy(&self) -> &LatencyPolicy {
        &self.policy
    }

    /// Admission decision for a command received at `received_us` from a
    /// team, addressed to `venue` (if any): it reaches the book after the
    /// team's virtual path to that venue. The arrival sequence and any clamp
    /// to the sequencer floor are filled in by [`Sequencer::admit`].
    pub fn decide(
        &mut self,
        received_us: u64,
        measured: &DelayEstimator,
        participant: ParticipantId,
        venue: Option<VenueId>,
    ) -> Result<AdmissionRecord, AdmissionError> {
        let (path, jitter_position) = self.path_delay(Direction::Inbound, participant, venue)?;
        Ok(AdmissionRecord {
            received_us,
            measured_one_way_us: measured.one_way_us(),
            rtt_source: measured.source(),
            path_latency_us: path,
            jitter_position,
            release_us: received_us.saturating_add(path),
            arrival_sequence: 0,
        })
    }

    /// How long a venue's message to `participant` travels the virtual path
    /// back: `L(v, p)`. The team's real delay then applies on the wire.
    /// Session-level messages (heartbeats, probes) are never delayed.
    pub fn outbound_delay_us(
        &mut self,
        participant: ParticipantId,
        venue: Option<VenueId>,
    ) -> Result<u64, AdmissionError> {
        self.path_delay(Direction::Outbound, participant, venue)
            .map(|(delay, _)| delay)
    }

    fn path_delay(
        &mut self,
        direction: Direction,
        participant: ParticipantId,
        venue: Option<VenueId>,
    ) -> Result<(u64, Option<u64>), AdmissionError> {
        let path = self.policy.path(participant, venue);
        if path.jitter_us == 0 {
            return Ok((path.latency_us, None));
        }
        let (jitter, position) = self.draw_jitter(direction, participant, venue, path.jitter_us)?;
        Ok((path.latency_us + jitter, Some(position)))
    }

    fn draw_jitter(
        &mut self,
        direction: Direction,
        participant: ParticipantId,
        venue: Option<VenueId>,
        jitter_us: u64,
    ) -> Result<(u64, u64), AdmissionError> {
        let key = (direction, participant, venue);
        if !self.streams.contains_key(&key) && self.streams.len() >= MAX_JITTER_STREAMS {
            return Err(AdmissionError::TooManyStreams {
                limit: MAX_JITTER_STREAMS,
            });
        }
        let position = self.streams.entry(key).or_insert(0);
        let current = *position;
        *position = position.wrapping_add(1);
        let stream =
            stream_seed(self.policy.jitter_seed, participant, venue) ^ splitmix64(direction as u64);
        let draw = splitmix64(stream ^ splitmix64(current));
        Ok((draw % (jitter_us + 1), current))
    }
}

fn stream_seed(seed: u64, participant: ParticipantId, venue: Option<VenueId>) -> u64 {
    let fold = |value: u128| {
        let low = u64::try_from(value & u128::from(u64::MAX)).unwrap_or(0);
        let high = u64::try_from(value >> 64).unwrap_or(0);
        splitmix64(low ^ splitmix64(high))
    };
    let venue = venue.map_or(u64::MAX, |venue| fold(venue.get()));
    splitmix64(seed ^ splitmix64(fold(participant.get()) ^ splitmix64(venue)))
}

/// `SplitMix64` (Steele, Lea and Flood, 2014): a public-domain, fully
/// specified 64-bit mixer, so jitter streams are identical on every host.
const fn splitmix64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Bounded release queue ordered by `(release_us, arrival_sequence)`.
///
/// Invariant: once a command is released at host time `now`, every later
/// admission has `release ≥ now`, so nothing can be ordered before an
/// already released command. The host must pass a monotonic `now_us` to
/// both [`Self::admit`] and [`Self::pop_due`] under the same lock.
#[derive(Debug)]
pub struct Sequencer<T> {
    capacity: usize,
    next_arrival: u64,
    floor_us: u64,
    heap: BinaryHeap<Reverse<(u64, u64)>>,
    items: BTreeMap<u64, (AdmissionRecord, T)>,
}

impl<T> Sequencer<T> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_arrival: 0,
            floor_us: 0,
            heap: BinaryHeap::new(),
            items: BTreeMap::new(),
        }
    }

    /// Queues one decided command. `now_us` is the host clock at insertion;
    /// the release is raised to it if a received timestamp taken earlier
    /// would otherwise order the command behind an already released one.
    pub fn admit(
        &mut self,
        now_us: u64,
        mut record: AdmissionRecord,
        item: T,
    ) -> Result<AdmissionRecord, AdmissionError> {
        if self.items.len() >= self.capacity {
            return Err(AdmissionError::QueueFull {
                limit: self.capacity,
            });
        }
        self.floor_us = self.floor_us.max(now_us);
        record.release_us = record.release_us.max(self.floor_us);
        record.arrival_sequence = self.next_arrival;
        self.next_arrival = self.next_arrival.wrapping_add(1);
        self.heap
            .push(Reverse((record.release_us, record.arrival_sequence)));
        self.items.insert(record.arrival_sequence, (record, item));
        Ok(record)
    }

    /// Removes the head if its release time has passed.
    pub fn pop_due(&mut self, now_us: u64) -> Option<(AdmissionRecord, T)> {
        self.floor_us = self.floor_us.max(now_us);
        let Reverse((release, arrival)) = *self.heap.peek()?;
        if release > self.floor_us {
            return None;
        }
        self.heap.pop();
        self.items.remove(&arrival)
    }

    /// Release time of the head, for the host's wait deadline.
    #[must_use]
    pub fn next_release_us(&self) -> Option<u64> {
        self.heap.peek().map(|Reverse((release, _))| *release)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests;
