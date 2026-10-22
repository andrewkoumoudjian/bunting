//! Bunting Native Protocol mapping (ADR 0040): BNP requests to canonical
//! engine commands, and committed canonical events to one participant's
//! private BNP messages. Sans-I/O and stateless.
//!
//! Identity is stateless too. A participant's client order ID (unique for
//! the run, across connections, as on an OUCH session) is placed in that
//! participant's BNP namespace, so the canonical order ID is
//! `namespace(run, participant) << 64 | client_order_id`, and the command
//! ID of a new order, a cancel or a kill switch is the same construction in
//! its own namespace. A resent order is therefore an idempotent duplicate,
//! two participants can never collide, and committed events map back to
//! client order IDs without any server-side session state.

use crate::competition::{DealChanges, account, negotiation_visible, news_visible, tender_visible};
use crate::{ApplicationError, VerifiedActor};
use bnp_wire::{
    CancelReason as WireCancelReason, CashBalance, ClientMessage, CurrencyInfo, InstrumentInfo,
    InstrumentKind as WireInstrumentKind, Liquidity, Listing, ListingInfo, ListingReference,
    MAX_REFERENCE_INSTRUMENTS, NegotiationInfo, NegotiationStatus, NewOrder,
    NewsAudience as WireNewsAudience, OpenOrder, OperatorAction, OrderType,
    OtcDecision as WireOtcDecision, Position, RejectReason, ServerMessage, Side as WireSide, Stamp,
    TenderInfo, TenderStatus, TimeInForce,
};
use bunting_engine::simulation::{InstrumentKind, OtcState, TenderState};
use bunting_engine::{OwnedOrderState, RunState};
use bunting_market_events::{
    CancelOrder, CancelReason, Command, CommandPayload, EventEnvelope, EventPayload, NewsAudience,
    OrderKind, OtcDecision, RejectCode, Side, SimulationCommand, SimulationCommandRequest,
    SimulationEvent, SubmitOrder, TenderDecision, TimeInForcePolicy,
};
use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, EventSequence, InstrumentId, ListingKey, LogicalTimeNs,
    MoneyMinor, NegotiationId, NewsId, OrderId, ParticipantId, PriceTicks, QuantityLots, RunId,
    TenderId, VenueId,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;

/// A BNP request that cannot become a command. The session answers with a
/// `Reject` naming it; nothing reaches the engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BnpMappingError {
    /// Client order and request IDs start at 1.
    ZeroIdentifier,
    /// Market orders execute immediately; they take IOC only, without
    /// post-only or a display size.
    InvalidMarketOrder,
    /// A display size must be positive and no larger than the order.
    InvalidDisplayQuantity,
}

impl fmt::Display for BnpMappingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ZeroIdentifier => "client order and request IDs must be at least 1",
            Self::InvalidMarketOrder => {
                "market orders are immediate-or-cancel, without post-only or display size"
            }
            Self::InvalidDisplayQuantity => "display quantity must be between 1 and the quantity",
        })
    }
}

impl std::error::Error for BnpMappingError {}

/// The namespaces of one participant's BNP identities in one run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BnpIdentity {
    run_id: RunId,
    participant: ParticipantId,
    orders: u64,
    new_orders: u64,
    cancels: u64,
    kill_switches: u64,
    kill_switch_resets: u64,
    operator_commands: u64,
    deal_commands: u64,
    negotiations: u64,
}

