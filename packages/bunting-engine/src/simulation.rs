//! Authoritative deterministic simulation-domain state and projections.

use bunting_ledger::{
    JournalPosting, JournalTransaction, LedgerError, PortfolioLedger, PostingAccount,
    TransactionKind,
};
use bunting_market_events::{
    ClockMode, CompositeLeg, CompositePolicy, EventPayload, NewsAudience, OtcDecision, Side,
    SimulationCommand, SimulationEvent, TenderDecision,
};
use bunting_market_types::{
    CurrencyId, FacilityId, InstrumentId, LogicalTimeNs, MoneyMinor, NegotiationId, NewsId,
    OrderId, ParticipantId, PriceTicks, QuantityLots, TenderId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Version of Bunting-native simulation policies in this module.
pub const SIMULATION_POLICY_VERSION: u16 = 1;
/// Maximum retained trades per instrument.
pub const MAX_TRADE_HISTORY: usize = 4_096;
/// Maximum news items per run.
pub const MAX_NEWS_ITEMS: usize = 4_096;
/// Maximum pending scheduled actions per run.
pub const MAX_SCHEDULED_ACTIONS: usize = 8_192;
/// Maximum legs in one composite command.
pub const MAX_COMPOSITE_LEGS: usize = 32;

/// Venue-independent economic product classification.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstrumentKind {
    Equity,
    Currency,
    Bond {
        coupon_bps: u32,
        maturity: LogicalTimeNs,
    },
    Option {
        underlying: InstrumentId,
        strike: PriceTicks,
        expiry: LogicalTimeNs,
        is_call: bool,
    },
    Future {
        underlying: InstrumentId,
        expiry: LogicalTimeNs,
        physical_delivery: bool,
    },
    Commodity,
    Synthetic {
        components: Vec<(InstrumentId, i64)>,
    },
}

/// Immutable economic instrument definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EconomicInstrument {
    pub instrument_id: InstrumentId,
    pub symbol: String,
    pub settlement_currency: CurrencyId,
    pub kind: InstrumentKind,
    pub contract_multiplier: i64,
}

/// Versioned facility category.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FacilityKind {
    Asset,
    Lease,
    Transport,
    Storage,
    Production,
    Conversion,
}

/// Immutable capacity-constrained facility definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityDefinition {
    pub facility_id: FacilityId,
    pub kind: FacilityKind,
    pub capacity: QuantityLots,
    pub input_instrument: Option<InstrumentId>,
    pub output_instrument: Option<InstrumentId>,
}

/// Deterministic run lifecycle.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLifecycle {
    #[default]
    Stopped,
    Active,
    Paused,
    Terminated,
}

/// Logical clock separated from wall-time pacing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogicalClock {
    pub now: LogicalTimeNs,
    pub step_ns: u64,
    pub mode: ClockMode,
}

impl Default for LogicalClock {
    fn default() -> Self {
        Self {
            now: LogicalTimeNs::new(0),
            step_ns: 1_000_000,
            mode: ClockMode::Lockstep,
        }
    }
}

/// Immutable scenario input for the full simulation domain.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimulationScenario {
    pub policy_version: u16,
    pub clock: LogicalClock,
    #[serde(with = "scenario_instruments")]
    pub instruments: BTreeMap<InstrumentId, EconomicInstrument>,
    #[serde(with = "scenario_facilities")]
    pub facilities: BTreeMap<FacilityId, FacilityDefinition>,
    pub scheduled_actions: Vec<ScheduledAction>,
    pub initial_news: Vec<NewsItem>,
}

mod scenario_instruments {
    use super::{EconomicInstrument, InstrumentId};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        value: &BTreeMap<InstrumentId, EconomicInstrument>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        value.values().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<InstrumentId, EconomicInstrument>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<EconomicInstrument>::deserialize(deserializer)?;
        let mut output = BTreeMap::new();
        for value in values {
            if output.insert(value.instrument_id, value).is_some() {
                return Err(serde::de::Error::custom("duplicate instrument"));
            }
        }
        Ok(output)
    }
}

mod scenario_facilities {
    use super::{FacilityDefinition, FacilityId};
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        value: &BTreeMap<FacilityId, FacilityDefinition>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        value.values().collect::<Vec<_>>().serialize(serializer)
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<FacilityId, FacilityDefinition>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let values = Vec::<FacilityDefinition>::deserialize(deserializer)?;
        let mut output = BTreeMap::new();
        for value in values {
            if output.insert(value.facility_id, value).is_some() {
                return Err(serde::de::Error::custom("duplicate facility"));
            }
        }
        Ok(output)
    }
}

