//! Authoritative deterministic simulation-domain state and projections.

use crate::ParticipantDefinition;
use bunting_ledger::{Fill, FillParty, FxRate, Ledger, LedgerError};
use bunting_market_events::{
    ClockMode, EventPayload, NewsAudience, OtcDecision, Side, SimulationCommand, SimulationEvent,
    TenderDecision,
};
use bunting_market_types::{
    CurrencyId, FacilityId, InstrumentId, ListingKey, LogicalTimeNs, MoneyMinor, NegotiationId,
    NewsId, ParticipantId, PriceTicks, QuantityLots, TenderId,
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
    #[serde(with = "scenario_facilities")]
    pub facilities: BTreeMap<FacilityId, FacilityDefinition>,
    pub scheduled_actions: Vec<ScheduledAction>,
    pub initial_news: Vec<NewsItem>,
    /// Whether the run accepts orders immediately or waits for `StartRun`.
    pub starts_active: bool,
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
            facilities: BTreeMap::new(),
            scheduled_actions: Vec::new(),
            initial_news: Vec::new(),
            starts_active: true,
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
        for (id, facility) in &self.facilities {
            if *id != facility.facility_id
                || id.get() == 0
                || facility.capacity.get() <= 0
                || (facility.input_instrument.is_none() && facility.output_instrument.is_none())
            {
                return Err(SimulationError::InvalidScenario);
            }
        }
        if self.scheduled_actions.iter().any(|action| {
            matches!(
                action.kind,
                ScheduledActionKind::ExerciseOption { .. } | ScheduledActionKind::Deliver { .. }
            )
        }) {
            // Option exercise and physical delivery have no settled semantics yet;
            // refuse them rather than silently ignoring scheduled economic events.
            return Err(SimulationError::Unsupported);
        }
        Ok(())
    }

    /// Instruments referenced by facilities and scheduled actions.
    pub fn referenced_instruments(&self) -> impl Iterator<Item = InstrumentId> + '_ {
        self.facilities
            .values()
            .flat_map(|facility| [facility.input_instrument, facility.output_instrument])
            .flatten()
            .chain(
                self.scheduled_actions
                    .iter()
                    .filter_map(|action| match action.kind {
                        ScheduledActionKind::ExpireInstrument { instrument_id }
                        | ScheduledActionKind::ExerciseOption { instrument_id, .. }
                        | ScheduledActionKind::Deliver { instrument_id, .. } => Some(instrument_id),
                        ScheduledActionKind::Cashflow { .. }
                        | ScheduledActionKind::CompleteFacilityJob { .. } => None,
                    }),
            )
    }
}

/// Economic authority and run facts a simulation command may read or post to.
pub struct SimulationContext<'a> {
    pub ledger: &'a mut Ledger,
    pub participants: &'a BTreeMap<ParticipantId, ParticipantDefinition>,
    pub reporting_currency: CurrencyId,
    pub fx_rates: &'a [FxRate],
}

impl SimulationContext<'_> {
    fn require_instrument(&self, instrument: InstrumentId) -> Result<(), SimulationError> {
        self.ledger
            .terms(instrument)
            .map(|_| ())
            .map_err(|_| SimulationError::UnknownIdentity)
    }

    fn require_participant(&self, participant: ParticipantId) -> Result<(), SimulationError> {
        if self.participants.contains_key(&participant) {
            Ok(())
        } else {
            Err(SimulationError::UnknownIdentity)
        }
    }

    fn party(&self, participant: ParticipantId) -> FillParty {
        FillParty {
            participant,
            fee: MoneyMinor::new(0),
            order: None,
            enforce_funding: self
                .participants
                .get(&participant)
                .is_some_and(|definition| definition.limits().cash_constrained),
        }
    }
}

/// Economic reason recorded with a scheduled cashflow.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CashflowKind {
    Dividend,
    Coupon,
    Interest,
    Fee,
    Settlement,
    Adjustment,
}