impl BnpIdentity {
    #[must_use]
    pub fn new(run_id: RunId, participant: ParticipantId) -> Self {
        let space = |domain: &[u8]| namespace(domain, run_id, participant);
        Self {
            run_id,
            participant,
            orders: space(b"order"),
            new_orders: space(b"new-order"),
            cancels: space(b"cancel"),
            kill_switches: space(b"kill-switch"),
            kill_switch_resets: space(b"kill-switch-reset"),
            operator_commands: space(b"operator"),
            deal_commands: space(b"deal"),
            negotiations: space(b"negotiation"),
        }
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn participant(&self) -> ParticipantId {
        self.participant
    }

    /// The canonical order ID of a client order ID.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for 0.
    pub fn order_id(&self, client_order_id: u64) -> Result<OrderId, BnpMappingError> {
        scoped(self.orders, client_order_id).map(OrderId::new)
    }

    /// The client order ID of a canonical order ID, or 0 when the order was
    /// not entered over BNP by this participant.
    #[must_use]
    pub fn client_order_id(&self, order_id: OrderId) -> u64 {
        unscoped(self.orders, order_id.get())
    }

    /// The command for a new order. Sequence and logical time are stamped
    /// when the sequencer releases it.
    ///
    /// # Errors
    /// Returns a [`BnpMappingError`] for an order the protocol refuses.
    pub fn new_order(
        &self,
        order: &NewOrder,
        correlation_id: CorrelationId,
    ) -> Result<Command, BnpMappingError> {
        let kind = order_kind(order)?;
        let listing_key = listing_key(order.listing);
        Ok(self.command(
            scoped(self.new_orders, order.client_order_id)?,
            correlation_id,
            CommandPayload::SubmitOrderAtListing {
                listing_key,
                order: SubmitOrder {
                    order_id: self.order_id(order.client_order_id)?,
                    instrument_id: listing_key.instrument_id,
                    participant_id: self.participant,
                    side: side(order.side),
                    quantity: QuantityLots::new(order.quantity),
                    kind,
                    anonymous: order.anonymous,
                },
            },
        ))
    }

    /// The command that cancels one of this participant's BNP orders.
    /// Cancelling the same order twice is one idempotent command.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for client order ID 0.
    pub fn cancel(
        &self,
        client_order_id: u64,
        correlation_id: CorrelationId,
    ) -> Result<Command, BnpMappingError> {
        Ok(self.command(
            scoped(self.cancels, client_order_id)?,
            correlation_id,
            CommandPayload::CancelOrder(CancelOrder {
                order_id: self.order_id(client_order_id)?,
                participant_id: self.participant,
            }),
        ))
    }

    /// The command that activates this participant's kill switch.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for request ID 0.
    pub fn kill_switch(
        &self,
        request_id: u64,
        correlation_id: CorrelationId,
    ) -> Result<Command, BnpMappingError> {
        Ok(self.command(
            scoped(self.kill_switches, request_id)?,
            correlation_id,
            CommandPayload::ActivateKillSwitch,
        ))
    }

    /// The command that resets this participant's kill switch.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for request ID 0.
    pub fn kill_switch_reset(
        &self,
        request_id: u64,
        correlation_id: CorrelationId,
    ) -> Result<Command, BnpMappingError> {
        Ok(self.command(
            scoped(self.kill_switch_resets, request_id)?,
            correlation_id,
            CommandPayload::ResetKillSwitch,
        ))
    }

    /// The engine request for an instructor's or administrator's command
    /// (ADR 0042). `self` is the operator's identity: its "participant" is
    /// the operator's actor ID. Logical time and expected sequence are set
    /// when the sequencer releases it.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for request ID 0.
    pub fn operator_command(
        &self,
        request_id: u64,
        action: &OperatorAction,
        correlation_id: CorrelationId,
    ) -> Result<SimulationCommandRequest, BnpMappingError> {
        self.simulation_request(
            scoped(self.operator_commands, request_id)?,
            correlation_id,
            simulation_command(action),
        )
    }

    /// The canonical negotiation ID of this participant's own one: placed
    /// in its namespace, so two teams' IDs never collide.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for 0.
    pub fn negotiation_id(&self, own: u64) -> Result<NegotiationId, BnpMappingError> {
        scoped(self.negotiations, own).map(NegotiationId::new)
    }

    /// The command for a tender decision or an OTC request (ADR 0043);
    /// `None` for any other message. Request IDs are unique for the
    /// participant for the run: a reused one is an idempotent duplicate.
    ///
    /// # Errors
    /// Returns [`BnpMappingError::ZeroIdentifier`] for a zero request or
    /// negotiation ID.
    pub fn deal_command(
        &self,
        message: &ClientMessage,
        correlation_id: CorrelationId,
    ) -> Result<Option<SimulationCommandRequest>, BnpMappingError> {
        let (request_id, payload) = match message {
            ClientMessage::TenderDecision {
                request_id,
                tender_id,
                accept,
            } => (
                *request_id,
                SimulationCommand::DecideTender {
                    tender_id: TenderId::new(*tender_id),
                    decision: if *accept {
                        TenderDecision::Accept
                    } else {
                        TenderDecision::Decline
                    },
                },
            ),
            ClientMessage::OtcPropose {
                request_id,
                negotiation_id,
                counterparty_id,
                instrument_id,
                side: proposer_side,
                quantity,
                price,
                expires_at_ns,
            } => (
                *request_id,
                SimulationCommand::OpenOtc {
                    negotiation_id: self.negotiation_id(*negotiation_id)?,
                    counterparty_id: ParticipantId::new(*counterparty_id),
                    instrument_id: InstrumentId::new(*instrument_id),
                    side: side(*proposer_side),
                    quantity: QuantityLots::new(*quantity),
                    price: PriceTicks::new(*price),
                    expires_at: LogicalTimeNs::new(*expires_at_ns),
                },
            ),
            ClientMessage::OtcCounter {
                request_id,
                negotiation_id,
                quantity,
                price,
            } => (
                *request_id,
                SimulationCommand::CounterOtc {
                    negotiation_id: NegotiationId::new(*negotiation_id),
                    quantity: QuantityLots::new(*quantity),
                    price: PriceTicks::new(*price),
                },
            ),
            ClientMessage::OtcDecision {
                request_id,
                negotiation_id,
                decision,
            } => (
                *request_id,
                SimulationCommand::DecideOtc {
                    negotiation_id: NegotiationId::new(*negotiation_id),
                    decision: match decision {
                        WireOtcDecision::Accept => OtcDecision::Accept,
                        WireOtcDecision::Reject => OtcDecision::Reject,
                        WireOtcDecision::Break => OtcDecision::Break,
                    },
                },
            ),
            _ => return Ok(None),
        };
        self.simulation_request(
            scoped(self.deal_commands, request_id)?,
            correlation_id,
            payload,
        )
        .map(Some)
    }

    #[expect(
        clippy::unnecessary_wraps,
        reason = "callers chain it after fallible ID mapping"
    )]
    const fn simulation_request(
        &self,
        command_id: u128,
        correlation_id: CorrelationId,
        payload: SimulationCommand,
    ) -> Result<SimulationCommandRequest, BnpMappingError> {
        Ok(SimulationCommandRequest {
            run_id: self.run_id,
            command_id: CommandId::new(command_id),
            correlation_id,
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: EventSequence::new(0),
            actor: self.participant,
            payload,
        })
    }

    fn command(
        &self,
        command_id: u128,
        correlation_id: CorrelationId,
        payload: CommandPayload,
    ) -> Command {
        Command {
            run_id: self.run_id,
            command_id: CommandId::new(command_id),
            correlation_id,
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: EventSequence::new(0),
            actor: self.participant,
            payload,
        }
    }

    fn is_cancel(&self, command_id: CommandId) -> bool {
        command_id.get() >> 64 == u128::from(self.cancels)
    }

    /// Whether a command came from this participant over BNP.
    fn is_own_command(&self, command_id: CommandId) -> bool {
        let high = command_id.get() >> 64;
        [
            self.new_orders,
            self.cancels,
            self.kill_switches,
            self.kill_switch_resets,
        ]
        .into_iter()
        .any(|space| high == u128::from(space))
    }

    /// This participant's private messages for one committed batch, in
    /// event order. Events about other participants produce nothing.
    /// `references` are the batch's public order references
    /// ([`crate::order_references`], read right after the commit).
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per canonical event keeps the attribution rules side by side"
    )]
    pub fn private_messages(
        &self,
        events: &[EventEnvelope],
        references: &[(OrderId, u64)],
    ) -> Vec<ServerMessage> {
        let reference = |order_id: OrderId| {
            references
                .binary_search_by_key(&order_id, |(id, _)| *id)
                .ok()
                .and_then(|index| references.get(index))
                .map(|(_, reference)| *reference)
        };
        let me = self.participant;
        let mine = |order_id: OrderId| unscoped(self.orders, order_id.get()) != 0;
        // OrderAccepted names only the order; its details are in the same
        // command's OrderReceived.
        let received: BTreeMap<OrderId, (&SubmitOrder, Option<ListingKey>)> = events
            .iter()
            .filter_map(|event| match &event.payload {
                EventPayload::OrderReceived { order, listing_key } => {
                    Some((order.order_id, (order, *listing_key)))
                }
                _ => None,
            })
            .collect();
        let mut messages = Vec::new();
        for event in events {
            let stamp = Stamp {
                sequence: event.sequence.get(),
                logical_time_ns: event.logical_time.get(),
            };
            let client_order_id = |order_id: OrderId| self.client_order_id(order_id);
            match &event.payload {
                EventPayload::OrderAccepted { order_id } if mine(*order_id) => {
                    if let Some((order, listing_key)) = received.get(order_id) {
                        let listing_key = listing_key
                            .unwrap_or(ListingKey::new(VenueId::new(0), order.instrument_id));
                        messages.push(ServerMessage::OrderAccepted {
                            stamp,
                            client_order_id: client_order_id(*order_id),
                            order_id: order_id.get(),
                            listing: wire_listing(listing_key),
                            side: wire_side(order.side),
                            quantity: order.quantity.get(),
                            price: order.kind.limit_price().map(PriceTicks::get),
                        });
                    }
                }
                EventPayload::OrderRejected { order_id, code }
                    if self.is_own_command(event.command_id) =>
                {
                    let client_order_id = order_id.map_or(0, client_order_id);
                    let reason = reject_reason(*code);
                    messages.push(if self.is_cancel(event.command_id) {
                        ServerMessage::CancelRejected {
                            stamp,
                            client_order_id,
                            reason,
                        }
                    } else {
                        ServerMessage::OrderRejected {
                            stamp,
                            client_order_id,
                            reason,
                        }
                    });
                }
                EventPayload::OrderRested {
                    order_id,
                    participant_id,
                    price,
                    remaining,
                    ..
                } if *participant_id == me => messages.push(ServerMessage::OrderRested {
                    stamp,
                    client_order_id: client_order_id(*order_id),
                    order_id: order_id.get(),
                    price: price.get(),
                    remaining: remaining.get(),
                    reference: reference(*order_id),
                }),
                EventPayload::OrderReduced {
                    order_id,
                    remaining,
                } if mine(*order_id) => messages.push(ServerMessage::OrderReduced {
                    stamp,
                    client_order_id: client_order_id(*order_id),
                    order_id: order_id.get(),
                    remaining: remaining.get(),
                    reference: reference(*order_id),
                }),
                EventPayload::OrderCompleted { order_id } if mine(*order_id) => {
                    messages.push(ServerMessage::OrderDone {
                        stamp,
                        client_order_id: client_order_id(*order_id),
                        order_id: order_id.get(),
                    });
                }
                EventPayload::OrderCanceled {
                    order_id,
                    participant_id,
                    remaining,
                    reason,
                    ..
                } if *participant_id == me => messages.push(ServerMessage::OrderCanceled {
                    stamp,
                    client_order_id: client_order_id(*order_id),
                    order_id: order_id.get(),
                    remaining: remaining.get(),
                    reason: cancel_reason(*reason),
                }),
                EventPayload::TradeExecuted {
                    instrument_id,
                    listing_key,
                    maker_order_id,
                    taker_order_id,
                    buyer_id,
                    seller_id,
                    price,
                    quantity,
                    buyer_fee,
                    seller_fee,
                } => {
                    let listing = wire_listing(
                        listing_key.unwrap_or(ListingKey::new(VenueId::new(0), *instrument_id)),
                    );
                    // A participant may sit on both sides (a self-trade):
                    // each side is its own fill.
                    for (participant, side, fee) in [
                        (*buyer_id, Side::Buy, *buyer_fee),
                        (*seller_id, Side::Sell, *seller_fee),
                    ] {
                        if participant != me {
                            continue;
                        }
                        // The side's order is the maker or the taker,
                        // whichever belongs to this participant on this side.
                        let (order_id, liquidity) =
                            if self.order_on_side(*maker_order_id, *taker_order_id, side, events) {
                                (*maker_order_id, Liquidity::Maker)
                            } else {
                                (*taker_order_id, Liquidity::Taker)
                            };
                        messages.push(ServerMessage::Fill {
                            stamp,
                            client_order_id: client_order_id(order_id),
                            order_id: order_id.get(),
                            listing,
                            side: wire_side(side),
                            price: price.get(),
                            quantity: quantity.get(),
                            fee: fee.get(),
                            liquidity,
                        });
                    }
                }
                EventPayload::PositionChanged {
                    participant_id,
                    instrument_id,
                    delta,
                } if *participant_id == me => messages.push(ServerMessage::PositionChanged {
                    stamp,
                    instrument_id: instrument_id.get(),
                    delta: delta.get(),
                }),
                EventPayload::BalanceChanged {
                    participant_id,
                    delta,
                } if *participant_id == me => messages.push(ServerMessage::BalanceChanged {
                    stamp,
                    delta: delta.get(),
                }),
                EventPayload::KillSwitchActivated if event.actor == me => {
                    messages.push(ServerMessage::KillSwitchActivated { stamp });
                }
                EventPayload::KillSwitchReset if event.actor == me => {
                    messages.push(ServerMessage::KillSwitchReset { stamp });
                }
                _ => {}
            }
        }
        messages
    }

    /// Whether the maker order is the one on `side`: the taker's side is
    /// the side of the command that traded, read from its `OrderReceived`
    /// when present; otherwise the maker is on `side` exactly when the
    /// taker order is not this participant's on that side.
    fn order_on_side(
        &self,
        maker: OrderId,
        taker: OrderId,
        side: Side,
        events: &[EventEnvelope],
    ) -> bool {
        let taker_side = events.iter().find_map(|event| match &event.payload {
            EventPayload::OrderReceived { order, .. } if order.order_id == taker => {
                Some(order.side)
            }
            _ => None,
        });
        match taker_side {
            Some(taker_side) => taker_side != side,
            // No OrderReceived (a legacy record): prefer the order in this
            // participant's namespace.
            None => unscoped(self.orders, maker.get()) != 0,
        }
    }

    /// This participant's live orders, for state recovery after a gap.
    #[must_use]
    pub fn open_orders(&self, state: &RunState) -> Vec<OpenOrder> {
        state
            .ownership()
            .values()
            .filter(|owned| {
                owned.participant_id == self.participant && owned.state == OwnedOrderState::Active
            })
            .map(|owned| OpenOrder {
                client_order_id: self.client_order_id(owned.order_id),
                order_id: owned.order_id.get(),
                listing: wire_listing(owned.listing_key),
                side: wire_side(owned.side),
                price: owned.limit_price.get(),
                original_quantity: owned.original_quantity.get(),
                remaining_quantity: owned.remaining_quantity.get(),
            })
            .collect()
    }
}

