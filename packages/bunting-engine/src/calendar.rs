//! Trading calendar and session phases (ADR 0037 §2).
//!
//! A calendar gives each venue one session per trading day, as offsets
//! from the day's start in run time. Each session crosses four
//! boundaries: pre-open, open (the opening auction), closing call and close
//! (the closing auction, then DAY expiry). A venue's progress is the count
//! of boundaries it has passed, so a transition can never be skipped, even
//! when the run clock jumps over several at once.

use bunting_market_events::SessionPhase;
use bunting_market_types::{LogicalTimeNs, VenueId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Longest calendar a scenario may define.
pub const MAX_CALENDAR_DAYS: u32 = 3_660;

/// Boundaries per trading day: pre-open, open, closing call, close.
const BOUNDARIES_PER_DAY: u64 = 4;

/// One venue's daily session, as run-time offsets from each day's start:
/// `pre_open_ns <= open_ns <= closing_call_ns <= close_ns <= day_ns`.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct VenueSession {
    pub venue_id: VenueId,
    pub pre_open_ns: u64,
    pub open_ns: u64,
    pub closing_call_ns: u64,
    pub close_ns: u64,
}

/// A run's trading calendar. Day `d` starts at run time `d × day_ns`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Calendar {
    /// Run time of one calendar day.
    pub day_ns: u64,
    /// Calendar days in the run; venues stay closed after the last.
    pub days: u32,
    /// Day indices on which no venue opens.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub holidays: BTreeSet<u32>,
    /// Exactly one session per venue, ordered by venue.
    pub sessions: Vec<VenueSession>,
}

/// One session boundary: when it is, what it starts, and on which day.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Boundary {
    pub at: LogicalTimeNs,
    pub phase: SessionPhase,
    pub day: u32,
}

impl Calendar {
    /// Checks the calendar covers exactly `venues`, with ordered offsets
    /// inside one day and every boundary representable.
    #[must_use]
    pub fn is_valid(&self, venues: &BTreeSet<VenueId>) -> bool {
        let covered = self
            .sessions
            .iter()
            .map(|session| session.venue_id)
            .collect::<BTreeSet<_>>();
        self.day_ns > 0
            && (1..=MAX_CALENDAR_DAYS).contains(&self.days)
            && self.day_ns.checked_mul(u64::from(self.days)).is_some()
            && self.holidays.iter().all(|day| *day < self.days)
            && covered.len() == self.sessions.len()
            && self.sessions.is_sorted_by_key(|session| session.venue_id)
            && &covered == venues
            && self.sessions.iter().all(|session| {
                session.pre_open_ns <= session.open_ns
                    && session.open_ns <= session.closing_call_ns
                    && session.closing_call_ns <= session.close_ns
                    && session.close_ns <= self.day_ns
            })
    }

    fn session(&self, venue: VenueId) -> Option<&VenueSession> {
        self.sessions
            .binary_search_by_key(&venue, |session| session.venue_id)
            .ok()
            .map(|index| &self.sessions[index])
    }

    /// The `index`-th trading (non-holiday) day.
    fn trading_day(&self, index: u64) -> Option<u32> {
        (0..self.days)
            .filter(|day| !self.holidays.contains(day))
            .nth(usize::try_from(index).ok()?)
    }

    /// Boundary number `step` of `venue`'s sessions, if the calendar has
    /// that many.
    #[must_use]
    pub fn boundary(&self, venue: VenueId, step: u64) -> Option<Boundary> {
        let session = self.session(venue)?;
        let day = self.trading_day(step / BOUNDARIES_PER_DAY)?;
        let (offset, phase) = match step % BOUNDARIES_PER_DAY {
            0 => (session.pre_open_ns, SessionPhase::PreOpen),
            1 => (session.open_ns, SessionPhase::Continuous),
            2 => (session.closing_call_ns, SessionPhase::ClosingCall),
            _ => (session.close_ns, SessionPhase::Closed),
        };
        let at = self
            .day_ns
            .checked_mul(u64::from(day))?
            .checked_add(offset)?;
        Some(Boundary {
            at: LogicalTimeNs::new(at),
            phase,
            day,
        })
    }

    /// The phase a venue is in after passing `steps` boundaries.
    #[must_use]
    pub const fn phase_after(steps: u64) -> SessionPhase {
        if steps == 0 {
            return SessionPhase::Closed;
        }
        match (steps - 1) % BOUNDARIES_PER_DAY {
            0 => SessionPhase::PreOpen,
            1 => SessionPhase::Continuous,
            2 => SessionPhase::ClosingCall,
            _ => SessionPhase::Closed,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn calendar() -> Calendar {
        Calendar {
            day_ns: 100,
            days: 3,
            holidays: BTreeSet::from([1]),
            sessions: vec![VenueSession {
                venue_id: VenueId::new(1),
                pre_open_ns: 10,
                open_ns: 20,
                closing_call_ns: 80,
                close_ns: 90,
            }],
        }
    }

    #[test]
    fn boundaries_walk_each_trading_day_and_skip_holidays() {
        let calendar = calendar();
        let venue = VenueId::new(1);
        let at = |step| calendar.boundary(venue, step).map(|b| b.at.get());
        assert_eq!(
            (0..8).map(at).collect::<Vec<_>>(),
            [10, 20, 80, 90, 210, 220, 280, 290].map(Some).to_vec()
        );
        assert_eq!(at(8), None);
        assert_eq!(calendar.boundary(venue, 4).unwrap().day, 2);
        assert_eq!(Calendar::phase_after(0), SessionPhase::Closed);
        assert_eq!(Calendar::phase_after(2), SessionPhase::Continuous);
        assert_eq!(Calendar::phase_after(4), SessionPhase::Closed);
        assert_eq!(Calendar::phase_after(5), SessionPhase::PreOpen);
    }

    #[test]
    fn validation_requires_ordered_offsets_and_every_venue() {
        let venues = BTreeSet::from([VenueId::new(1)]);
        assert!(calendar().is_valid(&venues));
        assert!(!calendar().is_valid(&BTreeSet::from([VenueId::new(2)])));
        let mut late = calendar();
        late.sessions[0].close_ns = 101;
        assert!(!late.is_valid(&venues));
        let mut unordered = calendar();
        unordered.sessions[0].open_ns = 5;
        assert!(!unordered.is_valid(&venues));
        let mut holiday = calendar();
        holiday.holidays.insert(3);
        assert!(!holiday.is_valid(&venues));
    }
}