impl Default for SimulationScenario {
    fn default() -> Self {
        Self {
            policy_version: SIMULATION_POLICY_VERSION,
            clock: LogicalClock::default(),
            instruments: BTreeMap::new(),
            facilities: BTreeMap::new(),
            scheduled_actions: Vec::new(),
            initial_news: Vec::new(),
        }
    }
}

impl SimulationScenario {
    /// Validates bounds, identities, and cross references.
    ///
    /// # Errors
    /// Returns an error for unsupported versions, invalid units, or broken references.
    pub fn validate(&self) -> Result<(), SimulationError> {
        if self.policy_version != SIMULATION_POLICY_VERSION
            || self.clock.step_ns == 0
            || self.scheduled_actions.len() > MAX_SCHEDULED_ACTIONS
            || self.initial_news.len() > MAX_NEWS_ITEMS
        {
            return Err(SimulationError::InvalidScenario);
        }
        for (id, instrument) in &self.instruments {
            if *id != instrument.instrument_id
                || id.get() == 0
                || instrument.symbol.is_empty()
                || instrument.symbol.len() > 128
                || instrument.settlement_currency.get() == 0
                || instrument.contract_multiplier <= 0
            {
                return Err(SimulationError::InvalidScenario);
            }
        }
        for (id, facility) in &self.facilities {
            if *id != facility.facility_id
                || id.get() == 0
                || facility.capacity.get() <= 0
                || facility
                    .input_instrument
                    .is_some_and(|instrument| !self.instruments.contains_key(&instrument))
                || facility
                    .output_instrument
                    .is_some_and(|instrument| !self.instruments.contains_key(&instrument))
            {
                return Err(SimulationError::InvalidScenario);
            }
        }
        Ok(())
    }
}

/// One versioned product or facility action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledActionKind {
    Cashflow {
        participant_id: ParticipantId,
        currency_id: CurrencyId,
        amount: MoneyMinor,
        kind: TransactionKind,
    },
    ExpireInstrument {
        instrument_id: InstrumentId,
    },
    ExerciseOption {
        participant_id: ParticipantId,
        instrument_id: InstrumentId,
        quantity: QuantityLots,
    },
    Deliver {
        participant_id: ParticipantId,
        instrument_id: InstrumentId,
        quantity: QuantityLots,
    },
    CompleteFacilityJob {
        job_id: u128,
    },
}

/// Canonically ordered scheduled action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduledAction {
    pub action_id: u128,
    pub effective_at: LogicalTimeNs,
    pub kind: ScheduledActionKind,
}

/// Immutable public or private news item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NewsItem {
    pub news_id: NewsId,
    pub published_at: LogicalTimeNs,
    pub audience: NewsAudience,
    pub headline: String,
    pub body: String,
}

/// Committed trade projection entry.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TradeRecord {
    pub logical_time: LogicalTimeNs,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
}

/// Exact OHLC and volume bar.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OhlcBar {
    pub bucket_start: LogicalTimeNs,
    pub open: PriceTicks,
    pub high: PriceTicks,
    pub low: PriceTicks,
    pub close: PriceTicks,
    pub volume: QuantityLots,
}

/// One visible L1 price/quantity level.
pub type L1Level = (PriceTicks, QuantityLots);
/// Best bid and best ask from committed depth.
pub type L1Quote = (Option<L1Level>, Option<L1Level>);

/// Bounded committed public market-data projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarketProjection {
    pub raw_bids: Vec<(PriceTicks, QuantityLots, OrderId)>,
    pub raw_asks: Vec<(PriceTicks, QuantityLots, OrderId)>,
    pub aggregated_bids: Vec<(PriceTicks, QuantityLots)>,
    pub aggregated_asks: Vec<(PriceTicks, QuantityLots)>,
    pub trades: VecDeque<TradeRecord>,
    pub bars: VecDeque<OhlcBar>,
    pub cumulative_volume: QuantityLots,
}

impl MarketProjection {
    /// Records a committed trade and updates exact history and OHLC.
    ///
    /// # Errors
    /// Returns an error when quantity aggregation overflows.
    pub fn record_trade(&mut self, trade: TradeRecord) -> Result<(), SimulationError> {
        self.cumulative_volume = self
            .cumulative_volume
            .checked_add(trade.quantity)
            .ok_or(SimulationError::ArithmeticOverflow)?;
        if self.trades.len() == MAX_TRADE_HISTORY {
            self.trades.pop_front();
        }
        self.trades.push_back(trade);
        let bucket_ns = 1_000_000_000_u64;
        let bucket = LogicalTimeNs::new((trade.logical_time.get() / bucket_ns) * bucket_ns);
        if let Some(bar) = self
            .bars
            .back_mut()
            .filter(|bar| bar.bucket_start == bucket)
        {
            bar.high = bar.high.max(trade.price);
            bar.low = bar.low.min(trade.price);
            bar.close = trade.price;
            bar.volume = bar
                .volume
                .checked_add(trade.quantity)
                .ok_or(SimulationError::ArithmeticOverflow)?;
        } else {
            if self.bars.len() == MAX_TRADE_HISTORY {
                self.bars.pop_front();
            }
            self.bars.push_back(OhlcBar {
                bucket_start: bucket,
                open: trade.price,
                high: trade.price,
                low: trade.price,
                close: trade.price,
                volume: trade.quantity,
            });
        }
        Ok(())
    }