/// The engine command for an operator action.
fn simulation_command(action: &OperatorAction) -> SimulationCommand {
    match action.clone() {
        OperatorAction::Start => SimulationCommand::StartRun,
        OperatorAction::Pause => SimulationCommand::PauseRun,
        OperatorAction::Resume => SimulationCommand::ResumeRun,
        OperatorAction::Advance { steps } => SimulationCommand::Advance { steps },
        OperatorAction::Terminate { reason } => SimulationCommand::Terminate { reason },
        OperatorAction::Halt {
            instrument_id,
            halted,
            reason,
        } => SimulationCommand::SetListingHalt {
            instrument_id: InstrumentId::new(instrument_id),
            halted,
            reason,
        },
        OperatorAction::MassCancel {
            participant_id,
            instrument_id,
        } => SimulationCommand::MassCancel {
            participant_id: participant_id.map(ParticipantId::new),
            instrument_id: instrument_id.map(InstrumentId::new),
        },
        OperatorAction::PublishNews {
            news_id,
            audience,
            headline,
            body,
        } => SimulationCommand::PublishNews {
            news_id: NewsId::new(news_id),
            audience: match audience {
                WireNewsAudience::Public => NewsAudience::Public,
                WireNewsAudience::Participant(id) => {
                    NewsAudience::Participant(ParticipantId::new(id))
                }
                WireNewsAudience::Team(id) => NewsAudience::Team(id),
                WireNewsAudience::Role(role) => NewsAudience::Role(role),
            },
            headline,
            body,
        },
        OperatorAction::Score => SimulationCommand::ScoreIteration,
        OperatorAction::Fine {
            participant_id,
            currency_id,
            amount,
            reason,
        } => SimulationCommand::ApplyFine {
            participant_id: ParticipantId::new(participant_id),
            currency_id: CurrencyId::new(currency_id),
            amount: MoneyMinor::new(amount),
            reason,
        },
        OperatorAction::OpenTender {
            tender_id,
            participant_id,
            instrument_id,
            side: tender_side,
            quantity,
            price,
            expires_at_ns,
        } => SimulationCommand::OpenTender {
            tender_id: TenderId::new(tender_id),
            participant_id: ParticipantId::new(participant_id),
            instrument_id: InstrumentId::new(instrument_id),
            side: side(tender_side),
            quantity: QuantityLots::new(quantity),
            price: PriceTicks::new(price),
            expires_at: LogicalTimeNs::new(expires_at_ns),
        },
    }
}

