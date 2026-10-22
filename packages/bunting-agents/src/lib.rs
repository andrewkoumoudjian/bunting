#![forbid(unsafe_code)]
//! Bunting-native deterministic agent policies composed with mandatory QUARCC execution.

use bunting_market_events::{OrderKind, Side, TimeInForcePolicy};
use bunting_market_types::{InstrumentId, LogicalTimeNs, ParticipantId, PriceTicks, QuantityLots};
use quarcc_execution_engine::command::ExecutionIntent;
use quarcc_execution_engine::ids::{ClientOrderId, IntentId};
use quarcc_execution_engine::lifecycle::OrderLifecycle;
use quarcc_execution_engine::normalized_report::NormalizedVenueReport;
use quarcc_execution_engine::order::DesiredOrder;
use quarcc_execution_engine::{
    ExecutionAction, ExecutionActionBuffer, ExecutionEngine, ExecutionError, ExecutionSnapshot,
    QuarccExecutionEngine,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyKind {
    ZeroIntelligenceNoise,
    PoissonNoise,
    StaticLiquidityProvider,
    MultiLevelReplenisher,
    FundamentalLiquidityProvider,
    StressSensitiveLiquidity,
    FixedSpreadMarketMaker,
    InventorySkewedMarketMaker,
    EwmaVolatilityMarketMaker,
    BookImbalanceMarketMaker,
    OrderFlowImbalanceMarketMaker,
    AvellanedaStoikov,
    Glft,
    QueueAwareMarketMaker,
    FullyInformed,
    PartiallyInformed,
    DelayedInformation,
    LongMomentum,
    ShortMomentum,
    MeanReversion,
    Giveaway,
    Zic,
    Shaver,
    Zip,
    AdaptiveAggressive,
    Przi,
    Spiking,
    MultivariateHawkes,
    Microprice,
    Imbalance,
    QueueReactiveMarketMaker,
    SpreadCapture,
    OrderFlowMomentum,
    FastLiquidityWithdrawal,
    LogicalLatency,
    CrossVenueArbitrage,
    Twap,
    Vwap,
    Pov,
    ArrivalPrice,
    ImplementationShortfall,
    LiquiditySeeker,
    BlockExecution,
    TenderHedger,
    MultiVenueParentOrder,
    OptionsFlow,
    /// A strategy expressed as QUARCC signals (`StrategySignal`: side,
    /// target quantity, confidence) and executed by QUARCC's order
    /// management: each wake it forms a signal from the fundamental's gap
    /// to the mid, and its working order is cancelled or replaced until
    /// the position reaches the signal's target. Bunting-native semantics;
    /// only the contract's fields are taken from the C++ engine
    /// (`docs/ports/quarcc-trading-engine.md`).
    QuarccSignalTrader,
}

impl PolicyKind {
    fn is_market_maker(self) -> bool {
        matches!(
            self,
            Self::StaticLiquidityProvider
                | Self::MultiLevelReplenisher
                | Self::FundamentalLiquidityProvider
                | Self::StressSensitiveLiquidity
                | Self::FixedSpreadMarketMaker
                | Self::InventorySkewedMarketMaker
                | Self::EwmaVolatilityMarketMaker
                | Self::BookImbalanceMarketMaker
                | Self::OrderFlowImbalanceMarketMaker
                | Self::AvellanedaStoikov
                | Self::Glft
                | Self::QueueAwareMarketMaker
                | Self::QueueReactiveMarketMaker
                | Self::SpreadCapture
        )
    }

    fn is_institutional(self) -> bool {
        matches!(
            self,
            Self::Twap
                | Self::Vwap
                | Self::Pov
                | Self::ArrivalPrice
                | Self::ImplementationShortfall
                | Self::LiquiditySeeker
                | Self::BlockExecution
                | Self::TenderHedger
                | Self::MultiVenueParentOrder
        )
    }

    fn crosses_spread(self) -> bool {
        matches!(
            self,
            Self::LongMomentum
                | Self::ShortMomentum
                | Self::MeanReversion
                | Self::OrderFlowMomentum
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentConfig {
    pub kind: PolicyKind,
    pub participant_id: ParticipantId,
    pub instrument_id: InstrumentId,
    pub base_quantity: QuantityLots,
    pub spread_ticks: i64,
    pub inventory_target: QuantityLots,
    pub wake_interval_ns: u64,
    pub seed: u64,
    pub max_intents_per_wake: usize,
    /// Fundamental liquidity providers: every this many lots of inventory
    /// beyond `inventory_target` move both quotes one tick against the
    /// position, so a provider leans out of what it holds. None keeps the
    /// quotes centred on the fundamental.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_skew_lots: Option<i64>,
    /// Fundamental liquidity providers: past this inventory (either way)
    /// the side that would add to it is withdrawn until the position
    /// comes back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_limit: Option<QuantityLots>,
    /// Built-in agents that take liquidity: wake after exponentially
    /// distributed gaps with mean `wake_interval_ns` (a Poisson arrival
    /// process, the research note's null flow) instead of on a fixed beat.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub poisson_wakes: bool,
}

/// A QUARCC strategy signal turned into the one working order that moves
/// the position toward the signal's target, or `None` at the target.
///
/// - Signal: side is the sign of `fundamental − mid`; confidence (in
///   thousandths) is the gap over `spread_ticks`, capped at 1.
/// - Target position: `confidence × base_quantity` on that side (so
///   `base_quantity` is the most the strategy will hold).
/// - Order: the remaining distance to the target, joining the near touch
///   while confidence is below 0.7, taking the far touch above it.
#[must_use]
pub fn quarcc_signal(
    context: &AgentContext,
    observation: &AgentObservation,
    config: &AgentConfig,
) -> Option<(Side, QuantityLots, PriceTicks)> {
    let mid = observation
        .best_bid
        .get()
        .checked_add(observation.best_ask.get())?
        / 2;
    let gap = observation.fundamental.get().checked_sub(mid)?;
    let scale = config.spread_ticks.max(1);
    let confidence = gap.unsigned_abs().saturating_mul(1_000) / scale.unsigned_abs();
    let confidence = i64::try_from(confidence.min(1_000)).ok()?;
    let target = gap.signum() * config.base_quantity.get().saturating_mul(confidence) / 1_000;
    let delta = target.checked_sub(context.current_position.get())?;
    if delta == 0 {
        return None;
    }
    let side = if delta > 0 { Side::Buy } else { Side::Sell };
    let price = match (side, confidence >= 700) {
        (Side::Buy, true) | (Side::Sell, false) => observation.best_ask,
        (Side::Buy, false) | (Side::Sell, true) => observation.best_bid,
    };
    Some((side, QuantityLots::new(delta.abs()), price))
}

/// `-ln(u)` for `u = x / 2^32`, `x` in `1..2^32`, in millionths: an
/// exponential draw in integer arithmetic, so a replay on any host makes
/// the same draw. `u = m × 2^-k` with `m` in [0.5, 1): `-ln u = k ln 2 −
/// ln m`, and `−ln m` comes from a 16-step table interpolated linearly
/// (error below 0.001).
#[must_use]
pub fn exponential_millionths(x: u32) -> u64 {
    // -ln(0.5 + i/32) in millionths, i = 0..=16.
    const TABLE: [u64; 17] = [
        693_147, 632_523, 575_364, 521_297, 470_004, 421_213, 374_693, 330_242, 287_682, 246_860,
        207_639, 169_899, 133_531, 98_440, 64_539, 31_749, 0,
    ];
    const LN2: u64 = 693_147;
    let x = x.max(1);
    let k = x.leading_zeros();
    // `x` normalized into [2^31, 2^32): `m` of [0.5, 1) scaled by 2^32.
    let m = u64::from(x) << k;
    let offset = m - (1 << 31);
    // Sixteen steps of 1/32 of 2^32 across [0.5, 1).
    let step = 1_u64 << 27;
    let index = usize::try_from(offset / step).unwrap_or(15).min(15);
    let within = offset % step;
    let (high, low) = (TABLE[index], TABLE[index + 1]);
    let interpolated = high - (high - low) * within / step;
    u64::from(k) * LN2 + interpolated
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentModelMetadata {
    pub name: String,
    pub version: u16,
    pub provenance: String,
    pub exact_units: String,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentContext {
    pub logical_time: LogicalTimeNs,
    pub current_position: QuantityLots,
    pub remaining_parent_quantity: QuantityLots,
}

#[derive(Clone, Copy, Debug)]
pub struct AgentObservation {
    pub best_bid: PriceTicks,
    pub best_ask: PriceTicks,
    pub bid_quantity: QuantityLots,
    pub ask_quantity: QuantityLots,
    pub last_trade: PriceTicks,
    pub fundamental: PriceTicks,
    pub previous_trade: PriceTicks,
    pub observed_volume: QuantityLots,
    pub stress_bps: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NextWake {
    pub logical_time: LogicalTimeNs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentError {
    BufferFull,
    InvalidObservation,
    ArithmeticOverflow,
    Execution(ExecutionError),
}

impl From<ExecutionError> for AgentError {
    fn from(value: ExecutionError) -> Self {
        Self::Execution(value)
    }
}

pub struct IntentBuffer {
    limit: usize,
    intents: Vec<ExecutionIntent>,
}

impl IntentBuffer {
    #[must_use]
    pub fn with_limit(limit: usize) -> Self {
        Self {
            limit,
            intents: Vec::new(),
        }
    }

    /// Appends one policy intent without exceeding the configured bound.
    ///
    /// # Errors
    /// Returns [`AgentError::BufferFull`] when the buffer is full.
    pub fn push(&mut self, intent: ExecutionIntent) -> Result<(), AgentError> {
        if self.intents.len() >= self.limit {
            return Err(AgentError::BufferFull);
        }
        self.intents.push(intent);
        Ok(())
    }

    fn drain(&mut self) -> impl Iterator<Item = ExecutionIntent> + '_ {
        self.intents.drain(..)
    }
}

/// One order of the agent that is resting (or on its way to rest) at the
/// venue, as its execution state knows it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestingOrder {
    pub client_order_id: ClientOrderId,
    pub side: Side,
    pub price: PriceTicks,
    pub remaining: QuantityLots,
}

pub trait AgentPolicy {
    type Config;
    type Snapshot;

    /// True for a policy whose quotes rest until their level moves: its
    /// orders carry no expiry and `reconcile` diffs each wake's desired
    /// quotes against them.
    fn rests_quotes(&self) -> bool {
        false
    }

    /// Turns a wake's desired quotes into the changes that reach the
    /// venue: unchanged levels stay as they are (and keep their queue
    /// position), moved levels are cancelled, a missing or partly filled
    /// level is topped up. The default sends the intents as they are.
    ///
    /// # Errors
    /// Returns an error from the policy's own ID allocation.
    fn reconcile(
        &mut self,
        intents: Vec<ExecutionIntent>,
        _resting: &[RestingOrder],
    ) -> Result<Vec<ExecutionIntent>, AgentError> {
        Ok(intents)
    }

    fn metadata(&self) -> AgentModelMetadata;
    /// Evaluates one deterministic logical wake.
    ///
    /// # Errors
    /// Returns an error for invalid observations, arithmetic, or bounded output.
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError>;
    /// Applies a committed participant-private report.
    ///
    /// # Errors
    /// Returns an error for invalid policy state or bounded output.
    fn on_private_event(
        &mut self,
        event: &NormalizedVenueReport,
        output: &mut IntentBuffer,
    ) -> Result<(), AgentError>;
    fn snapshot(&self) -> Self::Snapshot;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicySnapshot {
    pub config: AgentConfig,
    pub next_id: u128,
    pub ewma_volatility_ticks: i64,
    pub hawkes_intensity: u64,
    pub rng_streams: BTreeMap<String, u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltInPolicy {
    state: PolicySnapshot,
}

impl BuiltInPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        let stream_name = format!("{:?}", config.kind);
        Self {
            state: PolicySnapshot {
                rng_streams: BTreeMap::from([(stream_name, config.seed.max(1))]),
                config,
                next_id: 1,
                ewma_volatility_ticks: 0,
                hawkes_intensity: 1,
            },
        }
    }

    #[must_use]
    pub const fn restore(snapshot: PolicySnapshot) -> Self {
        Self { state: snapshot }
    }

    fn next_random(&mut self) -> u64 {
        let name = format!("{:?}", self.state.config.kind);
        let state = self.state.rng_streams.entry(name).or_insert(1);
        let mut value = *state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        *state = value;
        value
    }

    fn next_ids(&mut self) -> Result<(IntentId, ClientOrderId), AgentError> {
        let id = self.state.next_id;
        self.state.next_id = id.checked_add(1).ok_or(AgentError::ArithmeticOverflow)?;
        Ok((IntentId::new(id), ClientOrderId::new(id)))
    }

    fn submit(
        &mut self,
        side: Side,
        quantity: QuantityLots,
        price: PriceTicks,
        output: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        if quantity.get() <= 0 || price.get() <= 0 {
            return Err(AgentError::InvalidObservation);
        }
        let (intent_id, client_order_id) = self.next_ids()?;
        output.push(ExecutionIntent::Submit {
            intent_id,
            order: DesiredOrder {
                client_order_id,
                instrument_id: self.state.config.instrument_id,
                participant_id: self.state.config.participant_id,
                side,
                quantity,
                kind: OrderKind::Limit { price },
                anonymous: false,
            },
        })
    }

    /// One taker's order for this wake: side and price from its model.
    fn take_liquidity(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        let random = self.next_random();
        let momentum = observation
            .last_trade
            .get()
            .saturating_sub(observation.previous_trade.get());
        let fundamental_gap = observation
            .fundamental
            .get()
            .saturating_sub(observation.last_trade.get());
        let side = match self.state.config.kind {
            PolicyKind::LongMomentum | PolicyKind::OrderFlowMomentum => {
                if momentum >= 0 {
                    Side::Buy
                } else {
                    Side::Sell
                }
            }
            PolicyKind::ShortMomentum | PolicyKind::MeanReversion => {
                if momentum >= 0 {
                    Side::Sell
                } else {
                    Side::Buy
                }
            }
            PolicyKind::FullyInformed
            | PolicyKind::PartiallyInformed
            | PolicyKind::DelayedInformation => {
                if fundamental_gap >= 0 {
                    Side::Buy
                } else {
                    Side::Sell
                }
            }
            PolicyKind::CrossVenueArbitrage | PolicyKind::Microprice | PolicyKind::Imbalance => {
                if observation.bid_quantity >= observation.ask_quantity {
                    Side::Buy
                } else {
                    Side::Sell
                }
            }
            _ => {
                if random & 1 == 0 {
                    Side::Buy
                } else {
                    Side::Sell
                }
            }
        };
        let quantity = if self.state.config.kind.is_institutional() {
            QuantityLots::new(
                context
                    .remaining_parent_quantity
                    .get()
                    .min(self.state.config.base_quantity.get())
                    .max(1),
            )
        } else {
            self.state.config.base_quantity
        };
        if self.state.config.kind == PolicyKind::MultivariateHawkes {
            self.state.hawkes_intensity = self
                .state
                .hawkes_intensity
                .saturating_mul(7)
                .saturating_div(8)
                .saturating_add(1 + (random % 3));
        }
        let displacement = i64::try_from(random % 3).unwrap_or(0);
        let price = match (self.state.config.kind.crosses_spread(), side) {
            (true, Side::Buy) => observation.best_ask.get(),
            (true, Side::Sell) => observation.best_bid.get(),
            (false, Side::Buy) => observation
                .best_bid
                .get()
                .saturating_sub(displacement)
                .max(1),
            (false, Side::Sell) => observation.best_ask.get().saturating_add(displacement),
        };
        self.submit(side, quantity, PriceTicks::new(price), output)?;
        Ok(())
    }

    /// The gap to the next wake: the configured interval, or with
    /// `poisson_wakes` an exponential draw of that mean, at least 1 ms and
    /// at most five means.
    fn wake_gap(&mut self) -> u64 {
        let mean = self.state.config.wake_interval_ns;
        if !self.state.config.poisson_wakes || self.state.config.kind.is_market_maker() {
            return mean;
        }
        let draw = u32::try_from(self.next_random() >> 32).unwrap_or(1);
        u64::try_from(
            (u128::from(mean) * u128::from(exponential_millionths(draw)) / 1_000_000)
                .clamp(1_000_000, u128::from(mean).saturating_mul(5).max(1_000_000)),
        )
        .unwrap_or(mean)
    }

    fn quote_both_sides(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        // A fundamental liquidity provider quotes around the observed
        // fundamental, which a run can move on a deterministic path. Every
        // other market maker quotes around the last trade, not the book's
        // mid: the book holds this agent's own quotes, and centering on
        // them would ratchet the price a tick each wake while inventory
        // skews the quotes. The book mid serves only before any trade.
        let follows_fundamental = self.state.config.kind
            == PolicyKind::FundamentalLiquidityProvider
            && observation.fundamental.get() > 0;
        let mid = if follows_fundamental {
            observation.fundamental.get()
        } else if observation.last_trade.get() > 0 {
            observation.last_trade.get()
        } else {
            observation
                .best_bid
                .get()
                .checked_add(observation.best_ask.get())
                .ok_or(AgentError::ArithmeticOverflow)?
                / 2
        };
        let inventory_skew = context
            .current_position
            .get()
            .checked_sub(self.state.config.inventory_target.get())
            .ok_or(AgentError::ArithmeticOverflow)?
            / self.state.config.base_quantity.get().max(1);
        let imbalance_denominator = observation
            .bid_quantity
            .get()
            .checked_add(observation.ask_quantity.get())
            .ok_or(AgentError::ArithmeticOverflow)?
            .max(1);
        let imbalance = observation
            .bid_quantity
            .get()
            .checked_sub(observation.ask_quantity.get())
            .ok_or(AgentError::ArithmeticOverflow)?
            / imbalance_denominator;
        let change = observation
            .last_trade
            .get()
            .saturating_sub(observation.previous_trade.get())
            .unsigned_abs();
        self.state.ewma_volatility_ticks = (self.state.ewma_volatility_ticks * 7
            + i64::try_from(change).map_err(|_| AgentError::ArithmeticOverflow)?)
            / 8;
        let model_adjustment = match self.state.config.kind {
            PolicyKind::InventorySkewedMarketMaker | PolicyKind::AvellanedaStoikov => {
                -inventory_skew
            }
            PolicyKind::EwmaVolatilityMarketMaker | PolicyKind::Glft => {
                self.state.ewma_volatility_ticks
            }
            PolicyKind::BookImbalanceMarketMaker | PolicyKind::OrderFlowImbalanceMarketMaker => {
                imbalance
            }
            PolicyKind::QueueAwareMarketMaker | PolicyKind::QueueReactiveMarketMaker => -imbalance,
            PolicyKind::StressSensitiveLiquidity if observation.stress_bps > 500 => return Ok(()),
            _ => 0,
        };
        let adjustment = i64::try_from(model_adjustment.unsigned_abs())
            .map_err(|_| AgentError::ArithmeticOverflow)?;
        let half_spread = self
            .state
            .config
            .spread_ticks
            .max(1)
            .saturating_add(adjustment);
        // A fundamental provider leans its quotes away from its inventory,
        // one tick per `inventory_skew_lots`, and past `inventory_limit`
        // stops quoting the side that would add to it.
        let excess = context
            .current_position
            .get()
            .saturating_sub(self.state.config.inventory_target.get());
        let lean = match self.state.config.inventory_skew_lots {
            Some(lots) if follows_fundamental && lots > 0 => excess / lots,
            _ => 0,
        };
        let limit = self
            .state
            .config
            .inventory_limit
            .filter(|_| follows_fundamental)
            .map(QuantityLots::get);
        let quote_bid = limit.is_none_or(|limit| excess < limit);
        let quote_ask = limit.is_none_or(|limit| excess > -limit);
        let center = mid.saturating_add(model_adjustment).saturating_sub(lean);
        if quote_bid {
            self.submit(
                Side::Buy,
                self.state.config.base_quantity,
                PriceTicks::new(center.saturating_sub(half_spread).max(1)),
                output,
            )?;
        }
        if quote_ask {
            self.submit(
                Side::Sell,
                self.state.config.base_quantity,
                PriceTicks::new(center.saturating_add(half_spread).max(1)),
                output,
            )?;
        }
        Ok(())
    }
}

impl AgentPolicy for BuiltInPolicy {
    type Config = AgentConfig;
    type Snapshot = PolicySnapshot;

    fn rests_quotes(&self) -> bool {
        matches!(
            self.state.config.kind,
            PolicyKind::FundamentalLiquidityProvider | PolicyKind::QuarccSignalTrader
        )
    }

    fn reconcile(
        &mut self,
        intents: Vec<ExecutionIntent>,
        resting: &[RestingOrder],
    ) -> Result<Vec<ExecutionIntent>, AgentError> {
        if !self.rests_quotes() {
            return Ok(intents);
        }
        let level = |order: &DesiredOrder| match order.kind {
            OrderKind::Limit { price } | OrderKind::LimitWithPolicy { price, .. } => {
                Some((order.side, price))
            }
            OrderKind::Market => None,
        };
        let mut wanted = Vec::new();
        let mut passthrough = Vec::new();
        for intent in intents {
            match &intent {
                ExecutionIntent::Submit { order, .. } if level(order).is_some() => {
                    wanted.push(intent);
                }
                _ => passthrough.push(intent),
            }
        }
        let wanted_levels: Vec<(Side, PriceTicks)> = wanted
            .iter()
            .filter_map(|intent| match intent {
                ExecutionIntent::Submit { order, .. } => level(order),
                _ => None,
            })
            .collect();
        let mut out = passthrough;
        for order in resting {
            if !wanted_levels.contains(&(order.side, order.price)) {
                let (intent_id, _) = self.next_ids()?;
                out.push(ExecutionIntent::Cancel {
                    intent_id,
                    client_order_id: order.client_order_id,
                });
            }
        }
        for intent in wanted {
            let ExecutionIntent::Submit {
                intent_id,
                mut order,
            } = intent
            else {
                continue;
            };
            let Some(at) = level(&order) else { continue };
            let held: i64 = resting
                .iter()
                .filter(|held| (held.side, held.price) == at)
                .map(|held| held.remaining.get())
                .sum();
            let missing = order.quantity.get().saturating_sub(held);
            if missing > 0 {
                order.quantity = QuantityLots::new(missing);
                out.push(ExecutionIntent::Submit { intent_id, order });
            }
        }
        Ok(out)
    }

    fn metadata(&self) -> AgentModelMetadata {
        AgentModelMetadata {
            name: format!("{:?}", self.state.config.kind),
            version: 1,
            provenance: "Bunting-native deterministic model; no NBC formula equivalence claimed"
                .to_owned(),
            exact_units: "PriceTicks/QuantityLots/LogicalTimeNs".to_owned(),
        }
    }

    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        if observation.best_bid.get() <= 0
            || observation.best_ask <= observation.best_bid
            || self.state.config.base_quantity.get() <= 0
        {
            return Err(AgentError::InvalidObservation);
        }
        if self.state.config.kind == PolicyKind::QuarccSignalTrader {
            if let Some((side, quantity, price)) =
                quarcc_signal(context, observation, &self.state.config)
            {
                self.submit(side, quantity, price, output)?;
            }
        } else if self.state.config.kind.is_market_maker() {
            self.quote_both_sides(context, observation, output)?;
        } else {
            self.take_liquidity(context, observation, output)?;
        }
        let next = context.logical_time.get().checked_add(self.wake_gap());
        Ok(NextWake {
            logical_time: LogicalTimeNs::new(next.ok_or(AgentError::ArithmeticOverflow)?),
        })
    }

    fn on_private_event(
        &mut self,
        _event: &NormalizedVenueReport,
        _output: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }

    fn snapshot(&self) -> Self::Snapshot {
        self.state.clone()
    }
}

/// Snapshot shared by individually implemented Bunting-native policy families.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct IndividualPolicySnapshot {
    pub config: AgentConfig,
    pub stream_name: String,
    pub rng_state: u64,
    pub next_id: u128,
    pub wake_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct IndividualPolicyCore {
    state: IndividualPolicySnapshot,
}

impl IndividualPolicyCore {
    fn new(config: AgentConfig, stream_name: &str) -> Self {
        let fold_id = |value: u128| {
            let Ok(low) = u64::try_from(value & u128::from(u64::MAX)) else {
                unreachable!("masked identifier half fits u64")
            };
            let Ok(high) = u64::try_from(value >> 64) else {
                unreachable!("shifted identifier half fits u64")
            };
            low ^ high
        };
        let domain_seed = config.seed
            ^ fold_id(config.participant_id.get()).rotate_left(17)
            ^ fold_id(config.instrument_id.get()).rotate_left(31)
            ^ stream_name.bytes().fold(0_u64, |hash, byte| {
                hash.wrapping_mul(1_099_511_628_211)
                    .wrapping_add(u64::from(byte))
            });
        Self {
            state: IndividualPolicySnapshot {
                config,
                stream_name: stream_name.to_owned(),
                rng_state: domain_seed.max(1),
                next_id: 1,
                wake_count: 0,
            },
        }
    }

    fn restore(snapshot: IndividualPolicySnapshot) -> Self {
        Self { state: snapshot }
    }

    fn draw(&mut self) -> u64 {
        let mut value = self.state.rng_state;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.state.rng_state = value;
        value
    }

    fn emit(
        &mut self,
        side: Side,
        quantity: QuantityLots,
        price: PriceTicks,
        output: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        if quantity.get() <= 0 || price.get() <= 0 {
            return Err(AgentError::InvalidObservation);
        }
        let id = self.state.next_id;
        self.state.next_id = id.checked_add(1).ok_or(AgentError::ArithmeticOverflow)?;
        output.push(ExecutionIntent::Submit {
            intent_id: IntentId::new(id),
            order: DesiredOrder {
                client_order_id: ClientOrderId::new(id),
                instrument_id: self.state.config.instrument_id,
                participant_id: self.state.config.participant_id,
                side,
                quantity,
                kind: OrderKind::Limit { price },
                anonymous: false,
            },
        })
    }

    fn next_wake(&mut self, context: &AgentContext) -> Result<NextWake, AgentError> {
        self.state.wake_count = self.state.wake_count.saturating_add(1);
        Ok(NextWake {
            logical_time: LogicalTimeNs::new(
                context
                    .logical_time
                    .get()
                    .checked_add(self.state.config.wake_interval_ns)
                    .ok_or(AgentError::ArithmeticOverflow)?,
            ),
        })
    }
}

fn validate_observation(observation: &AgentObservation) -> Result<(), AgentError> {
    if observation.best_bid.get() <= 0 || observation.best_ask <= observation.best_bid {
        Err(AgentError::InvalidObservation)
    } else {
        Ok(())
    }
}

fn individual_metadata(name: &str, units: &str) -> AgentModelMetadata {
    AgentModelMetadata {
        name: name.to_owned(),
        version: 1,
        provenance: "Bunting-native v1; NBC and RIT formulas remain unresolved".to_owned(),
        exact_units: units.to_owned(),
    }
}

/// Bounded zero-intelligence arrival, side, size, and displacement policy.
pub struct NoisePolicy(IndividualPolicyCore);

impl NoisePolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "noise.arrival-side-size-price.v1",
        ))
    }
    #[must_use]
    pub fn restore(snapshot: IndividualPolicySnapshot) -> Self {
        Self(IndividualPolicyCore::restore(snapshot))
    }
}

impl AgentPolicy for NoisePolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata("noise.v1", "ticks,lots,logical-nanoseconds,uniform-integer")
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        let draw = self.0.draw();
        let side = if draw & 1 == 0 { Side::Buy } else { Side::Sell };
        let max_quantity = self.0.state.config.base_quantity.get().max(1);
        let quantity = QuantityLots::new(
            1 + i64::try_from(
                (draw >> 1)
                    % u64::try_from(max_quantity).map_err(|_| AgentError::ArithmeticOverflow)?,
            )
            .map_err(|_| AgentError::ArithmeticOverflow)?,
        );
        let width = u64::try_from(self.0.state.config.spread_ticks.max(1))
            .map_err(|_| AgentError::ArithmeticOverflow)?;
        let displacement =
            i64::try_from((draw >> 17) % width).map_err(|_| AgentError::ArithmeticOverflow)?;
        let price = match side {
            Side::Buy => observation
                .best_bid
                .get()
                .saturating_sub(displacement)
                .max(1),
            Side::Sell => observation.best_ask.get().saturating_add(displacement),
        };
        self.0
            .emit(side, quantity, PriceTicks::new(price), output)?;
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

/// Inventory-aware two-sided replenishment policy.
pub struct LiquidityPolicy(IndividualPolicyCore);

impl LiquidityPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "liquidity.level-spread-replenishment.v1",
        ))
    }
}

