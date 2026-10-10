#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Latency-modeled continuous admission (ADR 0030), sans-I/O and clock-free.
//!
//! The host measures; this crate decides. For a command received at server
//! monotonic time `t_rx` on a connection whose one-way delay estimate is `d̂`:
//!
//! ```text
//! physical:    release = t_rx
//! equalized:   release = t_rx − d̂ + D_max
//! geographic:  release = t_rx − d̂ + D_max + L(p, v)
//! ```
//!
//! `d̂` is half the windowed-minimum RTT, clamped to `[0, D_max]`, so
//! `release ≥ t_rx`. `L(p, v)` is a configured path latency plus a seeded
//! jitter draw. The [`Sequencer`] releases commands in
//! `(release, arrival_sequence)` order and never before their release time.
//! All times are microseconds on the host's monotonic clock.

use bunting_market_types::{ParticipantId, VenueId};
use serde::{Deserialize, Serialize};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};
use std::fmt;

/// Upper bound for any configured delay, so all arithmetic stays far from
/// overflow and a misconfiguration cannot park commands for hours.
pub const MAX_CONFIGURED_DELAY_US: u64 = 10_000_000;
/// Upper bound for the RTT sample window.
pub const MAX_RTT_WINDOW: usize = 1_024;
/// Upper bound for distinct `(participant, venue)` jitter streams.
pub const MAX_JITTER_STREAMS: usize = 65_536;

/// How admission compensates for physical distance (ADR 0030 table).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionMode {
    /// Server receive order; for LAN events where real latency counts.
    Physical,
    /// Estimated send time plus a constant; nobody gains from their ISP.
    Equalized,
    /// Estimated send time plus a constant plus simulated path latency.
    Geographic,
}

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
            Self::InvalidPolicy(reason) => write!(formatter, "invalid admission policy: {reason}"),
        }
    }
}

impl std::error::Error for AdmissionError {}

/// Windowed-minimum RTT estimator (the NTP/BBR propagation-delay filter):
/// queuing spikes raise single samples but not the window minimum.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RttEstimator {
    window: usize,
    samples: VecDeque<u64>,
}

impl RttEstimator {
    /// `window` is clamped to `1..=MAX_RTT_WINDOW`.
    #[must_use]
    pub fn new(window: usize) -> Self {
        let window = window.clamp(1, MAX_RTT_WINDOW);
        Self {
            window,
            samples: VecDeque::with_capacity(window),
        }
    }

    pub fn record(&mut self, rtt_us: u64) {
        if self.samples.len() == self.window {
            self.samples.pop_front();
        }
        self.samples.push_back(rtt_us);
    }

    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    #[must_use]
    pub fn min_rtt_us(&self) -> Option<u64> {
        self.samples.iter().copied().min()
    }

    /// `d̂ = clamp(min_rtt / 2, 0, D_max)`. A connection that has never
    /// answered a probe gets no compensation (`0`), never `D_max`.
    #[must_use]
    pub fn one_way_delay_us(&self, max_one_way_delay_us: u64) -> u64 {
        self.min_rtt_us()
            .map_or(0, |rtt| (rtt / 2).min(max_one_way_delay_us))
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

/// One row of the scenario latency table. `venue_id: None` is the
/// participant's default for every venue and for requests without a venue.
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyPolicy {
    pub mode: AdmissionMode,
    /// `D_max`: the largest one-way delay the venue compensates.
    pub max_one_way_delay_us: u64,
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
        if self.max_one_way_delay_us > MAX_CONFIGURED_DELAY_US {
            return Err(AdmissionError::InvalidPolicy(
                "max_one_way_delay_us exceeds 10 s",
            ));
        }
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

    fn path(&self, participant: ParticipantId, venue: Option<VenueId>) -> PathLatency {
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
    pub mode: AdmissionMode,
    /// Server monotonic receive time.
    pub received_us: u64,
    /// `d̂(c)` applied (0 in `physical`).
    pub one_way_delay_us: u64,
    /// `D_max` applied (0 in `physical`).
    pub max_one_way_delay_us: u64,
    /// `L(p, v)` applied, jitter included (0 unless `geographic`).
    pub path_latency_us: u64,
    /// Position in the `(participant, venue)` jitter stream that produced
    /// the jitter, when one was drawn.
    pub jitter_position: Option<u64>,
    /// When the command reaches the book.
    pub release_us: u64,
    /// Tie-breaker among equal releases: global admission order.
    pub arrival_sequence: u64,
}

/// Computes release times from the policy and per-path jitter streams.
#[derive(Clone, Debug)]
pub struct LatencyModel {
    policy: LatencyPolicy,
    streams: BTreeMap<(ParticipantId, Option<VenueId>), u64>,
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
    /// connection with estimator `rtt`, addressed to `venue` (if any). The
    /// arrival sequence and any clamp to the sequencer floor are filled in
    /// by [`Sequencer::admit`].
    pub fn decide(
        &mut self,
        received_us: u64,
        rtt: &RttEstimator,
        participant: ParticipantId,
        venue: Option<VenueId>,
    ) -> Result<AdmissionRecord, AdmissionError> {
        let d_max = self.policy.max_one_way_delay_us;
        let (one_way, max_one_way, path, jitter_position) = match self.policy.mode {
            AdmissionMode::Physical => (0, 0, 0, None),
            AdmissionMode::Equalized => (rtt.one_way_delay_us(d_max), d_max, 0, None),
            AdmissionMode::Geographic => {
                let path = self.policy.path(participant, venue);
                let (jitter, position) = if path.jitter_us == 0 {
                    (0, None)
                } else {
                    let (jitter, position) =
                        self.draw_jitter(participant, venue, path.jitter_us)?;
                    (jitter, Some(position))
                };
                (
                    rtt.one_way_delay_us(d_max),
                    d_max,
                    path.latency_us + jitter,
                    position,
                )
            }
        };
        // Written as `t_rx + (D_max − d̂) + L`: `one_way ≤ max_one_way`, so
        // nothing underflows even when the monotonic `t_rx` is below `d̂`
        // (early after host start), and `release ≥ received_us`.
        let release_us = received_us
            .saturating_add(max_one_way - one_way)
            .saturating_add(path);
        Ok(AdmissionRecord {
            mode: self.policy.mode,
            received_us,
            one_way_delay_us: one_way,
            max_one_way_delay_us: max_one_way,
            path_latency_us: path,
            jitter_position,
            release_us,
            arrival_sequence: 0,
        })
    }

    fn draw_jitter(
        &mut self,
        participant: ParticipantId,
        venue: Option<VenueId>,
        jitter_us: u64,
    ) -> Result<(u64, u64), AdmissionError> {
        let key = (participant, venue);
        if !self.streams.contains_key(&key) && self.streams.len() >= MAX_JITTER_STREAMS {
            return Err(AdmissionError::TooManyStreams {
                limit: MAX_JITTER_STREAMS,
            });
        }
        let position = self.streams.entry(key).or_insert(0);
        let current = *position;
        *position = position.wrapping_add(1);
        let stream = stream_seed(self.policy.jitter_seed, participant, venue);
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
