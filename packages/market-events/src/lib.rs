#![forbid(unsafe_code)]
//! Versioned, protocol-neutral commands and durable market facts.

use bunting_market_types::{
    CommandId, CorrelationId, EventId, EventSequence, InstrumentId, LogicalTimeNs, MoneyMinor,
    OrderId, ParticipantId, PriceTicks, QuantityLots, RunId,
};
use serde::{Deserialize, Serialize};

pub const EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Ord, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    Limit { price: PriceTicks },
    Market,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubmitOrder {
    pub order_id: OrderId,
    pub instrument_id: InstrumentId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub quantity: QuantityLots,
    pub kind: OrderKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelOrder {
    pub order_id: OrderId,
    pub participant_id: ParticipantId,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NbcDone {
    pub participant_id: ParticipantId,
    pub step: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandPayload {
    SubmitOrder(SubmitOrder),
    CancelOrder(CancelOrder),
    ActivateKillSwitch,
    NbcDone(NbcDone),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Command {
    pub run_id: RunId,
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub logical_time: LogicalTimeNs,
    pub expected_sequence: EventSequence,
    pub actor: ParticipantId,
    pub payload: CommandPayload,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectCode {
    DuplicateOrderId,
    InvalidOrderId,
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
    InsufficientLiquidity,
    LogicalTimeRegression,
    SequenceConflict,
    ArithmeticOverflow,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    Requested,
    KillSwitch,
    MarketRemainder,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
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
    OrderCompleted {
        order_id: OrderId,
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
        upstream_engine_sequence: u64,
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
    NbcParticipantDone {
        participant_id: ParticipantId,
        step: u32,
    },
    NbcStepAdvanced {
        executed_step: u32,
        current_step: u32,
        triggered_event_ids: Vec<String>,
        completed: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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