/// One versioned product or facility action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledActionKind {
    Cashflow {
        participant_id: ParticipantId,
        currency_id: CurrencyId,
        amount: MoneyMinor,
        kind: CashflowKind,
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

/// Bounded committed public trade history. Depth is read from the live book.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct MarketProjection {
    pub trades: VecDeque<TradeRecord>,
    pub bars: VecDeque<OhlcBar>,
    pub cumulative_volume: QuantityLots,
}

impl MarketProjection {
    fn validate_trade(&self, trade: TradeRecord) -> Result<(), SimulationError> {
        self.cumulative_volume
            .checked_add(trade.quantity)
            .ok_or(SimulationError::ArithmeticOverflow)?;
        let bucket_ns = 1_000_000_000_u64;
        let bucket = LogicalTimeNs::new((trade.logical_time.get() / bucket_ns) * bucket_ns);
        if let Some(bar) = self.bars.back().filter(|bar| bar.bucket_start == bucket) {
            bar.volume
                .checked_add(trade.quantity)
                .ok_or(SimulationError::ArithmeticOverflow)?;
        }
        Ok(())
    }

    /// Records a committed trade and updates exact history and OHLC.
    ///
    /// # Errors
    /// Returns an error when quantity aggregation overflows.
    pub fn record_trade(&mut self, trade: TradeRecord) -> Result<(), SimulationError> {
        self.validate_trade(trade)?;
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
}

/// Participant-private news projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrivateProjection {
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
    /// Party whose response the open negotiation awaits.
    pub awaiting: ParticipantId,
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
    pub facilities: BTreeMap<FacilityId, FacilityDefinition>,
    pub halted_instruments: BTreeSet<InstrumentId>,
    pub scheduled_actions: Vec<ScheduledAction>,
    pub applied_actions: BTreeSet<u128>,
    pub market: BTreeMap<InstrumentId, MarketProjection>,
    /// Raw depth and tape are scoped to the exchange listing, not the economic asset.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub market_by_listing: BTreeMap<ListingKey, MarketProjection>,
    pub private: BTreeMap<ParticipantId, PrivateProjection>,
    pub news: Vec<NewsItem>,
    pub tenders: BTreeMap<TenderId, TenderState>,
    pub otc: BTreeMap<NegotiationId, OtcState>,
    pub facility_jobs: BTreeMap<u128, FacilityJob>,
    pub administrator_changes: Vec<AdministratorChange>,
    pub reports: Vec<IterationReport>,
    pub next_action_id: u128,
}

