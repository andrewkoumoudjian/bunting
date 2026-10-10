#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Latency for venue admission (ADR 0035), sans-I/O and clock-free.
//!
//! Latency works as on a real network; there is nothing to switch on or
//! off. A team's real delay to the server (connection method, TCP stack,
//! uplink, distance) is inside every receive time and on the wire, and the
//! run's virtual geography ([`LatencyMap`]) adds the distance between
//! wherever the sender and the receiver sit:
//!
//! ```text
//! team -> venue:   release = t_rx + L(team, venue)
//! venue -> team:   send_at = t_venue + L(venue, team)
//! team -> team:    send_at = t_commit + L(sender, receiver)   # OTC, sharing
//! ```
//!
//! Real delay is still measured ([`DelayEstimator`]) and published as each
//! team's access latency; it never changes ordering, so inflating it can
//! only hurt. The [`Sequencer`] releases work in `(release,
//! arrival_sequence)` order and never before its release time. All times
//! are microseconds on the host's monotonic clock.

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

/// Simulated latency of one link: a fixed part plus uniform jitter in
/// `[0, jitter_us]`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PathLatency {
    pub latency_us: u64,
    #[serde(default)]
    pub jitter_us: u64,
}

/// Anything that sends or receives market messages.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Endpoint {
    /// A team.
    Participant(ParticipantId),
    /// A venue's matching engine.
    Venue(VenueId),
    /// The organizer's back office: tenders, news, account and score reports.
    Hub,
}

/// Places one team at a location.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantPlacement {
    pub participant_id: ParticipantId,
    pub location: String,
}

/// Places one venue at a location.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VenuePlacement {
    pub venue_id: VenueId,
    pub location: String,
}

/// A two-way link between two distinct locations.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub between: [String; 2],
    #[serde(flatten)]
    pub latency: PathLatency,
}

/// The run's virtual geography, published before it starts and never
/// changed during it: where every team, every venue and the organizer's hub
/// sit, and the latency between locations. Team-to-venue, venue-to-team and
/// team-to-team delays all come from it.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LatencyMap {
    /// Seeds every jitter stream; recorded with the run.
    #[serde(default)]
    pub jitter_seed: u64,
    /// Where the hub, and every team or venue not placed below, sit.
    #[serde(default = "default_location")]
    pub default_location: String,
    #[serde(default)]
    pub participants: Vec<ParticipantPlacement>,
    #[serde(default)]
    pub venues: Vec<VenuePlacement>,
    /// Latency between two endpoints at the same location (a cross-connect).
    #[serde(default)]
    pub local: PathLatency,
    /// Links between distinct locations; every pair of locations in use
    /// needs one.
    #[serde(default)]
    pub links: Vec<Link>,
}

fn default_location() -> String {
    "hub".to_owned()
}

impl Default for LatencyMap {
    fn default() -> Self {
        Self {
            jitter_seed: 0,
            default_location: default_location(),
            participants: Vec::new(),
            venues: Vec::new(),
            local: PathLatency::default(),
            links: Vec::new(),
        }
    }
}

impl LatencyMap {
    /// Checks bounds, duplicates and that every pair of locations in use is
    /// linked.
    pub fn validate(&self) -> Result<(), AdmissionError> {
        let bounded = |path: &PathLatency| {
            path.latency_us <= MAX_CONFIGURED_DELAY_US && path.jitter_us <= MAX_CONFIGURED_DELAY_US
        };
        if !bounded(&self.local) || !self.links.iter().all(|link| bounded(&link.latency)) {
            return Err(AdmissionError::InvalidPolicy("link latency exceeds 10 s"));
        }
        if self.participants.len() + self.venues.len() + self.links.len() > MAX_JITTER_STREAMS {
            return Err(AdmissionError::InvalidPolicy("latency map too large"));
        }
        if self.default_location.is_empty()
            || self
                .participants
                .iter()
                .map(|placement| &placement.location)
                .chain(self.venues.iter().map(|placement| &placement.location))
                .chain(self.links.iter().flat_map(|link| link.between.iter()))
                .any(String::is_empty)
        {
            return Err(AdmissionError::InvalidPolicy("empty location name"));
        }
        let mut teams = self
            .participants
            .iter()
            .map(|placement| placement.participant_id)
            .collect::<Vec<_>>();
        let mut venues = self
            .venues
            .iter()
            .map(|placement| placement.venue_id)
            .collect::<Vec<_>>();
        let mut links = self.links.iter().map(link_key).collect::<Vec<_>>();
        teams.sort_unstable();
        venues.sort_unstable();
        links.sort_unstable();
        if teams.windows(2).any(|pair| pair[0] == pair[1])
            || venues.windows(2).any(|pair| pair[0] == pair[1])
            || links.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(AdmissionError::InvalidPolicy("duplicate placement or link"));
        }
        if self
            .links
            .iter()
            .any(|link| link.between[0] == link.between[1])
        {
            return Err(AdmissionError::InvalidPolicy(
                "a link joins a location to itself; use `local`",
            ));
        }
        let mut used = std::collections::BTreeSet::new();
        used.insert(self.default_location.as_str());
        for placement in &self.participants {
            used.insert(placement.location.as_str());
        }
        for placement in &self.venues {
            used.insert(placement.location.as_str());
        }
        for (index, first) in used.iter().enumerate() {
            for second in used.iter().skip(index + 1) {
                if self.link(first, second).is_none() {
                    return Err(AdmissionError::InvalidPolicy(
                        "two locations in use have no link between them",
                    ));
                }
            }
        }
        Ok(())
    }

    /// The location of an endpoint.
    #[must_use]
    pub fn location(&self, endpoint: Endpoint) -> &str {
        let placed = match endpoint {
            Endpoint::Participant(participant) => self
                .participants
                .iter()
                .find(|placement| placement.participant_id == participant)
                .map(|placement| placement.location.as_str()),
            Endpoint::Venue(venue) => self
                .venues
                .iter()
                .find(|placement| placement.venue_id == venue)
                .map(|placement| placement.location.as_str()),
            Endpoint::Hub => None,
        };
        placed.unwrap_or(&self.default_location)
    }

    /// The configured latency between two endpoints (before jitter).
    #[must_use]
    pub fn path(&self, from: Endpoint, to: Endpoint) -> PathLatency {
        let (from, to) = (self.location(from), self.location(to));
        if from == to {
            return self.local;
        }
        self.link(from, to).unwrap_or_default()
    }

    fn link(&self, first: &str, second: &str) -> Option<PathLatency> {
        let key = ordered(first, second);
        self.links
            .iter()
            .find(|link| link_key(link) == key)
            .map(|link| link.latency)
    }
}

