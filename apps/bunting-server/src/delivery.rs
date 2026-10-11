//! Which committed batches' reports a FIX session has applied but not yet
//! written to its participant, so they reach the participant even when the
//! session closes, or the venue stops, before they leave: still held on
//! their virtual path, a near venue's later fill overtaking a far venue's
//! earlier one (ADR 0035). The session log keeps this with the application
//! (ADR 0045), so the next session sends exactly those reports, without
//! applying their batches a second time, and applies every batch committed
//! after [`Delivery::seen_through`] from the distributor or the journal.

use crate::distributor::Committed;
use serde::{Deserialize, Serialize};
use simfix_wire::FixMessage;
use std::collections::BTreeMap;

/// One applied batch's reports that have not been written yet.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Unwritten {
    /// The batch's last event sequence.
    pub(crate) last: u64,
    pub(crate) messages: Vec<FixMessage>,
    /// Venue time the reports leave; a restored session sends them at once.
    #[serde(skip)]
    pub(crate) send_at_us: u64,
}

/// The delivery state of one participant's FIX application.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Delivery {
    /// The last event sequence of the batches applied so far, in commit
    /// order; every later batch is still to be applied.
    seen_through: u64,
    /// Applied batches whose reports are not written, by last sequence.
    unwritten: BTreeMap<u64, Unwritten>,
}

impl Delivery {
    /// A stream that starts after event sequence `seen_through`.
    pub(crate) const fn at(seen_through: u64) -> Self {
        Self {
            seen_through,
            unwritten: BTreeMap::new(),
        }
    }

    /// Restores `unwritten` reports, due at once.
    pub(crate) fn restore(seen_through: u64, unwritten: Vec<Unwritten>) -> Self {
        Self {
            seen_through,
            unwritten: unwritten
                .into_iter()
                .map(|batch| (batch.last, batch))
                .collect(),
        }
    }

    pub(crate) const fn seen_through(&self) -> u64 {
        self.seen_through
    }

    /// Batches up to `seen_through` were applied, including those that
    /// left nothing to record.
    pub(crate) fn advance_to(&mut self, seen_through: u64) {
        self.seen_through = self.seen_through.max(seen_through);
    }

    /// Records an applied batch ending at `last` whose `messages` leave at
    /// `send_at_us`.
    pub(crate) fn on_batch(&mut self, last: u64, messages: &[FixMessage], send_at_us: u64) {
        self.seen_through = self.seen_through.max(last);
        if !messages.is_empty() {
            self.unwritten.insert(
                last,
                Unwritten {
                    last,
                    messages: messages.to_vec(),
                    send_at_us,
                },
            );
        }
    }

    /// Every report due at `now_us` is being written.
    pub(crate) fn on_written(&mut self, now_us: u64) {
        self.unwritten.retain(|_, batch| batch.send_at_us > now_us);
    }

    /// The last sequences of the batches still unwritten.
    pub(crate) fn unwritten_keys(&self) -> Vec<u64> {
        self.unwritten.keys().copied().collect()
    }

    /// The unwritten reports, oldest batch first.
    pub(crate) fn unwritten(&self) -> impl Iterator<Item = &Unwritten> {
        self.unwritten.values()
    }

    /// Keeps only the unwritten batches in `keys` (a delta's record of what
    /// was still unwritten when it was made durable).
    pub(crate) fn retain(&mut self, keys: &[u64]) {
        self.unwritten
            .retain(|last, _| keys.binary_search(last).is_ok());
    }
}

/// The last event sequence of a batch.
pub(crate) fn last_sequence(batch: &Committed) -> Option<u64> {
    Some(batch.events.last()?.sequence.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: &str) -> Vec<FixMessage> {
        let mut message = FixMessage::new("8");
        message.push(11, id);
        vec![message]
    }

    #[test]
    fn an_overtaken_report_stays_unwritten_after_a_later_one_was_written() {
        let mut delivery = Delivery::at(10);
        // A far venue's fill (..=12) leaves at 900; a near venue's later
        // fill (..=14) at 200; 15 has nothing for this participant.
        delivery.on_batch(12, &report("far"), 900);
        delivery.on_batch(14, &report("near"), 200);
        delivery.on_batch(15, &[], 300);
        delivery.on_written(500);
        assert_eq!(delivery.seen_through(), 15);
        assert_eq!(delivery.unwritten_keys(), vec![12]);
        delivery.on_written(900);
        assert!(delivery.unwritten_keys().is_empty());
    }

    #[test]
    fn restored_reports_are_due_at_once_and_a_delta_trims_them() {
        let mut delivery = Delivery::restore(
            20,
            vec![
                Unwritten {
                    last: 18,
                    messages: report("a"),
                    send_at_us: 0,
                },
                Unwritten {
                    last: 20,
                    messages: report("b"),
                    send_at_us: 0,
                },
            ],
        );
        let mut trimmed = delivery.clone();
        trimmed.retain(&[20]);
        assert_eq!(trimmed.unwritten_keys(), vec![20]);
        delivery.on_written(1);
        assert!(delivery.unwritten_keys().is_empty());
    }
}