/// The tender and negotiation changes of one committed batch that `actor`
/// may see (ADR 0043), stamped with their event sequence. `changes` are
/// the states read right after the commit
/// ([`crate::competition::deal_changes`]).
#[must_use]
pub fn deal_messages(
    changes: &DealChanges,
    events: &[EventEnvelope],
    actor: &VerifiedActor,
) -> Vec<ServerMessage> {
    events
        .iter()
        .filter_map(|event| {
            let stamp = Stamp {
                sequence: event.sequence.get(),
                logical_time_ns: event.logical_time.get(),
            };
            match &event.payload {
                EventPayload::Simulation(SimulationEvent::TenderChanged { tender_id, status }) => {
                    let tender = changes
                        .tenders
                        .iter()
                        .find(|tender| tender.tender_id == *tender_id)
                        .filter(|tender| tender_visible(tender, actor))?;
                    Some(ServerMessage::Tender {
                        stamp,
                        tender: tender_info(tender, status)?,
                    })
                }
                EventPayload::Simulation(SimulationEvent::OtcChanged {
                    negotiation_id,
                    status,
                }) => {
                    let otc = changes
                        .negotiations
                        .iter()
                        .find(|otc| otc.negotiation_id == *negotiation_id)
                        .filter(|otc| negotiation_visible(otc, actor))?;
                    Some(ServerMessage::Negotiation {
                        stamp,
                        negotiation: negotiation_info(otc, status)?,
                    })
                }
                _ => None,
            }
        })
        .collect()
}