fn ordered<'a>(first: &'a str, second: &'a str) -> (&'a str, &'a str) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

fn link_key(link: &Link) -> (&str, &str) {
    ordered(&link.between[0], &link.between[1])
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
    /// Where the command was going: a venue, or the hub.
    pub destination: Endpoint,
    /// `L(team, destination)` applied, jitter included.
    pub path_latency_us: u64,
    /// Position in the `(team, destination)` jitter stream, when a jitter
    /// draw was made.
    pub jitter_position: Option<u64>,
    /// When the command reaches its destination.
    pub release_us: u64,
    /// Tie-breaker among equal releases: global admission order.
    pub arrival_sequence: u64,
}

/// Computes virtual delays from the map and per-path jitter streams.
#[derive(Clone, Debug)]
pub struct LatencyModel {
    map: LatencyMap,
    /// One jitter stream per directed endpoint pair, so traffic one way
    /// never shifts the draws recorded for the other.
    streams: BTreeMap<(Endpoint, Endpoint), u64>,
}

impl LatencyModel {
    pub fn new(map: LatencyMap) -> Result<Self, AdmissionError> {
        map.validate()?;
        Ok(Self {
            map,
            streams: BTreeMap::new(),
        })
    }

    #[must_use]
    pub const fn map(&self) -> &LatencyMap {
        &self.map
    }

    /// Admission decision for a command received at `received_us` from a
    /// team and going to `destination` (a venue, or the hub): it arrives
    /// after the team's virtual path there. The arrival sequence and any
    /// clamp to the sequencer floor are filled in by [`Sequencer::admit`].
    pub fn decide(
        &mut self,
        received_us: u64,
        measured: &DelayEstimator,
        participant: ParticipantId,
        destination: Endpoint,
    ) -> Result<AdmissionRecord, AdmissionError> {
        let (path, jitter_position) =
            self.delay_with_position(Endpoint::Participant(participant), destination)?;
        Ok(AdmissionRecord {
            received_us,
            measured_one_way_us: measured.one_way_us(),
            rtt_source: measured.source(),
            destination,
            path_latency_us: path,
            jitter_position,
            release_us: received_us.saturating_add(path),
            arrival_sequence: 0,
        })
    }

    /// Virtual delay of one message from `from` to `to`: venue to team,
    /// team to team (OTC and any other team-to-team channel), hub to team.
    /// The recipient's real delay then applies on the wire. Session-level
    /// messages (heartbeats, probes) are never delayed.
    pub fn delay_us(&mut self, from: Endpoint, to: Endpoint) -> Result<u64, AdmissionError> {
        self.delay_with_position(from, to).map(|(delay, _)| delay)
    }

    fn delay_with_position(
        &mut self,
        from: Endpoint,
        to: Endpoint,
    ) -> Result<(u64, Option<u64>), AdmissionError> {
        let path = self.map.path(from, to);
        if path.jitter_us == 0 {
            return Ok((path.latency_us, None));
        }
        let key = (from, to);
        if !self.streams.contains_key(&key) && self.streams.len() >= MAX_JITTER_STREAMS {
            return Err(AdmissionError::TooManyStreams {
                limit: MAX_JITTER_STREAMS,
            });
        }
        let position = self.streams.entry(key).or_insert(0);
        let current = *position;
        *position = position.wrapping_add(1);
        let stream = splitmix64(
            self.map.jitter_seed ^ splitmix64(endpoint_seed(from) ^ splitmix64(endpoint_seed(to))),
        );
        let draw = splitmix64(stream ^ splitmix64(current));
        Ok((path.latency_us + draw % (path.jitter_us + 1), Some(current)))
    }
}

fn endpoint_seed(endpoint: Endpoint) -> u64 {
    let fold = |tag: u64, value: u128| {
        let low = u64::try_from(value & u128::from(u64::MAX)).unwrap_or(0);
        let high = u64::try_from(value >> 64).unwrap_or(0);
        splitmix64(tag ^ splitmix64(low ^ splitmix64(high)))
    };
    match endpoint {
        Endpoint::Participant(participant) => fold(1, participant.get()),
        Endpoint::Venue(venue) => fold(2, venue.get()),
        Endpoint::Hub => fold(3, 0),
    }
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
