#![forbid(unsafe_code)]
//! Mapping between QUARCC participant actions and committed Bunting facts.

use bunting_market_events::{
    CancelOrder, Command, CommandPayload, EventEnvelope, EventPayload, SubmitOrder,
};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, LogicalTimeNs, OrderId, ParticipantId, QuantityLots,
    RunId,
};
use quarcc_execution_engine::event::ExecutionAction;
use quarcc_execution_engine::ids::{LocalOrderId, ReportId, VenueOrderId};
use quarcc_execution_engine::normalized_report::{NormalizedVenueReport, VenueReportKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BuntingCommandContext {
    pub run_id: RunId,
    pub actor: ParticipantId,
    pub expected_sequence: EventSequence,
    pub logical_time: LogicalTimeNs,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AdapterError {
    UnsupportedAction,
    ActorMismatch,
    ArithmeticOverflow,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BuntingExecutionAdapter {
    cumulative_fills: BTreeMap<LocalOrderId, QuantityLots>,
    owned_orders: BTreeSet<LocalOrderId>,
}

impl BuntingExecutionAdapter {
    /// Converts one participant execution action into a canonical Bunting command.
    ///
    /// # Errors
    /// Returns an error for unsupported actions, actor mismatch, or identifier overflow.
    pub fn command_for_action(
        &self,
        action: &ExecutionAction,
        context: &BuntingCommandContext,
    ) -> Result<Command, AdapterError> {
        let (action_id, payload) = match action {
            ExecutionAction::Submit {
                action_id,
                local_order_id,
                order,
            } => {
                if order.participant_id != context.actor {
                    return Err(AdapterError::ActorMismatch);
                }
                (
                    *action_id,
                    CommandPayload::SubmitOrder(SubmitOrder {
                        order_id: OrderId::new(local_order_id.get()),
                        instrument_id: order.instrument_id,
                        participant_id: order.participant_id,
                        side: order.side,
                        quantity: order.quantity,
                        kind: order.kind,
                    }),
                )
            }
            ExecutionAction::Cancel {
                action_id,
                local_order_id,
                ..
            } => (
                *action_id,
                CommandPayload::CancelOrder(CancelOrder {
                    order_id: OrderId::new(local_order_id.get()),
                    participant_id: context.actor,
                }),
            ),
            ExecutionAction::Replace { .. }
            | ExecutionAction::QueryOrder { .. }
            | ExecutionAction::QueryOpenOrders { .. } => {
                return Err(AdapterError::UnsupportedAction);
            }
        };
        Ok(Command {
            run_id: context.run_id,
            command_id: CommandId::new(action_id.get()),
            correlation_id: context.correlation_id,
            logical_time: context.logical_time,
            expected_sequence: context.expected_sequence,
            actor: context.actor,
            payload,
        })
    }

    /// Converts committed canonical events into participant-side normalized reports.
    ///
    /// # Errors
    /// Returns an error when cumulative fill arithmetic overflows.
    pub fn normalize_committed_events(
        &mut self,
        actor: ParticipantId,
        events: &[EventEnvelope],
    ) -> Result<Vec<NormalizedVenueReport>, AdapterError> {
        let mut reports = Vec::new();
        for event in events {
            let (local, kind) = match &event.payload {
                EventPayload::OrderReceived { order } if order.participant_id == actor => {
                    self.owned_orders
                        .insert(LocalOrderId::new(order.order_id.get()));
                    continue;
                }
                EventPayload::OrderAccepted { order_id } => {
                    (LocalOrderId::new(order_id.get()), VenueReportKind::Accepted)
                }
                EventPayload::OrderRejected {
                    order_id: Some(order_id),
                    code,
                } => (
                    LocalOrderId::new(order_id.get()),
                    VenueReportKind::Rejected {
                        reason: format!("{code:?}"),
                    },
                ),
                EventPayload::OrderCanceled {
                    order_id,
                    participant_id,
                    ..
                } if *participant_id == actor => (
                    LocalOrderId::new(order_id.get()),
                    VenueReportKind::Cancelled,
                ),
                EventPayload::TradeExecuted {
                    maker_order_id,
                    taker_order_id,
                    buyer_id,
                    seller_id,
                    price,
                    quantity,
                    ..
                } if *buyer_id == actor || *seller_id == actor => {
                    let maker = LocalOrderId::new(maker_order_id.get());
                    let taker = LocalOrderId::new(taker_order_id.get());
                    let local = if self.owned_orders.contains(&maker) {
                        maker
                    } else if self.owned_orders.contains(&taker) {
                        taker
                    } else {
                        continue;
                    };
                    let prior = self
                        .cumulative_fills
                        .get(&local)
                        .copied()
                        .unwrap_or_else(|| QuantityLots::new(0));
                    let cumulative = prior
                        .checked_add(*quantity)
                        .ok_or(AdapterError::ArithmeticOverflow)?;
                    self.cumulative_fills.insert(local, cumulative);
                    (
                        local,
                        VenueReportKind::Fill {
                            last_quantity: *quantity,
                            cumulative_quantity: cumulative,
                            price: *price,
                        },
                    )
                }
                _ => continue,
            };
            reports.push(NormalizedVenueReport {
                report_id: ReportId::new(event.event_id.get()),
                source_sequence: Some(event.sequence.get()),
                client_order_id: None,
                local_order_id: Some(local),
                venue_order_id: Some(VenueOrderId::new(local.get().to_string())),
                leaves_quantity: None,
                kind,
            });
        }
        Ok(reports)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_market_events::{OrderKind, Side};
    use bunting_market_types::{InstrumentId, PriceTicks};
    use quarcc_execution_engine::ids::{ActionId, ClientOrderId};
    use quarcc_execution_engine::order::DesiredOrder;

    #[test]
    fn submit_preserves_action_id_and_expected_sequence() -> Result<(), AdapterError> {
        let action = ExecutionAction::Submit {
            action_id: ActionId::new(91),
            local_order_id: LocalOrderId::new(92),
            order: DesiredOrder {
                client_order_id: ClientOrderId::new(93),
                instrument_id: InstrumentId::new(5),
                participant_id: ParticipantId::new(6),
                side: Side::Buy,
                quantity: QuantityLots::new(7),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(8),
                },
            },
        };
        let command = BuntingExecutionAdapter::default().command_for_action(
            &action,
            &BuntingCommandContext {
                run_id: RunId::new(1),
                actor: ParticipantId::new(6),
                expected_sequence: EventSequence::new(44),
                logical_time: LogicalTimeNs::new(55),
                correlation_id: CorrelationId::new(66),
            },
        )?;
        assert_eq!(command.command_id, CommandId::new(91));
        assert_eq!(command.expected_sequence, EventSequence::new(44));
        Ok(())
    }
}