/// Whether a batch holds a tender or negotiation change.
#[must_use]
pub fn has_deal_messages(events: &[EventEnvelope]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Simulation(
                SimulationEvent::TenderChanged { .. } | SimulationEvent::OtcChanged { .. }
            )
        )
    })
}

/// Every tender and negotiation `actor` may see, in ID order.
#[must_use]
pub fn deals(state: &RunState, actor: &VerifiedActor) -> (Vec<TenderInfo>, Vec<NegotiationInfo>) {
    let simulation = state.simulation();
    let tenders = simulation
        .tenders
        .values()
        .filter(|tender| tender_visible(tender, actor))
        .filter_map(|tender| tender_info(tender, &tender.status))
        .collect();
    let negotiations = simulation
        .otc
        .values()
        .filter(|otc| negotiation_visible(otc, actor))
        .filter_map(|otc| negotiation_info(otc, &otc.status))
        .collect();
    (tenders, negotiations)
}

fn tender_info(tender: &TenderState, status: &str) -> Option<TenderInfo> {
    Some(TenderInfo {
        tender_id: tender.tender_id.get(),
        participant_id: tender.participant_id.get(),
        instrument_id: tender.instrument_id.get(),
        side: wire_side(tender.side),
        quantity: tender.quantity.get(),
        price: tender.price.get(),
        expires_at_ns: tender.expires_at.get(),
        status: match status {
            "open" => TenderStatus::Open,
            "accepted" => TenderStatus::Accepted,
            "declined" => TenderStatus::Declined,
            "expired" => TenderStatus::Expired,
            _ => return None,
        },
    })
}

