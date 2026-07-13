//! Bounded plain-Worker subscription planning over committed origin facts.

use bunting_market_events::{EventEnvelope, EventPayload};
use bunting_market_types::{EventSequence, InstrumentId, ParticipantId};
use serde::Serialize;
use serde_json::{Value, json};

pub const ORIGIN_READ_LIMIT: usize = 65;
const PUBLIC_EVENT_LIMIT: usize = 32;
const PRIVATE_EVENT_LIMIT: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamClass {
    Public { instrument_id: InstrumentId },
    Private { participant_id: ParticipantId },
}

#[derive(Debug, Eq, PartialEq)]
pub enum Plan {
    Tail(Vec<EventEnvelope>),
    Reset {
        cursor: EventSequence,
        reason: &'static str,
    },
}

fn is_public(event: &EventEnvelope, instrument_id: InstrumentId) -> bool {
    match &event.payload {
        EventPayload::OrderReceived { order } => order.instrument_id == instrument_id,
        EventPayload::OrderRested {
            instrument_id: id, ..
        }
        | EventPayload::OrderCanceled {
            instrument_id: id, ..
        }
        | EventPayload::TradeExecuted {
            instrument_id: id, ..
        }
        | EventPayload::PositionChanged {
            instrument_id: id, ..
        } => *id == instrument_id,
        _ => false,
    }
}

fn is_private(event: &EventEnvelope, participant_id: ParticipantId) -> bool {
    if event.actor == participant_id {
        return true;
    }
    match &event.payload {
        EventPayload::OrderReceived { order } => order.participant_id == participant_id,
        EventPayload::OrderRested {
            participant_id: id, ..
        }
        | EventPayload::OrderCanceled {
            participant_id: id, ..
        }
        | EventPayload::PositionChanged {
            participant_id: id, ..
        } => *id == participant_id,
        EventPayload::TradeExecuted {
            buyer_id,
            seller_id,
            ..
        } => *buyer_id == participant_id || *seller_id == participant_id,
        _ => false,
    }
}

pub fn plan(events: Vec<EventEnvelope>, current: EventSequence, class: StreamClass) -> Plan {
    if events.len() >= ORIGIN_READ_LIMIT {
        return Plan::Reset {
            cursor: current,
            reason: "origin_tail_exceeds_bound",
        };
    }
    let limit = match class {
        StreamClass::Public { .. } => PUBLIC_EVENT_LIMIT,
        StreamClass::Private { .. } => PRIVATE_EVENT_LIMIT,
    };
    let filtered: Vec<_> = events
        .into_iter()
        .filter(|event| match class {
            StreamClass::Public { instrument_id } => is_public(event, instrument_id),
            StreamClass::Private { participant_id } => is_private(event, participant_id),
        })
        .collect();
    if filtered.len() > limit {
        return Plan::Reset {
            cursor: current,
            reason: if matches!(class, StreamClass::Public { .. }) {
                "public_tail_coalesced_to_snapshot"
            } else {
                "private_slow_consumer_disconnected"
            },
        };
    }
    Plan::Tail(filtered)
}

fn frame(event: Option<&str>, id: Option<EventSequence>, data: &Value) -> Vec<u8> {
    let mut output = String::new();
    if let Some(event) = event {
        output.push_str("event: ");
        output.push_str(event);
        output.push('\n');
    }
    if let Some(id) = id {
        output.push_str("id: ");
        output.push_str(&id.to_string());
        output.push('\n');
    }
    output.push_str("data: ");
    output.push_str(&serde_json::to_string(data).unwrap_or_else(|_| "null".to_owned()));
    output.push_str("\n\n");
    output.into_bytes()
}

pub fn encode<T: Serialize>(
    plan: Plan,
    snapshot: Option<&T>,
    cursor: EventSequence,
) -> Vec<Vec<u8>> {
    let mut frames = vec![frame(Some("connected"), None, &json!({}))];
    match plan {
        Plan::Tail(events) => frames.extend(events.into_iter().map(|event| {
            frame(
                Some("committed.event"),
                Some(event.sequence),
                &json!({"kind":"committed.event","event":event}),
            )
        })),
        Plan::Reset { cursor, reason } => {
            frames.push(frame(
                Some("stream.reset"),
                Some(cursor),
                &json!({"kind":"stream.reset","afterSequence":cursor.to_string(),"reason":reason}),
            ));
            if let Some(snapshot) = snapshot {
                frames.push(frame(
                    Some("market.snapshot"),
                    Some(cursor),
                    &json!({"kind":"market.snapshot","snapshot":snapshot}),
                ));
            }
        }
    }
    frames.push(frame(Some("close"), Some(cursor), &json!({})));
    frames
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_market_events::{EVENT_SCHEMA_VERSION, EventPayload};
    use bunting_market_types::{CommandId, CorrelationId, EventId, LogicalTimeNs, RunId};

    fn event(sequence: u64, actor: u128) -> EventEnvelope {
        EventEnvelope {
            schema_version: EVENT_SCHEMA_VERSION,
            run_id: RunId::new(1),
            event_id: EventId::new(u128::from(sequence)),
            sequence: EventSequence::new(sequence),
            logical_time: LogicalTimeNs::new(sequence),
            actor: ParticipantId::new(actor),
            command_id: CommandId::new(u128::from(sequence)),
            correlation_id: CorrelationId::new(u128::from(sequence)),
            causation_sequence: None,
            payload: EventPayload::KillSwitchActivated,
        }
    }

    #[test]
    fn private_backlog_never_silently_drops() {
        let events = (1..=65).map(|sequence| event(sequence, 7)).collect();
        assert!(matches!(
            plan(
                events,
                EventSequence::new(65),
                StreamClass::Private {
                    participant_id: ParticipantId::new(7)
                }
            ),
            Plan::Reset {
                reason: "origin_tail_exceeds_bound",
                ..
            }
        ));
    }

    #[test]
    fn response_is_bounded_and_closes_with_recovery_cursor() {
        let frames = encode::<Value>(Plan::Tail(Vec::new()), None, EventSequence::new(9));
        assert_eq!(frames.len(), 2);
        assert!(String::from_utf8_lossy(&frames[1]).contains("event: close\nid: 9"));
    }
}
