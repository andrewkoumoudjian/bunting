//! Per-session outbound hold: messages leave in send-time order, ties in
//! the order they were held.

use std::collections::BTreeMap;

/// Outbound messages waiting to leave until they have crossed their
/// virtual path (ADR 0035). Bounded; overflow disconnects the session.
pub(crate) struct OutboundHold<M> {
    capacity: usize,
    next: u64,
    queue: BTreeMap<(u64, u64), M>,
}

impl<M> OutboundHold<M> {
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next: 0,
            queue: BTreeMap::new(),
        }
    }

    pub(crate) fn hold(&mut self, message: M, send_at_us: u64) -> Result<(), String> {
        if self.queue.len() >= self.capacity {
            return Err(format!(
                "max_outbound_hold limit {}: reconnect to recover",
                self.capacity
            ));
        }
        self.queue.insert((send_at_us, self.next), message);
        self.next = self.next.wrapping_add(1);
        Ok(())
    }

    pub(crate) fn next_due_us(&self) -> Option<u64> {
        self.queue.keys().next().map(|(due, _)| *due)
    }

    pub(crate) fn take_due(&mut self, now_us: u64) -> Vec<M> {
        let mut due = Vec::new();
        while let Some(entry) = self.queue.first_entry() {
            if entry.key().0 > now_us {
                break;
            }
            due.push(entry.remove());
        }
        due
    }
}