fn negotiation_info(otc: &OtcState, status: &str) -> Option<NegotiationInfo> {
    Some(NegotiationInfo {
        negotiation_id: otc.negotiation_id.get(),
        proposer_id: otc.proposer_id.get(),
        counterparty_id: otc.counterparty_id.get(),
        instrument_id: otc.instrument_id.get(),
        side: wire_side(otc.side),
        quantity: otc.quantity.get(),
        price: otc.price.get(),
        expires_at_ns: otc.expires_at.get(),
        status: match status {
            "proposed" => NegotiationStatus::Proposed,
            "countered" => NegotiationStatus::Countered,
            "accepted" => NegotiationStatus::Accepted,
            "rejected" => NegotiationStatus::Rejected,
            "broken" => NegotiationStatus::Broken,
            "expired" => NegotiationStatus::Expired,
            _ => return None,
        },
        awaiting: otc.awaiting.get(),
    })
}

/// The run's public facts in one committed batch, as `actor` may see them
/// (ADR 0042): lifecycle changes, instrument halts, and news whose audience
/// includes the actor (its headline and body read from `state`). Stamped
/// with their event sequence, like private reports.
#[must_use]
pub fn run_messages(
    state: &RunState,
    events: &[EventEnvelope],
    actor: &VerifiedActor,
) -> Vec<ServerMessage> {
    events
        .iter()
        .filter_map(|event| {
            let stamp = Stamp {
                sequence: event.sequence.get(),
                logical_time_ns: event.logical_time.get(),
            };
            let EventPayload::Simulation(simulation) = &event.payload else {
                return None;
            };
            match simulation {
                SimulationEvent::LifecycleChanged { status } => Some(ServerMessage::RunStatus {
                    stamp,
                    status: status.clone(),
                }),
                SimulationEvent::ListingHaltChanged {
                    instrument_id,
                    halted,
                } => Some(ServerMessage::ListingHalt {
                    stamp,
                    instrument_id: instrument_id.get(),
                    halted: *halted,
                }),
                SimulationEvent::NewsPublished { news_id, audience }
                    if news_visible(audience, actor) =>
                {
                    state
                        .simulation()
                        .news
                        .iter()
                        .find(|item| item.news_id == *news_id)
                        .map(|item| ServerMessage::News {
                            stamp,
                            news_id: news_id.get(),
                            headline: item.headline.clone(),
                            body: item.body.clone(),
                        })
                }
                _ => None,
            }
        })
        .collect()
}

/// Whether a batch holds anything [`run_messages`] maps, so a session reads
/// the run state only when it must.
#[must_use]
pub fn has_run_messages(events: &[EventEnvelope]) -> bool {
    events.iter().any(|event| {
        matches!(
            &event.payload,
            EventPayload::Simulation(
                SimulationEvent::LifecycleChanged { .. }
                    | SimulationEvent::ListingHaltChanged { .. }
                    | SimulationEvent::NewsPublished { .. }
            )
        )
    })
}

/// Every listing of the run, with its symbol.
#[must_use]
pub fn listings(state: &RunState) -> Vec<ListingInfo> {
    state
        .listings()
        .iter()
        .map(|(key, listing)| ListingInfo {
            listing: wire_listing(*key),
            symbol: listing.definition().symbol().to_owned(),
        })
        .collect()
}

/// One page of reference data (BNP 1.1, ADR 0054 §2): instruments after
/// `after` in ID order, at most `limit` (0 or more than 64 means 64), their
/// listings, and on the first page every currency they use. Currency code
/// and name stay empty until a scenario declares them (ADR 0054 §10).
#[must_use]
pub fn reference_data(
    state: &RunState,
    after: Option<u128>,
    limit: u16,
) -> (
    Option<u128>,
    Vec<CurrencyInfo>,
    Vec<InstrumentInfo>,
    Vec<ListingReference>,
) {
    let limit = if limit == 0 || limit > MAX_REFERENCE_INSTRUMENTS {
        MAX_REFERENCE_INSTRUMENTS
    } else {
        limit
    };
    let mut remaining = state
        .instruments()
        .values()
        .filter(|definition| after.is_none_or(|after| definition.instrument_id.get() > after));
    let page: Vec<_> = remaining.by_ref().take(usize::from(limit)).collect();
    let next_after = remaining
        .next()
        .and(page.last().map(|definition| definition.instrument_id.get()));
    let currencies = if after.is_none() {
        let mut ids: Vec<u128> = state
            .instruments()
            .values()
            .map(|definition| definition.currency.get())
            .chain(std::iter::once(state.reporting_currency().get()))
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter()
            .map(|currency_id| CurrencyInfo {
                currency_id,
                code: String::new(),
                name: String::new(),
                minor_decimals: 2,
            })
            .collect()
    } else {
        Vec::new()
    };
    let halted = &state.simulation().halted_instruments;
    let instruments: Vec<InstrumentInfo> = page
        .iter()
        .map(|definition| instrument_info(definition))
        .collect();
    let listings = state
        .listings()
        .iter()
        .filter(|(key, _)| {
            page.iter()
                .any(|definition| definition.instrument_id == key.instrument_id)
        })
        .map(|(key, listing)| {
            let definition = listing.definition();
            let fees = definition.fees();
            ListingReference {
                listing: wire_listing(*key),
                symbol: definition.symbol().to_owned(),
                min_price: definition.price_bounds().min.get(),
                max_price: definition.price_bounds().max.get(),
                tick_size: definition.tick_size(),
                taker_fee_per_lot: fees.taker_per_lot.get(),
                maker_fee_per_lot: fees.maker_per_lot.get(),
                broker_ids: definition.broker_ids(),
                primary: definition.primary(),
                halted: halted.contains(&key.instrument_id),
                in_window: true,
            }
        })
        .collect();
    (next_after, currencies, instruments, listings)
}