impl AgentPolicy for LiquidityPolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata(
            "liquidity.v1",
            "ticks,lots,inventory-lots,logical-nanoseconds",
        )
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        if observation.stress_bps <= 7_500 {
            let skew = context
                .current_position
                .get()
                .saturating_sub(self.0.state.config.inventory_target.get())
                .signum();
            let half = self.0.state.config.spread_ticks.max(1);
            let mid = observation
                .best_bid
                .get()
                .saturating_add(observation.best_ask.get())
                / 2;
            self.0.emit(
                Side::Buy,
                self.0.state.config.base_quantity,
                PriceTicks::new(mid.saturating_sub(half).saturating_sub(skew).max(1)),
                output,
            )?;
            self.0.emit(
                Side::Sell,
                self.0.state.config.base_quantity,
                PriceTicks::new(mid.saturating_add(half).saturating_sub(skew).max(1)),
                output,
            )?;
        }
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

/// Fundamental-gap policy with a versioned exact information coefficient.
pub struct InformedPolicy(IndividualPolicyCore);

impl InformedPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "informed.fundamental-gap.v1",
        ))
    }
}

impl AgentPolicy for InformedPolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata("informed.v1", "ticks,lots,information-coefficient-bps=7500")
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        let gap = observation
            .fundamental
            .get()
            .saturating_sub(observation.last_trade.get());
        let perceived = observation
            .last_trade
            .get()
            .saturating_add(gap.saturating_mul(7_500) / 10_000);
        let side = if gap >= 0 { Side::Buy } else { Side::Sell };
        self.0.emit(
            side,
            self.0.state.config.base_quantity,
            PriceTicks::new(perceived.max(1)),
            output,
        )?;
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