    /// Returns L1 from committed aggregated depth.
    #[must_use]
    pub fn l1(&self) -> L1Quote {
        (
            self.aggregated_bids.first().copied(),
            self.aggregated_asks.first().copied(),
        )
    }

    /// Calculates exact notional market impact over aggregated depth.
    #[must_use]
    pub fn impact(&self, side: Side, quantity: QuantityLots) -> Option<MoneyMinor> {
        let levels = match side {
            Side::Buy => &self.aggregated_asks,
            Side::Sell => &self.aggregated_bids,
        };
        let mut remaining = quantity;
        let mut notional = MoneyMinor::new(0);
        for (price, available) in levels {
            let fill = QuantityLots::new(remaining.get().min(available.get()));
            notional =
                notional.checked_add(MoneyMinor::checked_mul_price_quantity(*price, fill).ok()?)?;
            remaining = remaining.checked_sub(fill)?;
            if remaining.get() == 0 {
                return Some(notional);
            }
        }
        None
    }
}

/// Participant-private order and news projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrivateProjection {
    pub live_orders: BTreeSet<OrderId>,
    pub historical_orders: VecDeque<OrderId>,
    pub news: Vec<NewsId>,
}

/// Targeted tender lifecycle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TenderState {
    pub tender_id: TenderId,
    pub participant_id: ParticipantId,
    pub instrument_id: InstrumentId,
    pub side: Side,
    pub quantity: QuantityLots,
    pub price: PriceTicks,
    pub expires_at: LogicalTimeNs,
    pub status: String,
}

/// Bilateral OTC lifecycle separate from the CLOB.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OtcState {
    pub negotiation_id: NegotiationId,
    pub proposer_id: ParticipantId,
    pub counterparty_id: ParticipantId,
    pub instrument_id: InstrumentId,
    pub side: Side,
    pub quantity: QuantityLots,
    pub price: PriceTicks,
    pub expires_at: LogicalTimeNs,
    pub status: String,
}

/// Capacity reservation and completion state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FacilityJob {
    pub job_id: u128,
    pub facility_id: FacilityId,
    pub participant_id: ParticipantId,
    pub input_quantity: QuantityLots,
    pub output_quantity: QuantityLots,
    pub completes_at: LogicalTimeNs,
    pub completed: bool,
}

/// One deterministic participant score.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScoreEntry {
    pub participant_id: ParticipantId,
    pub score: MoneyMinor,
    pub rank: u32,
}

/// Frozen iteration report derived from committed state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IterationReport {
    pub policy_version: u16,
    pub generated_at: LogicalTimeNs,
    pub entries: Vec<ScoreEntry>,
}

/// Audited administrator mutation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdministratorChange {
    pub actor: ParticipantId,
    pub effective_at: LogicalTimeNs,
    pub reason: String,
}

/// Complete simulation component under the engine snapshot root.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct SimulationState {
    pub policy_version: u16,
    pub lifecycle: RunLifecycle,
    pub clock: LogicalClock,
    pub instruments: BTreeMap<InstrumentId, EconomicInstrument>,
    pub facilities: BTreeMap<FacilityId, FacilityDefinition>,
    pub halted_instruments: BTreeSet<InstrumentId>,
    pub scheduled_actions: Vec<ScheduledAction>,
    pub applied_actions: BTreeSet<u128>,
    pub market: BTreeMap<InstrumentId, MarketProjection>,
    pub private: BTreeMap<ParticipantId, PrivateProjection>,
    pub portfolio_ledger: PortfolioLedger,
    pub news: Vec<NewsItem>,
    pub tenders: BTreeMap<TenderId, TenderState>,
    pub otc: BTreeMap<NegotiationId, OtcState>,
    pub facility_jobs: BTreeMap<u128, FacilityJob>,
    pub administrator_changes: Vec<AdministratorChange>,
    pub reports: Vec<IterationReport>,
    pub next_action_id: u128,
    pub next_transaction_id: u128,
}

