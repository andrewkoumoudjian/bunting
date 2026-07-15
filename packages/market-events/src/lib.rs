#![forbid(unsafe_code)]
//! Versioned, protocol-neutral commands and durable market facts.

use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, EventId, EventSequence, FacilityId, InstrumentId,
    LogicalTimeNs, MoneyMinor, NegotiationId, NewsId, OrderId, ParticipantId, PriceTicks,
    QuantityLots, RunId, TenderId,
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
    Limit {
        price: PriceTicks,
    },
    Market,
    LimitWithPolicy {
        price: PriceTicks,
        time_in_force: TimeInForcePolicy,
    },
    AdvancedLimit {
        price: PriceTicks,
        time_in_force: TimeInForcePolicy,
        policy: AdvancedOrderPolicy,
    },
}

/// Host-driven time-in-force policy mapped to OrderBook-rs.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeInForcePolicy {
    Gtc,
    Ioc,
    Fok,
    Gtd { expires_at_millis: u64 },
    Day,
}

/// Useful released OrderBook-rs special-order surface.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdvancedOrderPolicy {
    PostOnly,
    Iceberg {
        visible_quantity: QuantityLots,
    },
    Reserve {
        visible_quantity: QuantityLots,
        replenish_threshold: QuantityLots,
        replenish_quantity: QuantityLots,
        auto_replenish: bool,
    },
    Pegged {
        offset_ticks: i64,
        reference: PegReference,
    },
    TrailingStop {
        trail_ticks: QuantityLots,
    },
    MarketToLimit,
}

/// Upstream reference price used by pegged orders.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PegReference {
    BestBid,
    BestAsk,
    MidPrice,
    LastTrade,
}

impl OrderKind {
    /// Returns the explicit limit price when present.
    #[must_use]
    pub const fn limit_price(self) -> Option<PriceTicks> {
        match self {
            Self::Limit { price }
            | Self::LimitWithPolicy { price, .. }
            | Self::AdvancedLimit { price, .. } => Some(price),
            Self::Market => None,
        }
    }

    /// Returns whether the order is an unpriced market order.
    #[must_use]
    pub const fn is_market(self) -> bool {
        matches!(self, Self::Market)
    }
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

/// Deterministic logical-clock execution policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum ClockMode {
    Lockstep,
    Accelerated { max_steps_per_advance: u32 },
    Paced { step_interval_ns: u64 },
}

/// Audience for immutable or audited live news.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum NewsAudience {
    Public,
    Participant(ParticipantId),
    Team(u128),
    Role(String),
}

/// Tender-side participant decision.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TenderDecision {
    Accept,
    Decline,
}

/// OTC negotiation transition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OtcDecision {
    Accept,
    Reject,
    Break,
}

/// Atomicity policy for a bounded multi-leg command.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompositePolicy {
    BestEffort,
    MinimumFill,
    AllOrNone,
    AtomicConversion,
}

/// One exact leg in a composite command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompositeLeg {
    pub instrument_id: InstrumentId,
    pub side: Side,
    pub quantity: QuantityLots,
    pub limit_price: PriceTicks,
}

/// Simulation-domain commands sharing the authoritative run sequence.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationCommand {
    StartRun,
    PauseRun,
    ResumeRun,
    Advance {
        steps: u32,
    },
    SetPacing {
        mode: ClockMode,
        reason: String,
    },
    Terminate {
        reason: String,
    },
    SetListingHalt {
        instrument_id: InstrumentId,
        halted: bool,
        reason: String,
    },
    MassCancel {
        participant_id: Option<ParticipantId>,
        instrument_id: Option<InstrumentId>,
    },
    PublishNews {
        news_id: NewsId,
        audience: NewsAudience,
        headline: String,
        body: String,
    },
    OpenTender {
        tender_id: TenderId,
        participant_id: ParticipantId,
        instrument_id: InstrumentId,
        side: Side,
        quantity: QuantityLots,
        price: PriceTicks,
        expires_at: LogicalTimeNs,
    },
    DecideTender {
        tender_id: TenderId,
        decision: TenderDecision,
    },
    OpenOtc {
        negotiation_id: NegotiationId,
        counterparty_id: ParticipantId,
        instrument_id: InstrumentId,
        side: Side,
        quantity: QuantityLots,
        price: PriceTicks,
        expires_at: LogicalTimeNs,
    },
    CounterOtc {
        negotiation_id: NegotiationId,
        quantity: QuantityLots,
        price: PriceTicks,
    },
    DecideOtc {
        negotiation_id: NegotiationId,
        decision: OtcDecision,
    },
    SubmitComposite {
        policy: CompositePolicy,
        minimum_fill: QuantityLots,
        legs: Vec<CompositeLeg>,
    },
    ScheduleCashflow {
        participant_id: ParticipantId,
        currency_id: CurrencyId,
        amount: MoneyMinor,
        effective_at: LogicalTimeNs,
        reason: String,
    },
    ScheduleFacilityJob {
        facility_id: FacilityId,
        participant_id: ParticipantId,
        input_quantity: QuantityLots,
        output_quantity: QuantityLots,
        completes_at: LogicalTimeNs,
    },
    ScoreIteration,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandPayload {
    SubmitOrder(SubmitOrder),
    CancelOrder(CancelOrder),
    ActivateKillSwitch,
    NbcDone(NbcDone),
}

/// Administrator/simulation command envelope separate from participant order flow.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationCommandRequest {
    pub run_id: RunId,
    pub command_id: CommandId,
    pub correlation_id: CorrelationId,
    pub logical_time: LogicalTimeNs,
    pub expected_sequence: EventSequence,
    pub actor: ParticipantId,
    pub payload: SimulationCommand,
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
    RunNotActive,
    ListingHalted,
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
    MassCancel,
    Expired,
    Halt,
}

/// Durable simulation facts applied during replay.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimulationEvent {
    LifecycleChanged {
        status: String,
    },
    ClockAdvanced {
        from: LogicalTimeNs,
        to: LogicalTimeNs,
    },
    PacingChanged {
        mode: ClockMode,
        reason: String,
    },
    AdministratorChangeRecorded {
        reason: String,
    },
    ListingHaltChanged {
        instrument_id: InstrumentId,
        halted: bool,
    },
    MassCancelCompleted {
        canceled_orders: Vec<OrderId>,
    },
    NewsPublished {
        news_id: NewsId,
        audience: NewsAudience,
    },
    TenderChanged {
        tender_id: TenderId,
        status: String,
    },
    OtcChanged {
        negotiation_id: NegotiationId,
        status: String,
    },
    CompositeCompleted {
        policy: CompositePolicy,
        accepted_legs: u32,
    },
    CashflowScheduled {
        participant_id: ParticipantId,
        currency_id: CurrencyId,
        amount: MoneyMinor,
    },
    FacilityJobScheduled {
        facility_id: FacilityId,
        participant_id: ParticipantId,
    },
    ScheduledActionApplied {
        action_id: u128,
    },
    IterationScored {
        participant_count: u32,
    },
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
    Simulation(SimulationEvent),
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