/// Signed price-change momentum policy.
pub struct MomentumPolicy(IndividualPolicyCore);

impl MomentumPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "momentum.price-change.v1",
        ))
    }
}

impl AgentPolicy for MomentumPolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata("momentum.v1", "ticks,lots,one-observation-return")
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        let side = if observation.last_trade >= observation.previous_trade {
            Side::Buy
        } else {
            Side::Sell
        };
        let price = if side == Side::Buy {
            observation.best_ask
        } else {
            observation.best_bid
        };
        self.0
            .emit(side, self.0.state.config.base_quantity, price, output)?;
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

/// Rare bounded directional spike policy.
pub struct SpikingPolicy(IndividualPolicyCore);

impl SpikingPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "spiking.activation-volume.v1",
        ))
    }
}

impl AgentPolicy for SpikingPolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata("spiking.v1", "activation-bps=500,ticks,lots")
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        let draw = self.0.draw();
        if draw % 10_000 < 500 {
            let side = if (draw >> 16) & 1 == 0 {
                Side::Buy
            } else {
                Side::Sell
            };
            let price = if side == Side::Buy {
                observation.best_ask
            } else {
                observation.best_bid
            };
            self.0.emit(
                side,
                QuantityLots::new(self.0.state.config.base_quantity.get().saturating_mul(4)),
                price,
                output,
            )?;
        }
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