impl Default for SimulationState {
    fn default() -> Self {
        Self {
            policy_version: SIMULATION_POLICY_VERSION,
            lifecycle: RunLifecycle::Stopped,
            clock: LogicalClock::default(),
            instruments: BTreeMap::new(),
            facilities: BTreeMap::new(),
            halted_instruments: BTreeSet::new(),
            scheduled_actions: Vec::new(),
            applied_actions: BTreeSet::new(),
            market: BTreeMap::new(),
            private: BTreeMap::new(),
            portfolio_ledger: PortfolioLedger::new(),
            news: Vec::new(),
            tenders: BTreeMap::new(),
            otc: BTreeMap::new(),
            facility_jobs: BTreeMap::new(),
            administrator_changes: Vec::new(),
            reports: Vec::new(),
            next_action_id: 1,
            next_transaction_id: 1,
        }
    }
}

impl SimulationState {
    /// Creates deterministic run state from immutable scenario input.
    ///
    /// # Errors
    /// Returns an error when the scenario is invalid or contains duplicate actions.
    pub fn from_scenario(scenario: &SimulationScenario) -> Result<Self, SimulationError> {
        scenario.validate()?;
        let mut state = Self {
            clock: scenario.clock,
            instruments: scenario.instruments.clone(),
            facilities: scenario.facilities.clone(),
            news: scenario.initial_news.clone(),
            ..Self::default()
        };
        let mut action_ids = BTreeSet::new();
        for action in &scenario.scheduled_actions {
            if !action_ids.insert(action.action_id) {
                return Err(SimulationError::DuplicateIdentity);
            }
            state.scheduled_actions.push(action.clone());
            state.next_action_id = state.next_action_id.max(action.action_id.saturating_add(1));
        }
        state
            .scheduled_actions
            .sort_by_key(|action| (action.effective_at, action.action_id));
        Ok(state)
    }

