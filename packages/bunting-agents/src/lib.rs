#![forbid(unsafe_code)]
//! Bunting-native deterministic agent policies composed with mandatory QUARCC execution.

use bunting_market_events::{OrderKind, Side};
use bunting_market_types::{InstrumentId, LogicalTimeNs, ParticipantId, PriceTicks, QuantityLots};
use quarcc_execution_engine::command::ExecutionIntent;
use quarcc_execution_engine::ids::{ClientOrderId, IntentId};
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

pub trait AgentPolicy {
    type Config;
    type Snapshot;

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
            },
        })
    }

    fn quote_both_sides(
        &mut self,
        context: &AgentContext,
        observation: &AgentObservation,
        output: &mut IntentBuffer,
    ) -> Result<(), AgentError> {
        let mid = observation
            .best_bid
            .get()
            .checked_add(observation.best_ask.get())
            .ok_or(AgentError::ArithmeticOverflow)?
            / 2;
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
        let center = mid.saturating_add(model_adjustment);
        self.submit(
            Side::Buy,
            self.state.config.base_quantity,
            PriceTicks::new(center.saturating_sub(half_spread).max(1)),
            output,
        )?;
        self.submit(
            Side::Sell,
            self.state.config.base_quantity,
            PriceTicks::new(center.saturating_add(half_spread).max(1)),
            output,
        )
    }
}

impl AgentPolicy for BuiltInPolicy {
    type Config = AgentConfig;
    type Snapshot = PolicySnapshot;

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
        if self.state.config.kind.is_market_maker() {
            self.quote_both_sides(context, observation, output)?;
        } else {
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
                PolicyKind::CrossVenueArbitrage
                | PolicyKind::Microprice
                | PolicyKind::Imbalance => {
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
            let price = match side {
                Side::Buy => observation
                    .best_bid
                    .get()
                    .saturating_sub(displacement)
                    .max(1),
                Side::Sell => observation.best_ask.get().saturating_add(displacement),
            };
            self.submit(side, quantity, PriceTicks::new(price), output)?;
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagedAgentSnapshot<S> {
    pub policy: S,
    pub execution: ExecutionSnapshot,
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
        let mut actions = ExecutionActionBuffer::with_limit(limit);
        for intent in intents.drain() {
            self.execution.submit_intent(intent, &mut actions)?;
        }
        Ok((next, actions.into_vec()))
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
            self.execution.submit_intent(intent, &mut actions)?;
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
}
