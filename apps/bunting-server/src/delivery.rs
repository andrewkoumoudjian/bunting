//! Which committed batches a FIX session has written to its participant, so
//! the next session of that participant sends exactly the private reports
//! the last one never wrote: those still held on their virtual path when it
//! closed, and those committed while no session was connected.
//!
//! Reports do not leave in commit order (a fill from a near venue overtakes
//! an earlier one from a far venue, ADR 0035), so one "highest sequence
//! sent" cursor would skip the overtaken report. The tracker keeps a
//! low-water mark below which everything was written, plus the batches
//! above it still unwritten.

use crate::distributor::Committed;
use std::collections::{BTreeMap, BTreeSet};

/// What a closed session left for the next one.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct DeliveryCursor {
    /// Every batch whose events all lie at or below this sequence was
    /// written (or had nothing for the participant).
    pub(crate) through: u64,
    /// The last event sequence the session saw.
    pub(crate) seen_through: u64,
    /// Last event sequence of each batch above `through` that the session
    /// saw but had not written.
    pub(crate) unwritten: BTreeSet<u64>,
}

impl DeliveryCursor {
    /// A cursor for a stream that starts at the run's current sequence.
    pub(crate) fn at(sequence: u64) -> Self {
        Self {
            through: sequence,
            seen_through: sequence,
            unwritten: BTreeSet::new(),
        }
    }

    /// Whether a replayed batch ending at `last` still has to be sent: it
    /// was unwritten, or the last session never saw it.
    pub(crate) fn owes(&self, last: u64) -> bool {
        last > self.through && (last > self.seen_through || self.unwritten.contains(&last))
    }
}

/// The live tracker of one session.
#[derive(Debug)]
pub(crate) struct Delivery {
    cursor: DeliveryCursor,
    /// Unwritten batches by last sequence: first sequence and send time.
    held: BTreeMap<u64, (u64, u64)>,
}

impl Delivery {
    /// Starts from `cursor`. Batches it still owes are passed to
    /// [`Self::on_batch`] again by the replay; the rest stay written.
    pub(crate) fn new(cursor: &DeliveryCursor) -> Self {
        Self {
            cursor: DeliveryCursor {
                through: cursor.through,
                seen_through: cursor.seen_through,
                unwritten: BTreeSet::new(),
            },
            held: BTreeMap::new(),
        }
    }

    /// Records a batch spanning event sequences `first..=last` that the
    /// session processed: `send_at_us` is when its reports leave, or `None`
    /// when it has none for this participant.
    pub(crate) fn on_batch(&mut self, (first, last): (u64, u64), send_at_us: Option<u64>) {
        self.cursor.seen_through = self.cursor.seen_through.max(last);
        if let Some(send_at) = send_at_us {
            self.held.insert(last, (first, send_at));
        }
        self.advance();
    }

    /// Every report due at `now_us` was written.
    pub(crate) fn on_written(&mut self, now_us: u64) {
        self.held.retain(|_, (_, send_at)| *send_at > now_us);
        self.advance();
    }

    /// The cursor the next session resumes from.
    pub(crate) fn cursor(&self) -> DeliveryCursor {
        DeliveryCursor {
            through: self.cursor.through,
            seen_through: self.cursor.seen_through,
            unwritten: self.held.keys().copied().collect(),
        }
    }

    fn advance(&mut self) {
        let through = self
            .held
            .values()
            .map(|(first, _)| first.saturating_sub(1))
            .min()
            .unwrap_or(self.cursor.seen_through);
        self.cursor.through = self.cursor.through.max(through);
    }
}

/// The first and last event sequence of a batch.
pub(crate) fn bounds(batch: &Committed) -> Option<(u64, u64)> {
    Some((
        batch.events.first()?.sequence.get(),
        batch.events.last()?.sequence.get(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overtaken_report_is_still_owed_after_a_later_one_was_written() {
        let mut delivery = Delivery::new(&DeliveryCursor::at(10));
        // A far venue's fill (11..=12) leaves at 900; a near venue's later
        // fill (13..=14) at 200; 15 has nothing for this participant.
        delivery.on_batch((11, 12), Some(900));
        delivery.on_batch((13, 14), Some(200));
        delivery.on_batch((15, 15), None);
        delivery.on_written(500);
        let cursor = delivery.cursor();
        assert_eq!(cursor.through, 10);
        assert_eq!(cursor.seen_through, 15);
        assert!(cursor.owes(12), "the overtaken far fill was never written");
        assert!(!cursor.owes(14), "the near fill was written");
        assert!(!cursor.owes(15), "nothing to send");
        assert!(cursor.owes(16), "committed after the session closed");
        // The next session replays 12 and 16 and writes both.
        let mut next = Delivery::new(&cursor);
        next.on_batch((11, 12), Some(1_000));
        next.on_batch((16, 16), Some(1_000));
        assert_eq!(next.cursor().through, 10);
        next.on_written(1_000);
        assert_eq!(next.cursor(), DeliveryCursor::at(16));
    }

    #[test]
    fn the_low_water_mark_follows_the_oldest_unwritten_batch() {
        let mut delivery = Delivery::new(&DeliveryCursor::at(0));
        delivery.on_batch((1, 1), Some(10));
        delivery.on_batch((2, 3), Some(20));
        delivery.on_written(10);
        assert_eq!(delivery.cursor().through, 1);
        delivery.on_written(20);
        assert_eq!(delivery.cursor(), DeliveryCursor::at(3));
    }
}