    /// Applies a simulation command to candidate state and returns durable facts.
    ///
    /// # Errors
    /// Returns an error for invalid lifecycle, bounds, ownership, or arithmetic.
    #[expect(
        clippy::too_many_lines,
        reason = "the exhaustive command reducer keeps domain mutations and emitted facts visibly paired"
    )]
    pub fn apply(
        &mut self,
        actor: ParticipantId,
        logical_time: LogicalTimeNs,
        command: &SimulationCommand,
    ) -> Result<Vec<SimulationEvent>, SimulationError> {
        if logical_time < self.clock.now {
            return Err(SimulationError::LogicalTimeRegression);
        }
        match command {
            SimulationCommand::StartRun => {
                self.require_lifecycle(RunLifecycle::Stopped)?;
                self.lifecycle = RunLifecycle::Active;
                Ok(vec![SimulationEvent::LifecycleChanged {
                    status: "active".into(),
                }])
            }
            SimulationCommand::PauseRun => {
                self.require_lifecycle(RunLifecycle::Active)?;
                self.lifecycle = RunLifecycle::Paused;
                Ok(vec![SimulationEvent::LifecycleChanged {
                    status: "paused".into(),
                }])
            }
            SimulationCommand::ResumeRun => {
                self.require_lifecycle(RunLifecycle::Paused)?;
                self.lifecycle = RunLifecycle::Active;
                Ok(vec![SimulationEvent::LifecycleChanged {
                    status: "active".into(),
                }])
            }
            SimulationCommand::Advance { steps } => self.advance(*steps),
            SimulationCommand::SetPacing { mode, reason } => {
                validate_reason(reason)?;
                self.clock.mode = *mode;
                self.record_admin(actor, logical_time, reason.clone());
                Ok(vec![
                    SimulationEvent::PacingChanged {
                        mode: *mode,
                        reason: reason.clone(),
                    },
                    SimulationEvent::AdministratorChangeRecorded {
                        reason: reason.clone(),
                    },
                ])
            }
            SimulationCommand::Terminate { reason } => {
                validate_reason(reason)?;
                if self.lifecycle == RunLifecycle::Terminated {
                    return Err(SimulationError::InvalidLifecycle);
                }
                self.lifecycle = RunLifecycle::Terminated;
                self.record_admin(actor, logical_time, reason.clone());
                Ok(vec![
                    SimulationEvent::LifecycleChanged {
                        status: "terminated".into(),
                    },
                    SimulationEvent::AdministratorChangeRecorded {
                        reason: reason.clone(),
                    },
                ])
            }
            SimulationCommand::SetListingHalt {
                instrument_id,
                halted,
                reason,
            } => {
                validate_reason(reason)?;
                self.require_instrument(*instrument_id)?;
                if *halted {
                    self.halted_instruments.insert(*instrument_id);
                } else {
                    self.halted_instruments.remove(instrument_id);
                }
                self.record_admin(actor, logical_time, reason.clone());
                Ok(vec![
                    SimulationEvent::ListingHaltChanged {
                        instrument_id: *instrument_id,
                        halted: *halted,
                    },
                    SimulationEvent::AdministratorChangeRecorded {
                        reason: reason.clone(),
                    },
                ])
            }
            SimulationCommand::PublishNews {
                news_id,
                audience,
                headline,
                body,
            } => {
                if self.news.len() >= MAX_NEWS_ITEMS
                    || headline.is_empty()
                    || headline.len() > 256
                    || body.len() > 16_384
                    || self.news.iter().any(|news| news.news_id == *news_id)
                {
                    return Err(SimulationError::BoundExceeded);
                }
                self.news.push(NewsItem {
                    news_id: *news_id,
                    published_at: logical_time,
                    audience: audience.clone(),
                    headline: headline.clone(),
                    body: body.clone(),
                });
                self.route_news(*news_id, audience);
                Ok(vec![SimulationEvent::NewsPublished {
                    news_id: *news_id,
                    audience: audience.clone(),
                }])
            }
            SimulationCommand::OpenTender {
                tender_id,
                participant_id,
                instrument_id,
                side,
                quantity,
                price,
                expires_at,
            } => {
                self.require_instrument(*instrument_id)?;
                if quantity.get() <= 0
                    || price.get() <= 0
                    || *expires_at <= logical_time
                    || self.tenders.contains_key(tender_id)
                {
                    return Err(SimulationError::InvalidCommand);
                }
                self.tenders.insert(
                    *tender_id,
                    TenderState {
                        tender_id: *tender_id,
                        participant_id: *participant_id,
                        instrument_id: *instrument_id,
                        side: *side,
                        quantity: *quantity,
                        price: *price,
                        expires_at: *expires_at,
                        status: "open".into(),
                    },
                );
                Ok(vec![SimulationEvent::TenderChanged {
                    tender_id: *tender_id,
                    status: "open".into(),
                }])
            }
            SimulationCommand::DecideTender {
                tender_id,
                decision,
            } => {
                let tender = self
                    .tenders
                    .get_mut(tender_id)
                    .ok_or(SimulationError::UnknownIdentity)?;
                if tender.participant_id != actor
                    || tender.status != "open"
                    || tender.expires_at <= logical_time
                {
                    return Err(SimulationError::InvalidLifecycle);
                }
                tender.status = match decision {
                    TenderDecision::Accept => "accepted",
                    TenderDecision::Decline => "declined",
                }
                .into();
                Ok(vec![SimulationEvent::TenderChanged {
                    tender_id: *tender_id,
                    status: tender.status.clone(),
                }])
            }
            SimulationCommand::OpenOtc {
                negotiation_id,
                counterparty_id,
                instrument_id,
                side,
                quantity,
                price,
                expires_at,
            } => {
                self.require_instrument(*instrument_id)?;
                if quantity.get() <= 0
                    || price.get() <= 0
                    || *expires_at <= logical_time
                    || self.otc.contains_key(negotiation_id)
                {
                    return Err(SimulationError::InvalidCommand);
                }
                self.otc.insert(
                    *negotiation_id,
                    OtcState {
                        negotiation_id: *negotiation_id,
                        proposer_id: actor,
                        counterparty_id: *counterparty_id,
                        instrument_id: *instrument_id,
                        side: *side,
                        quantity: *quantity,
                        price: *price,
                        expires_at: *expires_at,
                        status: "proposed".into(),
                    },
                );
                Ok(vec![SimulationEvent::OtcChanged {
                    negotiation_id: *negotiation_id,
                    status: "proposed".into(),
                }])
            }
            SimulationCommand::CounterOtc {
                negotiation_id,
                quantity,
                price,
            } => {
                let otc = self
                    .otc
                    .get_mut(negotiation_id)
                    .ok_or(SimulationError::UnknownIdentity)?;
                if !matches!(actor, value if value == otc.proposer_id || value == otc.counterparty_id)
                    || otc.status == "accepted"
                    || otc.status == "broken"
                    || quantity.get() <= 0
                    || price.get() <= 0
                {
                    return Err(SimulationError::InvalidLifecycle);
                }
                otc.quantity = *quantity;
                otc.price = *price;
                otc.status = "countered".into();
                Ok(vec![SimulationEvent::OtcChanged {
                    negotiation_id: *negotiation_id,
                    status: otc.status.clone(),
                }])
            }
            SimulationCommand::DecideOtc {
                negotiation_id,
                decision,
            } => {
                let otc = self
                    .otc
                    .get_mut(negotiation_id)
                    .ok_or(SimulationError::UnknownIdentity)?;
                if actor != otc.counterparty_id && !matches!(decision, OtcDecision::Break) {
                    return Err(SimulationError::NotOwner);
                }
                otc.status = match decision {
                    OtcDecision::Accept => "accepted",
                    OtcDecision::Reject => "rejected",
                    OtcDecision::Break => "broken",
                }
                .into();
                Ok(vec![SimulationEvent::OtcChanged {
                    negotiation_id: *negotiation_id,
                    status: otc.status.clone(),
                }])
            }
            SimulationCommand::SubmitComposite {
                policy,
                minimum_fill,
                legs,
            } => self.apply_composite(*policy, *minimum_fill, legs),
            SimulationCommand::ScheduleCashflow {
                participant_id,
                currency_id,
                amount,
                effective_at,
                reason,
            } => {
                validate_reason(reason)?;
                let action_id = self.next_action();
                self.schedule(ScheduledAction {
                    action_id,
                    effective_at: *effective_at,
                    kind: ScheduledActionKind::Cashflow {
                        participant_id: *participant_id,
                        currency_id: *currency_id,
                        amount: *amount,
                        kind: TransactionKind::Adjustment,
                    },
                })?;
                Ok(vec![SimulationEvent::CashflowScheduled {
                    participant_id: *participant_id,
                    currency_id: *currency_id,
                    amount: *amount,
                }])
            }
            SimulationCommand::ScheduleFacilityJob {
                facility_id,
                participant_id,
                input_quantity,
                output_quantity,
                completes_at,
            } => {
                let facility = self
                    .facilities
                    .get(facility_id)
                    .ok_or(SimulationError::UnknownIdentity)?;
                let reserved = self
                    .facility_jobs
                    .values()
                    .filter(|job| job.facility_id == *facility_id && !job.completed)
                    .try_fold(QuantityLots::new(0), |total, job| {
                        total.checked_add(job.input_quantity)
                    })
                    .ok_or(SimulationError::ArithmeticOverflow)?;
                if input_quantity.get() <= 0
                    || output_quantity.get() <= 0
                    || reserved
                        .checked_add(*input_quantity)
                        .is_none_or(|total| total > facility.capacity)
                    || *completes_at <= logical_time
                {
                    return Err(SimulationError::InvalidCommand);
                }
                let job_id = self.next_action();
                self.facility_jobs.insert(
                    job_id,
                    FacilityJob {
                        job_id,
                        facility_id: *facility_id,
                        participant_id: *participant_id,
                        input_quantity: *input_quantity,
                        output_quantity: *output_quantity,
                        completes_at: *completes_at,
                        completed: false,
                    },
                );
                self.schedule(ScheduledAction {
                    action_id: job_id,
                    effective_at: *completes_at,
                    kind: ScheduledActionKind::CompleteFacilityJob { job_id },
                })?;
                Ok(vec![SimulationEvent::FacilityJobScheduled {
                    facility_id: *facility_id,
                    participant_id: *participant_id,
                }])
            }
            SimulationCommand::ScoreIteration => self.score_iteration(),
            SimulationCommand::MassCancel { .. } => Err(SimulationError::RequiresMatchingState),
        }
    }

    /// Projects one committed canonical event into public and private views.
    ///
    /// # Errors
    /// Returns an error when exact market-data aggregation overflows.
    pub fn project_event(
        &mut self,
        logical_time: LogicalTimeNs,
        event: &EventPayload,
    ) -> Result<(), SimulationError> {
        match event {
            EventPayload::OrderRested {
                order_id,
                participant_id,
                ..
            } => {
                self.private
                    .entry(*participant_id)
                    .or_default()
                    .live_orders
                    .insert(*order_id);
            }
            EventPayload::OrderCanceled {
                order_id,
                participant_id,
                ..
            } => {
                let projection = self.private.entry(*participant_id).or_default();
                projection.live_orders.remove(order_id);
                if projection.historical_orders.len() == MAX_TRADE_HISTORY {
                    projection.historical_orders.pop_front();
                }
                projection.historical_orders.push_back(*order_id);
            }
            EventPayload::TradeExecuted {
                instrument_id,
                price,
                quantity,
                ..
            } => {
                self.market
                    .entry(*instrument_id)
                    .or_default()
                    .record_trade(TradeRecord {
                        logical_time,
                        price: *price,
                        quantity: *quantity,
                    })?;
            }
            _ => {}
        }
        Ok(())
    }

    /// Replaces committed aggregated L2 while preserving deterministic order.
    pub fn set_depth(
        &mut self,
        instrument_id: InstrumentId,
        raw_bids: Vec<(PriceTicks, QuantityLots, OrderId)>,
        raw_asks: Vec<(PriceTicks, QuantityLots, OrderId)>,
        bids: Vec<(PriceTicks, QuantityLots)>,
        asks: Vec<(PriceTicks, QuantityLots)>,
    ) {
        let projection = self.market.entry(instrument_id).or_default();
        projection.raw_bids = raw_bids;
        projection.raw_asks = raw_asks;
        projection.aggregated_bids = bids;
        projection.aggregated_asks = asks;
    }

    fn advance(&mut self, steps: u32) -> Result<Vec<SimulationEvent>, SimulationError> {
        self.require_lifecycle(RunLifecycle::Active)?;
        if steps == 0 {
            return Err(SimulationError::InvalidCommand);
        }
        let limit = match self.clock.mode {
            ClockMode::Lockstep | ClockMode::Paced { .. } => 1,
            ClockMode::Accelerated {
                max_steps_per_advance,
            } => max_steps_per_advance.max(1),
        };
        if steps > limit {
            return Err(SimulationError::BoundExceeded);
        }
        let from = self.clock.now;
        let delta = self
            .clock
            .step_ns
            .checked_mul(u64::from(steps))
            .ok_or(SimulationError::ArithmeticOverflow)?;
        let to = LogicalTimeNs::new(
            from.get()
                .checked_add(delta)
                .ok_or(SimulationError::ArithmeticOverflow)?,
        );
        self.clock.now = to;
        let mut events = vec![SimulationEvent::ClockAdvanced { from, to }];
        let pending = self.scheduled_actions.split_off(
            self.scheduled_actions
                .partition_point(|action| action.effective_at <= to),
        );
        let due = std::mem::replace(&mut self.scheduled_actions, pending);
        for action in due {
            self.apply_scheduled(&action)?;
            self.applied_actions.insert(action.action_id);
            events.push(SimulationEvent::ScheduledActionApplied {
                action_id: action.action_id,
            });
        }
        for tender in self.tenders.values_mut() {
            if tender.status == "open" && tender.expires_at <= to {
                tender.status = "expired".into();
            }
        }
        for otc in self.otc.values_mut() {
            if matches!(otc.status.as_str(), "proposed" | "countered") && otc.expires_at <= to {
                otc.status = "expired".into();
            }
        }
        Ok(events)
    }

    fn apply_scheduled(&mut self, action: &ScheduledAction) -> Result<(), SimulationError> {
        match action.kind {
            ScheduledActionKind::Cashflow {
                participant_id,
                currency_id,
                amount,
                kind,
            } => {
                let transaction_id = self.next_transaction();
                self.portfolio_ledger.post(JournalTransaction {
                    transaction_id,
                    kind,
                    postings: vec![
                        JournalPosting {
                            participant_id: Some(participant_id),
                            currency_id,
                            account: PostingAccount::Cash,
                            amount,
                        },
                        JournalPosting {
                            participant_id: None,
                            currency_id,
                            account: PostingAccount::Clearing,
                            amount: MoneyMinor::new(
                                amount
                                    .get()
                                    .checked_neg()
                                    .ok_or(SimulationError::ArithmeticOverflow)?,
                            ),
                        },
                    ],
                })?;
            }
            ScheduledActionKind::CompleteFacilityJob { job_id } => {
                let job = self
                    .facility_jobs
                    .get(&job_id)
                    .cloned()
                    .ok_or(SimulationError::UnknownIdentity)?;
                let facility = self
                    .facilities
                    .get(&job.facility_id)
                    .ok_or(SimulationError::UnknownIdentity)?;
                let mut ledger = self.portfolio_ledger.clone();
                if let Some(input) = facility.input_instrument {
                    ledger.adjust_position(
                        job.participant_id,
                        input,
                        QuantityLots::new(
                            job.input_quantity
                                .get()
                                .checked_neg()
                                .ok_or(SimulationError::ArithmeticOverflow)?,
                        ),
                    )?;
                }
                if let Some(output) = facility.output_instrument {
                    ledger.adjust_position(job.participant_id, output, job.output_quantity)?;
                }
                self.portfolio_ledger = ledger;
                self.facility_jobs
                    .get_mut(&job_id)
                    .ok_or(SimulationError::UnknownIdentity)?
                    .completed = true;
            }
            ScheduledActionKind::ExpireInstrument { instrument_id } => {
                self.halted_instruments.insert(instrument_id);
            }
            ScheduledActionKind::ExerciseOption { .. } | ScheduledActionKind::Deliver { .. } => {}
        }
        Ok(())
    }

    fn apply_composite(
        &self,
        policy: CompositePolicy,
        minimum_fill: QuantityLots,
        legs: &[CompositeLeg],
    ) -> Result<Vec<SimulationEvent>, SimulationError> {
        if legs.is_empty() || legs.len() > MAX_COMPOSITE_LEGS || minimum_fill.get() < 0 {
            return Err(SimulationError::BoundExceeded);
        }
        for leg in legs {
            self.require_instrument(leg.instrument_id)?;
            if leg.quantity.get() <= 0
                || leg.limit_price.get() <= 0
                || self.halted_instruments.contains(&leg.instrument_id)
            {
                return Err(SimulationError::InvalidCommand);
            }
        }
        if matches!(
            policy,
            CompositePolicy::AllOrNone | CompositePolicy::AtomicConversion
        ) && minimum_fill.get() > 0
            && legs.iter().any(|leg| leg.quantity < minimum_fill)
        {
            return Err(SimulationError::InvalidCommand);
        }
        Ok(vec![SimulationEvent::CompositeCompleted {
            policy,
            accepted_legs: u32::try_from(legs.len()).map_err(|_| SimulationError::BoundExceeded)?,
        }])
    }

    fn score_iteration(&mut self) -> Result<Vec<SimulationEvent>, SimulationError> {
        let currency = self
            .instruments
            .values()
            .next()
            .map(|instrument| instrument.settlement_currency)
            .ok_or(SimulationError::InvalidScenario)?;
        let participants = self
            .private
            .keys()
            .copied()
            .chain(self.tenders.values().map(|tender| tender.participant_id))
            .collect::<BTreeSet<_>>();
        let mut entries = participants
            .into_iter()
            .map(|participant_id| {
                self.portfolio_ledger
                    .net_liquidation_value(participant_id, currency)
                    .map(|score| ScoreEntry {
                        participant_id,
                        score,
                        rank: 0,
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| (std::cmp::Reverse(entry.score), entry.participant_id));
        for (index, entry) in entries.iter_mut().enumerate() {
            entry.rank = u32::try_from(index + 1).map_err(|_| SimulationError::BoundExceeded)?;
        }
        let count = u32::try_from(entries.len()).map_err(|_| SimulationError::BoundExceeded)?;
        self.reports.push(IterationReport {
            policy_version: SIMULATION_POLICY_VERSION,
            generated_at: self.clock.now,
            entries,
        });
        Ok(vec![SimulationEvent::IterationScored {
            participant_count: count,
        }])
    }

    fn require_lifecycle(&self, required: RunLifecycle) -> Result<(), SimulationError> {
        if self.lifecycle == required {
            Ok(())
        } else {
            Err(SimulationError::InvalidLifecycle)
        }
    }

    fn require_instrument(&self, instrument: InstrumentId) -> Result<(), SimulationError> {
        if self.instruments.contains_key(&instrument) {
            Ok(())
        } else {
            Err(SimulationError::UnknownIdentity)
        }
    }

    fn route_news(&mut self, news_id: NewsId, audience: &NewsAudience) {
        match audience {
            NewsAudience::Participant(participant) => self
                .private
                .entry(*participant)
                .or_default()
                .news
                .push(news_id),
            NewsAudience::Public | NewsAudience::Team(_) | NewsAudience::Role(_) => {}
        }
    }

    fn record_admin(&mut self, actor: ParticipantId, effective_at: LogicalTimeNs, reason: String) {
        self.administrator_changes.push(AdministratorChange {
            actor,
            effective_at,
            reason,
        });
    }

    fn schedule(&mut self, action: ScheduledAction) -> Result<(), SimulationError> {
        if self.scheduled_actions.len() >= MAX_SCHEDULED_ACTIONS
            || action.effective_at <= self.clock.now
            || self
                .scheduled_actions
                .iter()
                .any(|existing| existing.action_id == action.action_id)
        {
            return Err(SimulationError::BoundExceeded);
        }
        self.scheduled_actions.push(action);
        self.scheduled_actions
            .sort_by_key(|item| (item.effective_at, item.action_id));
        Ok(())
    }

    fn next_action(&mut self) -> u128 {
        let value = self.next_action_id;
        self.next_action_id = self.next_action_id.saturating_add(1);
        value
    }

    fn next_transaction(&mut self) -> u128 {
        let value = self.next_transaction_id;
        self.next_transaction_id = self.next_transaction_id.saturating_add(1);
        value
    }
}

fn validate_reason(reason: &str) -> Result<(), SimulationError> {
    if reason.is_empty() || reason.len() > 1_024 {
        Err(SimulationError::InvalidCommand)
    } else {
        Ok(())
    }
}

/// Stable simulation transition failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SimulationError {
    InvalidScenario,
    InvalidLifecycle,
    InvalidCommand,
    LogicalTimeRegression,
    BoundExceeded,
    DuplicateIdentity,
    UnknownIdentity,
    NotOwner,
    ArithmeticOverflow,
    RequiresMatchingState,
    Ledger,
}

impl From<LedgerError> for SimulationError {
    fn from(_: LedgerError) -> Self {
        Self::Ledger
    }
}