impl Default for SimulationState {
    fn default() -> Self {
        Self {
            policy_version: SIMULATION_POLICY_VERSION,
            lifecycle: RunLifecycle::Stopped,
            clock: LogicalClock::default(),
            facilities: BTreeMap::new(),
            halted_instruments: BTreeSet::new(),
            scheduled_actions: Vec::new(),
            applied_actions: BTreeSet::new(),
            market: BTreeMap::new(),
            market_by_listing: BTreeMap::new(),
            private: BTreeMap::new(),
            news: Vec::new(),
            tenders: BTreeMap::new(),
            otc: BTreeMap::new(),
            facility_jobs: BTreeMap::new(),
            administrator_changes: Vec::new(),
            reports: Vec::new(),
            next_action_id: 1,
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
            lifecycle: if scenario.starts_active {
                RunLifecycle::Active
            } else {
                RunLifecycle::Stopped
            },
            clock: scenario.clock,
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
        context: &mut SimulationContext<'_>,
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
            SimulationCommand::Advance { steps } => self.advance(context, *steps),
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
                context.require_instrument(*instrument_id)?;
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
                context.require_instrument(*instrument_id)?;
                context.require_participant(*participant_id)?;
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
                let mut events = Vec::new();
                if *decision == TenderDecision::Accept {
                    let party = context.party(tender.participant_id);
                    let (buyer, seller) = match tender.side {
                        Side::Buy => (Some(party), None),
                        Side::Sell => (None, Some(party)),
                    };
                    context.ledger.settle(Fill {
                        instrument: tender.instrument_id,
                        price: tender.price,
                        quantity: tender.quantity,
                        buyer,
                        seller,
                        set_mark: false,
                    })?;
                    events.push(SimulationEvent::TenderSettled {
                        tender_id: *tender_id,
                        participant_id: tender.participant_id,
                        instrument_id: tender.instrument_id,
                        side: tender.side,
                        quantity: tender.quantity,
                        price: tender.price,
                    });
                }
                tender.status = match decision {
                    TenderDecision::Accept => "accepted",
                    TenderDecision::Decline => "declined",
                }
                .into();
                events.insert(
                    0,
                    SimulationEvent::TenderChanged {
                        tender_id: *tender_id,
                        status: tender.status.clone(),
                    },
                );
                Ok(events)
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
                context.require_instrument(*instrument_id)?;
                context.require_participant(actor)?;
                context.require_participant(*counterparty_id)?;
                if quantity.get() <= 0
                    || price.get() <= 0
                    || *expires_at <= logical_time
                    || *counterparty_id == actor
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
                        awaiting: *counterparty_id,
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
                if actor != otc.awaiting
                    || !matches!(otc.status.as_str(), "proposed" | "countered")
                    || otc.expires_at <= logical_time
                    || quantity.get() <= 0
                    || price.get() <= 0
                {
                    return Err(SimulationError::InvalidLifecycle);
                }
                otc.quantity = *quantity;
                otc.price = *price;
                otc.status = "countered".into();
                otc.awaiting = if actor == otc.proposer_id {
                    otc.counterparty_id
                } else {
                    otc.proposer_id
                };
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
                let party = actor == otc.proposer_id || actor == otc.counterparty_id;
                let responder = actor == otc.awaiting;
                if !(party && (responder || *decision == OtcDecision::Break)) {
                    return Err(SimulationError::NotOwner);
                }
                // Settled trades are final; only open negotiations can be broken.
                if !matches!(otc.status.as_str(), "proposed" | "countered")
                    || otc.expires_at <= logical_time
                {
                    return Err(SimulationError::InvalidLifecycle);
                }
                let mut events = Vec::new();
                if *decision == OtcDecision::Accept {
                    let (buyer_id, seller_id) = match otc.side {
                        Side::Buy => (otc.proposer_id, otc.counterparty_id),
                        Side::Sell => (otc.counterparty_id, otc.proposer_id),
                    };
                    context.ledger.settle(Fill {
                        instrument: otc.instrument_id,
                        price: otc.price,
                        quantity: otc.quantity,
                        buyer: Some(context.party(buyer_id)),
                        seller: Some(context.party(seller_id)),
                        set_mark: false,
                    })?;
                    events.push(SimulationEvent::OtcSettled {
                        negotiation_id: *negotiation_id,
                        buyer_id,
                        seller_id,
                        instrument_id: otc.instrument_id,
                        quantity: otc.quantity,
                        price: otc.price,
                    });
                }
                otc.status = match decision {
                    OtcDecision::Accept => "accepted",
                    OtcDecision::Reject => "rejected",
                    OtcDecision::Break => "broken",
                }
                .into();
                events.insert(
                    0,
                    SimulationEvent::OtcChanged {
                        negotiation_id: *negotiation_id,
                        status: otc.status.clone(),
                    },
                );
                Ok(events)
            }
            SimulationCommand::SubmitComposite {
                policy,
                minimum_fill,
                legs,
            } => {
                // Multi-leg execution has no atomic matching/settlement path yet.
                let _ = (policy, minimum_fill, legs);
                Err(SimulationError::Unsupported)
            }
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
                        kind: CashflowKind::Adjustment,
                    },
                })?;
                Ok(vec![SimulationEvent::CashflowScheduled {
                    participant_id: *participant_id,
                    currency_id: *currency_id,
                    amount: *amount,
                }])
            }
            SimulationCommand::ApplyFine {
                participant_id,
                currency_id,
                amount,
                reason,
            } => {
                validate_reason(reason)?;
                if amount.get() <= 0 {
                    return Err(SimulationError::InvalidCommand);
                }
                context.require_participant(*participant_id)?;
                let debit = MoneyMinor::new(0)
                    .checked_sub(*amount)
                    .ok_or(SimulationError::ArithmeticOverflow)?;
                context
                    .ledger
                    .post_cash(*participant_id, *currency_id, debit)?;
                self.record_admin(actor, logical_time, reason.clone());
                Ok(vec![
                    SimulationEvent::FineApplied {
                        participant_id: *participant_id,
                        currency_id: *currency_id,
                        amount: *amount,
                        policy_version: SIMULATION_POLICY_VERSION,
                    },
                    SimulationEvent::AdministratorChangeRecorded {
                        reason: reason.clone(),
                    },
                ])
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
            SimulationCommand::ScoreIteration => self.score_iteration(context),
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
        if let EventPayload::TradeExecuted {
            instrument_id,
            listing_key,
            price,
            quantity,
            ..
        } = event
        {
            let trade = TradeRecord {
                logical_time,
                price: *price,
                quantity: *quantity,
            };
            // Validate both volume series before either mutable projection advances.
            // This avoids copying bounded trade history on the hot path.
            if let Some(listing) = listing_key {
                if listing.instrument_id != *instrument_id {
                    return Err(SimulationError::UnknownIdentity);
                }
                if let Some(view) = self.market_by_listing.get(listing) {
                    view.validate_trade(trade)?;
                }
            }
            if let Some(view) = self.market.get(instrument_id) {
                view.validate_trade(trade)?;
            }
            self.market
                .entry(*instrument_id)
                .or_default()
                .record_trade(trade)?;
            if let Some(listing) = listing_key {
                self.market_by_listing
                    .entry(*listing)
                    .or_default()
                    .record_trade(trade)?;
            }
        }
        Ok(())
    }

    fn advance(
        &mut self,
        context: &mut SimulationContext<'_>,
        steps: u32,
    ) -> Result<Vec<SimulationEvent>, SimulationError> {
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
            self.apply_scheduled(context, &action)?;
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

    fn apply_scheduled(
        &mut self,
        context: &mut SimulationContext<'_>,
        action: &ScheduledAction,
    ) -> Result<(), SimulationError> {
        match action.kind {
            ScheduledActionKind::Cashflow {
                participant_id,
                currency_id,
                amount,
                ..
            } => {
                context
                    .ledger
                    .post_cash(participant_id, currency_id, amount)?;
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
                context.ledger.convert(
                    job.participant_id,
                    facility
                        .input_instrument
                        .map(|instrument| (instrument, job.input_quantity)),
                    facility
                        .output_instrument
                        .map(|instrument| (instrument, job.output_quantity)),
                )?;
                self.facility_jobs
                    .get_mut(&job_id)
                    .ok_or(SimulationError::UnknownIdentity)?
                    .completed = true;
            }
            ScheduledActionKind::ExpireInstrument { instrument_id } => {
                self.halted_instruments.insert(instrument_id);
            }
            ScheduledActionKind::ExerciseOption { .. } | ScheduledActionKind::Deliver { .. } => {
                return Err(SimulationError::Unsupported);
            }
        }
        Ok(())
    }

    /// Freezes one deterministic NLV ranking of every enrolled participant at
    /// the committed instrument marks, in the reporting currency.
    fn score_iteration(
        &mut self,
        context: &SimulationContext<'_>,
    ) -> Result<Vec<SimulationEvent>, SimulationError> {
        let mut entries = context
            .participants
            .keys()
            .map(|participant_id| {
                context
                    .ledger
                    .reporting_value(
                        *participant_id,
                        context.reporting_currency,
                        context.fx_rates,
                    )
                    .map(|score| ScoreEntry {
                        participant_id: *participant_id,
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
    Unsupported,
    Ledger(LedgerError),
}

impl From<LedgerError> for SimulationError {
    fn from(error: LedgerError) -> Self {
        Self::Ledger(error)
    }
}