fn instrument_info(definition: &bunting_engine::InstrumentDefinition) -> InstrumentInfo {
    let mut info = InstrumentInfo {
        instrument_id: definition.instrument_id.get(),
        symbol: definition.symbol.clone(),
        kind: WireInstrumentKind::Equity,
        underlying: None,
        strike: None,
        expiry_ns: None,
        is_call: false,
        physical_delivery: false,
        coupon_bps: 0,
        components: Vec::new(),
        multiplier: definition.multiplier,
        currency_id: definition.currency.get(),
        quoted_decimals: definition.quoted_decimals,
        shortable: definition.shortable,
        tradeable: true,
        window_start_ns: None,
        window_stop_ns: None,
        display_unit: definition.display_unit.clone(),
        description: definition.description.clone(),
        opening_mark: definition.opening_mark.map(PriceTicks::get),
        max_order_quantity: None,
    };
    match &definition.kind {
        InstrumentKind::Equity => {}
        InstrumentKind::Currency => info.kind = WireInstrumentKind::Currency,
        InstrumentKind::Bond {
            coupon_bps,
            maturity,
        } => {
            info.kind = WireInstrumentKind::Bond;
            info.coupon_bps = *coupon_bps;
            info.expiry_ns = Some(maturity.get());
        }
        InstrumentKind::Option {
            underlying,
            strike,
            expiry,
            is_call,
        } => {
            info.kind = WireInstrumentKind::Option;
            info.underlying = Some(underlying.get());
            info.strike = Some(strike.get());
            info.expiry_ns = Some(expiry.get());
            info.is_call = *is_call;
        }
        InstrumentKind::Future {
            underlying,
            expiry,
            physical_delivery,
        } => {
            info.kind = WireInstrumentKind::Future;
            info.underlying = Some(underlying.get());
            info.expiry_ns = Some(expiry.get());
            info.physical_delivery = *physical_delivery;
        }
        InstrumentKind::Commodity => info.kind = WireInstrumentKind::Commodity,
        InstrumentKind::Synthetic { components } => {
            info.kind = WireInstrumentKind::Synthetic;
            info.components = components
                .iter()
                .map(|(instrument, weight)| (instrument.get(), *weight))
                .collect();
        }
    }
    info
}

/// The actor's cash and positions from the authoritative ledger.
///
/// # Errors
/// Returns an error for an actor without a participant account.
pub fn account_balances(
    state: &RunState,
    actor: &VerifiedActor,
) -> Result<(Vec<CashBalance>, Vec<Position>), ApplicationError> {
    let view = account(state, actor)?;
    let cash = view
        .cash
        .iter()
        .map(|cash| CashBalance {
            currency_id: cash.currency_id.get(),
            balance: cash.balance.get(),
            reserved: cash.reserved.get(),
        })
        .collect();
    let positions = view
        .holdings
        .iter()
        .map(|holding| Position {
            instrument_id: holding.instrument_id.get(),
            position: holding.position.get(),
            open_buy: holding.open_buy.get(),
            open_sell: holding.open_sell.get(),
        })
        .collect();
    Ok((cash, positions))
}

/// The canonical listing of a wire listing.
#[must_use]
pub fn listing_key(listing: Listing) -> ListingKey {
    ListingKey::new(
        VenueId::new(listing.venue_id),
        InstrumentId::new(listing.instrument_id),
    )
}

#[must_use]
pub fn wire_listing(key: ListingKey) -> Listing {
    Listing {
        venue_id: key.venue_id.get(),
        instrument_id: key.instrument_id.get(),
    }
}

