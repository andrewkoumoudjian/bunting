#![forbid(unsafe_code)]
//! Versioned, protocol-neutral commands and durable market facts.

use bunting_market_types::{
    CommandId, CorrelationId, EventId, EventSequence, InstrumentId, LogicalTimeNs, MoneyMinor,
    OrderId, ParticipantId, PriceTicks, QuantityLots, RunId,
};

pub const EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderKind {
    Limit { price: PriceTicks },
    Market,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmitOrder {
    pub order_id: OrderId,
    pub instrument_id: InstrumentId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub quantity: QuantityLots,
    pub kind: OrderKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CancelOrder {
    pub order_id: OrderId,
    pub participant_id: ParticipantId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandPayload {
    SubmitOrder(SubmitOrder),
    CancelOrder(CancelOrder),
    ActivateKillSwitch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    pub run_id: RunId,
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub logical_time: LogicalTimeNs,
    pub expected_sequence: EventSequence,
    pub actor: ParticipantId,
    pub payload: CommandPayload,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectCode {
    DuplicateOrderId,
    UnknownOrder,
    NotOrderOwner,
    KillSwitchActive,
    ParticipantDisabled,
    InvalidQuantity,
    InvalidInstrument,
    PriceOutOfBounds,
    MaxOrderQuantity,
    MaxOpenOrderQuantity,
    PositionLimit,
    InsufficientCash,
    InsufficientInventory,
    LogicalTimeRegression,
    SequenceConflict,
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CancelReason {
    Requested,
    KillSwitch,
    MarketRemainder,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventPayload {
    OrderReceived {
        order: SubmitOrder,
    },
    OrderAccepted {
        order_id: OrderId,
    },
    OrderRejected {
        order_id: Option<OrderId>,
        code: RejectCode,
    },
    OrderRested {
        order_id: OrderId,
        participant_id: ParticipantId,
        instrument_id: InstrumentId,
        side: Side,
        price: PriceTicks,
        remaining: QuantityLots,
    },
    OrderReduced {
        order_id: OrderId,
        remaining: QuantityLots,
    },
    OrderCanceled {
        order_id: OrderId,
        participant_id: ParticipantId,
        instrument_id: InstrumentId,
        remaining: QuantityLots,
        reason: CancelReason,
    },
    TradeExecuted {
        instrument_id: InstrumentId,
        maker_order_id: OrderId,
        taker_order_id: OrderId,
        buyer_id: ParticipantId,
        seller_id: ParticipantId,
        price: PriceTicks,
        quantity: QuantityLots,
    },
    PositionChanged {
        participant_id: ParticipantId,
        instrument_id: InstrumentId,
        delta: QuantityLots,
    },
    BalanceChanged {
        participant_id: ParticipantId,
        delta: MoneyMinor,
    },
    KillSwitchActivated,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope {
    pub schema_version: u16,
    pub run_id: RunId,
    pub event_id: EventId,
    pub sequence: EventSequence,
    pub logical_time: LogicalTimeNs,
    pub actor: ParticipantId,
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub causation_sequence: Option<EventSequence>,
    pub payload: EventPayload,
}