/// Parent-order participation policy with exact bounded slices.
pub struct InstitutionalPolicy(IndividualPolicyCore);

impl InstitutionalPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "institutional.parent-slice.v1",
        ))
    }
}

impl AgentPolicy for InstitutionalPolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata(
            "institutional.v1",
            "lots,observed-volume-participation-bps=1000",
        )
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        let volume_slice = (observation.observed_volume.get() / 10).max(1);
        let quantity = QuantityLots::new(
            context
                .remaining_parent_quantity
                .get()
                .min(self.0.state.config.base_quantity.get())
                .min(volume_slice)
                .max(1),
        );
        let side = if context.remaining_parent_quantity.get() >= 0 {
            Side::Buy
        } else {
            Side::Sell
        };
        let price = if side == Side::Buy {
            observation.best_ask
        } else {
            observation.best_bid
        };
        self.0.emit(side, quantity, price, output)?;
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

/// Options-flow pressure policy expressed in exact observable units.
pub struct OptionsFlowPolicy(IndividualPolicyCore);

impl OptionsFlowPolicy {
    #[must_use]
    pub fn new(config: AgentConfig) -> Self {
        Self(IndividualPolicyCore::new(
            config,
            "options-flow.pressure.v1",
        ))
    }
}

impl AgentPolicy for OptionsFlowPolicy {
    type Config = AgentConfig;
    type Snapshot = IndividualPolicySnapshot;
    fn metadata(&self) -> AgentModelMetadata {
        individual_metadata(
            "options-flow.v1",
            "depth-lots,fundamental-gap-ticks,option-lots",
        )
    }
    fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<NextWake, AgentError> {
        validate_observation(observation)?;
        let pressure = observation
            .bid_quantity
            .get()
            .saturating_sub(observation.ask_quantity.get())
            .saturating_add(
                observation
                    .fundamental
                    .get()
                    .saturating_sub(observation.last_trade.get()),
            );
        let side = if pressure >= 0 { Side::Buy } else { Side::Sell };
        let price = if side == Side::Buy {
            observation.best_ask
        } else {
            observation.best_bid
        };
        self.0
            .emit(side, self.0.state.config.base_quantity, price, output)?;
        self.0.next_wake(context)
    }
    fn on_private_event(
        &mut self,
        _: &NormalizedVenueReport,
        _: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        Ok(())
    }
    fn snapshot(&self) -> Self::Snapshot {
        self.0.state.clone()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedAgentSnapshot<S> {
    pub policy: S,
    pub execution: ExecutionSnapshot,
}

/// How many of its wake intervals a built-in agent's limit order rests
/// before the venue expires it (GTD). Two keep a quoting agent's book
/// covered between wakes; agents never pile up stale orders.
pub const ORDER_LIFETIME_WAKES: u64 = 2;

/// The GTD expiry of an order decided at `now` by an agent next waking at
/// `next`: `ORDER_LIFETIME_WAKES` intervals on, and always after `now`.
fn order_expiry(now: LogicalTimeNs, next: LogicalTimeNs) -> Result<LogicalTimeNs, AgentError> {
    let interval = next.get().saturating_sub(now.get()).max(1);
    interval
        .checked_mul(ORDER_LIFETIME_WAKES)
        .and_then(|lifetime| now.get().checked_add(lifetime))
        .map(LogicalTimeNs::new)
        .ok_or(AgentError::ArithmeticOverflow)
}

pub struct ManagedAgent<P: AgentPolicy> {
    policy: P,
    execution: QuarccExecutionEngine,
}

impl<P: AgentPolicy> ManagedAgent<P> {
    #[must_use]
    pub const fn new(policy: P, execution: QuarccExecutionEngine) -> Self {
        Self { policy, execution }
    }

    /// Runs the policy and routes every emitted intent through mandatory QUARCC state.
    ///
    /// # Errors
    /// Returns an error from policy evaluation, execution state, or bounded output.
    pub fn on_wake(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
    ) -> Result<(NextWake, Vec<ExecutionAction>), AgentError> {
        let limit = self.execution.snapshot().config.max_actions_per_call;
        let mut intents = IntentBuffer::with_limit(limit);
        let next = self.policy.on_wake(context, observation, &mut intents)?;
        let expires_at = order_expiry(context.logical_time, next.logical_time)?;
        let rests = self.policy.rests_quotes();
        let drained: Vec<ExecutionIntent> = intents.drain().collect();
        let drained = if rests {
            let resting: Vec<RestingOrder> = self
                .execution
                .snapshot()
                .orders
                .values()
                .filter(|managed| {
                    matches!(
                        managed.lifecycle,
                        OrderLifecycle::PendingSubmit
                            | OrderLifecycle::Live
                            | OrderLifecycle::PartiallyFilled
                    )
                })
                .filter_map(|managed| match managed.desired.kind {
                    OrderKind::Limit { price } | OrderKind::LimitWithPolicy { price, .. } => {
                        Some(RestingOrder {
                            client_order_id: managed.desired.client_order_id,
                            side: managed.desired.side,
                            price,
                            remaining: managed.remaining_quantity(),
                        })
                    }
                    OrderKind::Market => None,
                })
                .collect();
            self.policy.reconcile(drained, &resting)?
        } else {
            drained
        };
        let mut actions = ExecutionActionBuffer::with_limit(limit);
        for mut intent in drained {
            if !rests
                && let ExecutionIntent::Submit { order, .. } = &mut intent
                && let OrderKind::Limit { price } = order.kind
            {
                order.kind = OrderKind::LimitWithPolicy {
                    price,
                    time_in_force: TimeInForcePolicy::Gtd { expires_at },
                    post_only: false,
                    display_quantity: None,
                };
            }
            self.route(intent, &mut actions)?;
        }
        Ok((next, actions.into_vec()))
    }

    /// Sends one intent through QUARCC. An intent its pre-trade checks
    /// refuse (open-order or position limit, kill switch) is dropped, as a
    /// trader's own risk system drops it: nothing reaches the venue and the
    /// agent carries on at its next wake. Any other failure is a bug.
    fn route(
        &mut self,
        intent: ExecutionIntent,
        actions: &mut ExecutionActionBuffer,
    ) -> Result<(), AgentError> {
        match self.execution.submit_intent(intent, actions) {
            Err(error) if error.is_pre_trade_refusal() => Ok(()),
            result => result.map_err(AgentError::from),
        }
    }

    /// Applies a private report to QUARCC before allowing the policy to react.
    ///
    /// # Errors
    /// Returns an error from report reconciliation, policy evaluation, or bounded output.
    pub fn on_private_event(
        &mut self,
        event: &NormalizedVenueReport,
    ) -> Result<Vec<ExecutionAction>, AgentError> {
        let limit = self.execution.snapshot().config.max_actions_per_call;
        let mut actions = ExecutionActionBuffer::with_limit(limit);
        self.execution.apply_venue_report(event, &mut actions)?;
        let mut intents = IntentBuffer::with_limit(limit);
        self.policy.on_private_event(event, &mut intents)?;
        for intent in intents.drain() {
            self.route(intent, &mut actions)?;
        }
        Ok(actions.into_vec())
    }

    #[must_use]
    pub fn snapshot(&self) -> ManagedAgentSnapshot<P::Snapshot> {
        ManagedAgentSnapshot {
            policy: self.policy.snapshot(),
            execution: self.execution.snapshot(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quarcc_execution_engine::ExecutionConfig;

    fn config(kind: PolicyKind) -> AgentConfig {
        AgentConfig {
            inventory_skew_lots: None,
            inventory_limit: None,
            poisson_wakes: false,
            kind,
            participant_id: ParticipantId::new(1),
            instrument_id: InstrumentId::new(2),
            base_quantity: QuantityLots::new(10),
            spread_ticks: 2,
            inventory_target: QuantityLots::new(0),
            wake_interval_ns: 1_000,
            seed: 42,
            max_intents_per_wake: 4,
        }
    }

    fn observation() -> AgentObservation {
        AgentObservation {
            best_bid: PriceTicks::new(99),
            best_ask: PriceTicks::new(101),
            bid_quantity: QuantityLots::new(100),
            ask_quantity: QuantityLots::new(80),
            last_trade: PriceTicks::new(100),
            fundamental: PriceTicks::new(102),
            previous_trade: PriceTicks::new(99),
            observed_volume: QuantityLots::new(1_000),
            stress_bps: 0,
        }
    }

    #[test]
    fn market_maker_always_routes_two_intents_through_quarcc() -> Result<(), AgentError> {
        let policy = BuiltInPolicy::new(config(PolicyKind::AvellanedaStoikov));
        let mut agent = ManagedAgent::new(
            policy,
            QuarccExecutionEngine::new(ExecutionConfig::default()),
        );
        let (_, actions) = agent.on_wake(
            &AgentContext {
                logical_time: LogicalTimeNs::new(0),
                current_position: QuantityLots::new(0),
                remaining_parent_quantity: QuantityLots::new(0),
            },
            &observation(),
        )?;
        assert_eq!(actions.len(), 2);
        assert!(
            actions
                .iter()
                .all(|action| matches!(action, ExecutionAction::Submit { .. }))
        );
        Ok(())
    }

    #[test]
    fn liquidity_provider_quotes_follow_the_moving_fundamental() -> Result<(), AgentError> {
        let quoted = |kind: PolicyKind, fundamental: i64| -> Result<Vec<i64>, AgentError> {
            let mut agent = ManagedAgent::new(
                BuiltInPolicy::new(config(kind)),
                QuarccExecutionEngine::new(ExecutionConfig::default()),
            );
            let (_, actions) = agent.on_wake(
                &AgentContext {
                    logical_time: LogicalTimeNs::new(0),
                    current_position: QuantityLots::new(0),
                    remaining_parent_quantity: QuantityLots::new(0),
                },
                &AgentObservation {
                    fundamental: PriceTicks::new(fundamental),
                    ..observation()
                },
            )?;
            let mut prices = Vec::new();
            for action in actions {
                let ExecutionAction::Submit { order, .. } = action else {
                    return Err(AgentError::InvalidObservation);
                };
                match order.kind {
                    OrderKind::Limit { price } | OrderKind::LimitWithPolicy { price, .. } => {
                        prices.push(price.get());
                    }
                    OrderKind::Market => return Err(AgentError::InvalidObservation),
                }
            }
            prices.sort_unstable();
            Ok(prices)
        };
        let follower = PolicyKind::FundamentalLiquidityProvider;
        let spread = config(follower).spread_ticks;
        // Last trade stays at 100: only the fundamental moves the quotes.
        assert_eq!(quoted(follower, 100)?, [100 - spread, 100 + spread]);
        assert_eq!(quoted(follower, 112)?, [112 - spread, 112 + spread]);
        assert_eq!(quoted(follower, 90)?, [90 - spread, 90 + spread]);
        // The static provider keeps quoting around the last trade.
        assert_eq!(
            quoted(PolicyKind::StaticLiquidityProvider, 112)?,
            [100 - spread, 100 + spread]
        );
        Ok(())
    }

    fn provider_wake(
        agent: &mut ManagedAgent<BuiltInPolicy>,
        at: u64,
        fundamental: i64,
    ) -> Result<Vec<ExecutionAction>, AgentError> {
        let (_, actions) = agent.on_wake(
            &AgentContext {
                logical_time: LogicalTimeNs::new(at),
                current_position: QuantityLots::new(0),
                remaining_parent_quantity: QuantityLots::new(0),
            },
            &AgentObservation {
                fundamental: PriceTicks::new(fundamental),
                ..observation()
            },
        )?;
        Ok(actions)
    }

    #[test]
    fn a_provider_whose_fundamental_did_not_move_leaves_its_quotes_alone() -> Result<(), AgentError>
    {
        let mut agent = ManagedAgent::new(
            BuiltInPolicy::new(config(PolicyKind::FundamentalLiquidityProvider)),
            QuarccExecutionEngine::new(ExecutionConfig::default()),
        );
        let first = provider_wake(&mut agent, 0, 100)?;
        assert_eq!(first.len(), 2, "a bid and an offer");
        // Same fundamental, many wakes later, orders never expire: nothing.
        for wake in 1..=5 {
            assert!(provider_wake(&mut agent, wake * 500_000_000, 100)?.is_empty());
        }
        // The fundamental moved a tick: both levels move, nothing else.
        let moved = provider_wake(&mut agent, 3_000_000_000, 101)?;
        let cancels = moved
            .iter()
            .filter(|action| matches!(action, ExecutionAction::Cancel { .. }))
            .count();
        let submits = moved
            .iter()
            .filter(|action| matches!(action, ExecutionAction::Submit { .. }))
            .count();
        assert_eq!((cancels, submits), (2, 2));
        Ok(())
    }

    #[test]
    fn a_partly_filled_level_is_topped_up_not_replaced() -> Result<(), AgentError> {
        let mut agent = ManagedAgent::new(
            BuiltInPolicy::new(config(PolicyKind::FundamentalLiquidityProvider)),
            QuarccExecutionEngine::new(ExecutionConfig::default()),
        );
        let first = provider_wake(&mut agent, 0, 100)?;
        let (local, quantity) = first
            .iter()
            .find_map(|action| match action {
                ExecutionAction::Submit {
                    local_order_id,
                    order,
                    ..
                } if order.side == Side::Buy => Some((*local_order_id, order.quantity.get())),
                _ => None,
            })
            .ok_or(AgentError::InvalidObservation)?;
        let filled = 3;
        agent.on_private_event(&NormalizedVenueReport {
            report_id: quarcc_execution_engine::ids::ReportId::new(1),
            source_sequence: Some(1),
            client_order_id: None,
            local_order_id: Some(local),
            venue_order_id: None,
            leaves_quantity: Some(QuantityLots::new(quantity - filled)),
            kind: quarcc_execution_engine::normalized_report::VenueReportKind::Fill {
                last_quantity: QuantityLots::new(filled),
                cumulative_quantity: QuantityLots::new(filled),
                price: PriceTicks::new(98),
            },
        })?;
        let next = provider_wake(&mut agent, 500_000_000, 100)?;
        // One new bid for what was filled; the rest keeps resting.
        assert_eq!(next.len(), 1);
        let ExecutionAction::Submit { order, .. } = &next[0] else {
            return Err(AgentError::InvalidObservation);
        };
        assert_eq!((order.side, order.quantity.get()), (Side::Buy, filled));
        Ok(())
    }

    #[test]
    fn an_agent_at_its_order_limit_skips_the_order_and_carries_on() -> Result<(), AgentError> {
        // Room for three open orders: the market maker's second wake fits
        // one of its two quotes, its third fits none. Nothing fails.
        let mut agent = ManagedAgent::new(
            BuiltInPolicy::new(config(PolicyKind::StaticLiquidityProvider)),
            QuarccExecutionEngine::new(ExecutionConfig {
                max_orders: 3,
                ..ExecutionConfig::default()
            }),
        );
        let mut sent = Vec::new();
        let mut now = LogicalTimeNs::new(0);
        for _ in 0..3 {
            let (next, actions) = agent.on_wake(
                &AgentContext {
                    logical_time: now,
                    current_position: QuantityLots::new(0),
                    remaining_parent_quantity: QuantityLots::new(0),
                },
                &observation(),
            )?;
            sent.push(actions.len());
            // Each order rests two wakes, then the venue expires it.
            for action in &actions {
                let ExecutionAction::Submit { order, .. } = action else {
                    return Err(AgentError::InvalidObservation);
                };
                let expiry = LogicalTimeNs::new(now.get() + 2_000);
                assert!(matches!(
                    order.kind,
                    OrderKind::LimitWithPolicy {
                        time_in_force: TimeInForcePolicy::Gtd { expires_at },
                        post_only: false,
                        display_quantity: None,
                        ..
                    } if expires_at == expiry
                ));
            }
            now = next.logical_time;
        }
        assert_eq!(sent, [2, 1, 0]);
        Ok(())
    }

    #[test]
    fn named_rng_and_policy_snapshot_are_deterministic() -> Result<(), AgentError> {
        let mut first = BuiltInPolicy::new(config(PolicyKind::ZeroIntelligenceNoise));
        let mut second = BuiltInPolicy::new(config(PolicyKind::ZeroIntelligenceNoise));
        let context = AgentContext {
            logical_time: LogicalTimeNs::new(0),
            current_position: QuantityLots::new(0),
            remaining_parent_quantity: QuantityLots::new(0),
        };
        let mut left = IntentBuffer::with_limit(4);
        let mut right = IntentBuffer::with_limit(4);
        first.on_wake(&context, &observation(), &mut left)?;
        second.on_wake(&context, &observation(), &mut right)?;
        assert_eq!(left.intents, right.intents);
        assert_eq!(first.snapshot(), second.snapshot());
        Ok(())
    }

    #[test]
    fn built_in_momentum_crosses_the_observed_spread() -> Result<(), AgentError> {
        let mut policy = BuiltInPolicy::new(config(PolicyKind::LongMomentum));
        let context = AgentContext {
            logical_time: LogicalTimeNs::new(10),
            current_position: QuantityLots::new(0),
            remaining_parent_quantity: QuantityLots::new(20),
        };
        let observation = observation();
        let mut output = IntentBuffer::with_limit(1);
        policy.on_wake(&context, &observation, &mut output)?;
        assert!(matches!(
            output.intents.as_slice(),
            [ExecutionIntent::Submit { order, .. }]
                if order.side == Side::Buy
                    && order.kind == OrderKind::Limit { price: observation.best_ask }
        ));
        Ok(())
    }

    #[test]
    fn individual_policy_golden_vectors_are_stable() -> Result<(), AgentError> {
        fn side(intent: &ExecutionIntent) -> Option<Side> {
            match intent {
                ExecutionIntent::Submit { order, .. } => Some(order.side),
                _ => None,
            }
        }
        let context = AgentContext {
            logical_time: LogicalTimeNs::new(10),
            current_position: QuantityLots::new(0),
            remaining_parent_quantity: QuantityLots::new(20),
        };
        let observation = observation();
        let mut vectors = Vec::new();
        let mut collect = |policy: &mut dyn AgentPolicy<
            Config = AgentConfig,
            Snapshot = IndividualPolicySnapshot,
        >|
         -> Result<(), AgentError> {
            let mut output = IntentBuffer::with_limit(4);
            policy.on_wake(&context, &observation, &mut output)?;
            vectors.push(output.intents.clone());
            Ok(())
        };
        collect(&mut NoisePolicy::new(config(
            PolicyKind::ZeroIntelligenceNoise,
        )))?;
        collect(&mut LiquidityPolicy::new(config(
            PolicyKind::StaticLiquidityProvider,
        )))?;
        collect(&mut InformedPolicy::new(config(PolicyKind::FullyInformed)))?;
        collect(&mut MomentumPolicy::new(config(PolicyKind::LongMomentum)))?;
        collect(&mut SpikingPolicy::new(config(PolicyKind::Spiking)))?;
        collect(&mut InstitutionalPolicy::new(config(PolicyKind::Twap)))?;
        collect(&mut OptionsFlowPolicy::new(config(PolicyKind::OptionsFlow)))?;
        assert_eq!(
            vectors.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![1, 2, 1, 1, 1, 1, 1]
        );
        assert_eq!(side(&vectors[2][0]), Some(Side::Buy));
        assert_eq!(side(&vectors[3][0]), Some(Side::Buy));
        assert_eq!(side(&vectors[6][0]), Some(Side::Buy));
        Ok(())
    }

    #[test]
    fn noise_distribution_is_bounded_and_snapshot_replays() -> Result<(), AgentError> {
        fn is_buy(intent: &ExecutionIntent) -> bool {
            matches!(intent, ExecutionIntent::Submit { order, .. } if order.side == Side::Buy)
        }
        let mut policy = NoisePolicy::new(config(PolicyKind::ZeroIntelligenceNoise));
        let observation = observation();
        let mut context = AgentContext {
            logical_time: LogicalTimeNs::new(0),
            current_position: QuantityLots::new(0),
            remaining_parent_quantity: QuantityLots::new(0),
        };
        let mut buys = 0_u32;
        for _ in 0..1_024 {
            let mut output = IntentBuffer::with_limit(1);
            let next = policy.on_wake(&context, &observation, &mut output)?;
            if is_buy(&output.intents[0]) {
                buys += 1;
            }
            context.logical_time = next.logical_time;
        }
        assert!((400..=624).contains(&buys));
        let snapshot = policy.snapshot();
        let mut restored = NoisePolicy::restore(snapshot.clone());
        let mut left = IntentBuffer::with_limit(1);
        let mut right = IntentBuffer::with_limit(1);
        policy.on_wake(&context, &observation, &mut left)?;
        restored.on_wake(&context, &observation, &mut right)?;
        assert_eq!(left.intents, right.intents);
        assert_eq!(restored.snapshot().stream_name, snapshot.stream_name);
        Ok(())
    }

    fn at(logical_time: u64) -> AgentContext {
        AgentContext {
            logical_time: LogicalTimeNs::new(logical_time),
            current_position: QuantityLots::new(0),
            remaining_parent_quantity: QuantityLots::new(0),
        }
    }

    #[test]
    fn exponential_draws_match_minus_ln_u() {
        // u = 1/2, 1/4 and 1/8 of 2^32: -ln u = 0.693147, 1.386294, 2.079442.
        assert_eq!(exponential_millionths(1 << 31), 693_147);
        assert_eq!(exponential_millionths(1 << 30), 1_386_294);
        assert_eq!(exponential_millionths(1 << 29), 2_079_441);
        assert!(exponential_millionths(u32::MAX) <= 1, "u near 1");
        // -ln(0.75) = 0.287682.
        assert_eq!(exponential_millionths(3 << 30), 287_682);
        // Between table points the error stays under 0.001.
        // u = 3e9 / 2^32 = 0.698492: -ln u = 0.358851.
        assert!(exponential_millionths(3_000_000_000).abs_diff(358_851) < 1_000);
    }

    #[test]
    fn poisson_wakes_vary_around_the_mean_and_stay_bounded() -> Result<(), AgentError> {
        let mut agent = BuiltInPolicy::new(AgentConfig {
            poisson_wakes: true,
            wake_interval_ns: 1_000_000_000,
            ..config(PolicyKind::ZeroIntelligenceNoise)
        });
        let mut gaps = Vec::new();
        for _ in 0..400 {
            let mut output = IntentBuffer::with_limit(8);
            let woken = agent.on_wake(&at(0), &observation(), &mut output)?;
            gaps.push(woken.logical_time.get());
        }
        let mean = agent.state.config.wake_interval_ns;
        assert!(gaps.iter().all(|gap| (1_000_000..=mean * 5).contains(gap)));
        let average = gaps.iter().sum::<u64>() / 400;
        assert!(
            average > mean * 7 / 10 && average < mean * 13 / 10,
            "{average}"
        );
        assert!(
            gaps.windows(2).any(|pair| pair[0] != pair[1]),
            "not a fixed beat"
        );
        Ok(())
    }

    #[test]
    fn a_fundamental_provider_leans_from_inventory_and_withdraws_past_its_limit()
    -> Result<(), AgentError> {
        let provider = |position: i64| -> Result<Vec<(Side, i64)>, AgentError> {
            let mut agent = BuiltInPolicy::new(AgentConfig {
                inventory_skew_lots: Some(10),
                inventory_limit: Some(QuantityLots::new(50)),
                ..config(PolicyKind::FundamentalLiquidityProvider)
            });
            let mut output = IntentBuffer::with_limit(8);
            let mut held = at(0);
            held.current_position = QuantityLots::new(position);
            let mut seen = observation();
            seen.fundamental = PriceTicks::new(100);
            agent.on_wake(&held, &seen, &mut output)?;
            Ok(output
                .drain()
                .filter_map(|intent| match intent {
                    ExecutionIntent::Submit { order, .. } => match order.kind {
                        OrderKind::Limit { price } => Some((order.side, price.get())),
                        _ => None,
                    },
                    _ => None,
                })
                .collect())
        };
        let flat = provider(0)?;
        let long = provider(30)?;
        assert_eq!(long.len(), 2);
        for ((side, flat_price), (_, long_price)) in flat.iter().zip(&long) {
            assert_eq!(flat_price - long_price, 3, "{side:?} leans 3 ticks lower");
        }
        // Long past the limit: only the offer remains.
        let capped = provider(60)?;
        assert_eq!(
            capped.iter().map(|(side, _)| *side).collect::<Vec<_>>(),
            vec![Side::Sell]
        );
        let short = provider(-60)?;
        assert_eq!(
            short.iter().map(|(side, _)| *side).collect::<Vec<_>>(),
            vec![Side::Buy]
        );
        Ok(())
    }
}

#[cfg(test)]
mod quarcc_signal_tests {
    use super::*;

    fn config() -> AgentConfig {
        AgentConfig {
            kind: PolicyKind::QuarccSignalTrader,
            participant_id: ParticipantId::new(44),
            instrument_id: InstrumentId::new(1),
            base_quantity: QuantityLots::new(500),
            spread_ticks: 20,
            inventory_target: QuantityLots::new(0),
            wake_interval_ns: 1_000_000_000,
            seed: 1,
            max_intents_per_wake: 4,
            inventory_skew_lots: None,
            inventory_limit: None,
            poisson_wakes: false,
        }
    }

    fn seen(fundamental: i64) -> AgentObservation {
        AgentObservation {
            best_bid: PriceTicks::new(9_999),
            best_ask: PriceTicks::new(10_001),
            bid_quantity: QuantityLots::new(100),
            ask_quantity: QuantityLots::new(100),
            last_trade: PriceTicks::new(10_000),
            fundamental: PriceTicks::new(fundamental),
            previous_trade: PriceTicks::new(10_000),
            observed_volume: QuantityLots::new(200),
            stress_bps: 0,
        }
    }

    fn holding(position: i64) -> AgentContext {
        AgentContext {
            logical_time: LogicalTimeNs::new(0),
            current_position: QuantityLots::new(position),
            remaining_parent_quantity: QuantityLots::new(0),
        }
    }

    #[test]
    fn a_signal_sets_a_target_and_the_order_closes_the_distance() {
        // Fundamental 10 ticks above mid: confidence 0.5, target +250,
        // joined at the bid.
        assert_eq!(
            quarcc_signal(&holding(0), &seen(10_010), &config()),
            Some((Side::Buy, QuantityLots::new(250), PriceTicks::new(9_999)))
        );
        // Already holding 100: only the rest.
        assert_eq!(
            quarcc_signal(&holding(100), &seen(10_010), &config()).map(|(_, q, _)| q.get()),
            Some(150)
        );
        // Strong signal below mid: target −500, taken at the bid.
        assert_eq!(
            quarcc_signal(&holding(0), &seen(9_960), &config()),
            Some((Side::Sell, QuantityLots::new(500), PriceTicks::new(9_999)))
        );
        // At the target: nothing; the working order is cancelled.
        assert_eq!(quarcc_signal(&holding(250), &seen(10_010), &config()), None);
        // The signal flips: unwind, joining the ask.
        assert_eq!(
            quarcc_signal(&holding(250), &seen(10_000), &config()),
            Some((Side::Sell, QuantityLots::new(250), PriceTicks::new(10_001)))
        );
    }
}
