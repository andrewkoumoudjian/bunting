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
    OptionsFlow,
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
}
