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

use crate::competition::account;
use crate::{ApplicationError, VerifiedActor};
use bnp_wire::{
    CancelReason as WireCancelReason, CashBalance, Liquidity, Listing, ListingInfo, NewOrder,
    OpenOrder, OrderType, Position, RejectReason, ServerMessage, Side as WireSide, Stamp,
    TimeInForce,
};
use bunting_engine::{OwnedOrderState, RunState};
use bunting_market_events::{
    CancelOrder, CancelReason, Command, CommandPayload, EventEnvelope, EventPayload, OrderKind,
    RejectCode, Side, SubmitOrder, TimeInForcePolicy,
};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, ListingKey, LogicalTimeNs, OrderId,
    ParticipantId, PriceTicks, QuantityLots, RunId, VenueId,
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
        }
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
        [self.new_orders, self.cancels, self.kill_switches]
            .into_iter()
            .any(|space| high == u128::from(space))
    }

    /// This participant's private messages for one committed batch, in
    /// event order. Events about other participants produce nothing.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per canonical event keeps the attribution rules side by side"
    )]
    pub fn private_messages(&self, events: &[EventEnvelope]) -> Vec<ServerMessage> {
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
                }),
                EventPayload::OrderReduced {
                    order_id,
                    remaining,
                } if mine(*order_id) => messages.push(ServerMessage::OrderReduced {
                    stamp,
                    client_order_id: client_order_id(*order_id),
                    order_id: order_id.get(),
                    remaining: remaining.get(),
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
        RejectCode::MaxOrderQuantity => RejectReason::MaxOrderQuantity,
        RejectCode::MaxOpenOrderQuantity => RejectReason::MaxOpenOrderQuantity,
        RejectCode::MaxLiveOrders => RejectReason::MaxLiveOrders,
        RejectCode::PositionLimit => RejectReason::PositionLimit,
        RejectCode::InsufficientCash => RejectReason::InsufficientCash,
        RejectCode::InsufficientInventory => RejectReason::InsufficientInventory,
        RejectCode::InsufficientLiquidity => RejectReason::InsufficientLiquidity,
        RejectCode::PostOnlyWouldCross => RejectReason::PostOnlyWouldCross,
        RejectCode::InvalidTimeInForce => RejectReason::InvalidTimeInForce,
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
