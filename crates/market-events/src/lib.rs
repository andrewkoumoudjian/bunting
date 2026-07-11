#![forbid(unsafe_code)]
//! Canonical command and event envelopes for the deterministic market kernel.

use bunting_market_types::{EventSequence, LogicalTimeNs};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope<E> {
    pub schema_version: u16,
    pub sequence: EventSequence,
    pub logical_time: LogicalTimeNs,
    pub correlation_id: String,
    pub causation_sequence: Option<EventSequence>,
    pub payload: E,
}

impl<E> EventEnvelope<E> {
    #[must_use]
    pub fn map<T>(self, map: impl FnOnce(E) -> T) -> EventEnvelope<T> {
        EventEnvelope {
            schema_version: self.schema_version,
            sequence: self.sequence,
            logical_time: self.logical_time,
            correlation_id: self.correlation_id,
            causation_sequence: self.causation_sequence,
            payload: map(self.payload),
        }
    }
}