#[must_use]
pub const fn reject_reason(code: RejectCode) -> RejectReason {
    match code {
        RejectCode::DuplicateOrderId => RejectReason::DuplicateOrderId,
        RejectCode::InvalidOrderId => RejectReason::InvalidOrderId,
        RejectCode::UnknownOrder => RejectReason::UnknownOrder,
        RejectCode::NotOrderOwner => RejectReason::NotOrderOwner,
        RejectCode::KillSwitchActive => RejectReason::KillSwitchActive,
        RejectCode::RunNotActive => RejectReason::RunNotActive,
        RejectCode::ListingHalted => RejectReason::ListingHalted,
        RejectCode::ParticipantDisabled => RejectReason::ParticipantDisabled,
        RejectCode::InvalidQuantity => RejectReason::InvalidQuantity,
        RejectCode::InvalidInstrument => RejectReason::InvalidInstrument,
        RejectCode::PriceOutOfBounds => RejectReason::PriceOutOfBounds,
        RejectCode::PriceNotOnTick => RejectReason::PriceNotOnTick,
        RejectCode::MaxOrderQuantity => RejectReason::MaxOrderQuantity,
        RejectCode::MaxOpenOrderQuantity => RejectReason::MaxOpenOrderQuantity,
        RejectCode::MaxLiveOrders => RejectReason::MaxLiveOrders,
        RejectCode::PositionLimit => RejectReason::PositionLimit,
        RejectCode::InsufficientCash => RejectReason::InsufficientCash,
        RejectCode::InsufficientInventory => RejectReason::InsufficientInventory,
        RejectCode::InsufficientLiquidity => RejectReason::InsufficientLiquidity,
        RejectCode::PostOnlyWouldCross => RejectReason::PostOnlyWouldCross,
        RejectCode::InvalidTimeInForce => RejectReason::InvalidTimeInForce,
        RejectCode::MarketClosed => RejectReason::MarketClosed,
        RejectCode::CallPhaseOrderType => RejectReason::CallPhaseOrderType,
        RejectCode::LogicalTimeRegression => RejectReason::LogicalTimeRegression,
        RejectCode::SequenceConflict => RejectReason::SequenceConflict,
        RejectCode::ArithmeticOverflow => RejectReason::ArithmeticOverflow,
    }
}

#[must_use]
pub const fn cancel_reason(reason: CancelReason) -> WireCancelReason {
    match reason {
        CancelReason::Requested => WireCancelReason::Requested,
        CancelReason::KillSwitch => WireCancelReason::KillSwitch,
        CancelReason::MarketRemainder => WireCancelReason::MarketRemainder,
        CancelReason::MassCancel => WireCancelReason::MassCancel,
        CancelReason::Expired => WireCancelReason::Expired,
        CancelReason::Halt => WireCancelReason::Halt,
    }
}

const fn side(side: WireSide) -> Side {
    match side {
        WireSide::Buy => Side::Buy,
        WireSide::Sell => Side::Sell,
    }
}

const fn wire_side(side: Side) -> WireSide {
    match side {
        Side::Buy => WireSide::Buy,
        Side::Sell => WireSide::Sell,
    }
}

fn order_kind(order: &NewOrder) -> Result<OrderKind, BnpMappingError> {
    if let Some(display) = order.display_quantity
        && (display <= 0 || display > order.quantity)
    {
        return Err(BnpMappingError::InvalidDisplayQuantity);
    }
    let price = match order.order_type {
        OrderType::Market => {
            return if order.time_in_force == TimeInForce::Ioc
                && !order.post_only
                && order.display_quantity.is_none()
            {
                Ok(OrderKind::Market)
            } else {
                Err(BnpMappingError::InvalidMarketOrder)
            };
        }
        OrderType::Limit { price } => PriceTicks::new(price),
    };
    let time_in_force = match order.time_in_force {
        TimeInForce::Gtc if !order.post_only && order.display_quantity.is_none() => {
            return Ok(OrderKind::Limit { price });
        }
        TimeInForce::Gtc => TimeInForcePolicy::Gtc,
        TimeInForce::Ioc => TimeInForcePolicy::Ioc,
        TimeInForce::Fok => TimeInForcePolicy::Fok,
        TimeInForce::Day => TimeInForcePolicy::Day,
        TimeInForce::Gtd { expires_at_ns } => TimeInForcePolicy::Gtd {
            expires_at: LogicalTimeNs::new(expires_at_ns),
        },
    };
    Ok(OrderKind::LimitWithPolicy {
        price,
        time_in_force,
        post_only: order.post_only,
        display_quantity: order.display_quantity.map(QuantityLots::new),
    })
}

fn namespace(domain: &[u8], run_id: RunId, participant: ParticipantId) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"bunting.bnp-namespace.v1\0");
    hasher.update(domain);
    hasher.update(run_id.get().to_be_bytes());
    hasher.update(participant.get().to_be_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes).max(1)
}

fn scoped(namespace: u64, local: u64) -> Result<u128, BnpMappingError> {
    if local == 0 {
        return Err(BnpMappingError::ZeroIdentifier);
    }
    Ok((u128::from(namespace) << 64) | u128::from(local))
}

fn unscoped(namespace: u64, canonical: u128) -> u64 {
    if canonical >> 64 == u128::from(namespace) {
        u64::try_from(canonical & u128::from(u64::MAX)).unwrap_or(0)
    } else {
        0
    }
}

#[cfg(test)]
mod tests;
