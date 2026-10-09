#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Authoritative sans-I/O Bunting market-simulation engine.

mod book;
pub mod simulation;

pub use book::BookOrder;
use book::{Book, Match};
use bunting_ledger::{Fill, FillParty, LedgerError};
pub use bunting_ledger::{FxRate, InstrumentTerms, Ledger, Reservation};
use bunting_market_events::{
    CancelReason, Command, CommandPayload, EVENT_SCHEMA_VERSION, EventEnvelope, EventPayload,
    OrderKind, RejectCode, Side, SimulationCommand, SimulationCommandRequest, TimeInForcePolicy,
};
use bunting_market_types::{
    CurrencyId, EventId, EventSequence, InstrumentId, IterationId, ListingKey, LogicalTimeNs,
    MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId,
    ScenarioVersion,
};
use bunting_risk_engine::RiskLimits;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use simulation::InstrumentKind;
use simulation::{
    SIMULATION_POLICY_VERSION, SimulationContext, SimulationError, SimulationScenario,
    SimulationState,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

/// Version of the central engine behavior established by this foundation slice.
pub const ENGINE_VERSION: u16 = 5;
/// Version of the complete persisted engine snapshot envelope.
pub const ENGINE_SNAPSHOT_VERSION: u16 = 5;
/// Version of the Bunting-native scenario schema.
pub const SCENARIO_SCHEMA_VERSION: u16 = 2;
/// Maximum economic instruments admitted into one run.
pub const MAX_INSTRUMENTS: usize = 256;
/// Maximum listings admitted into one foundation run.
pub const MAX_LISTINGS: usize = 64;
/// Maximum participants admitted into one foundation run.
pub const MAX_PARTICIPANTS: usize = 1_024;
/// Maximum simultaneously live orders across every listing of a run.
pub const MAX_LIVE_ORDERS: usize = 250_000;
/// Recently terminal orders retained for late cancels and duplicate detection.
pub const MAX_RETIRED_ORDERS: usize = 65_536;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Visible price and quantity levels for one side of a listing.
pub type VisibleLevels = Vec<(PriceTicks, QuantityLots)>;
/// Visible bid and ask levels for one listing.
pub type VisibleDepth = (VisibleLevels, VisibleLevels);

/// Stable foundation engine configuration.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineConfig {
    /// Versioned engine behavior.
    pub engine_version: u16,
    /// Maximum listing count for this run.
    pub max_listings: u16,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            engine_version: ENGINE_VERSION,
            max_listings: u16::try_from(MAX_LISTINGS).unwrap_or(u16::MAX),
        }
    }
}

/// Per-lot fee schedule of one listing, charged in the instrument currency.
///
/// RIT's `TradingFee` maps to `taker_per_lot` and `LimitOrderRebate` maps to a
/// negative `maker_per_lot`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FeeSchedule {
    /// Charged per filled lot to the aggressing order; negative is a rebate.
    pub taker_per_lot: MoneyMinor,
    /// Charged per filled lot to the resting order; negative is a rebate.
    pub maker_per_lot: MoneyMinor,
}

impl FeeSchedule {
    /// Largest per-lot charge either role can incur, used for buy reservations.
    #[must_use]
    pub fn bound(self) -> MoneyMinor {
        MoneyMinor::new(
            self.taker_per_lot
                .get()
                .max(self.maker_per_lot.get())
                .max(0),
        )
    }
}

/// Immutable venue-specific listing input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingDefinition {
    key: ListingKey,
    symbol: String,
    price_bounds: PriceBounds,
    #[serde(default)]
    fees: FeeSchedule,
}

impl ListingDefinition {
    pub fn new(
        key: ListingKey,
        symbol: String,
        price_bounds: PriceBounds,
    ) -> Result<Self, ScenarioError> {
        if key.venue_id.get() == 0
            || key.instrument_id.get() == 0
            || symbol.is_empty()
            || symbol.len() > 128
        {
            return Err(ScenarioError::InvalidListing);
        }
        Ok(Self {
            key,
            symbol,
            price_bounds,
            fees: FeeSchedule::default(),
        })
    }

    /// Replaces the listing fee schedule.
    #[must_use]
    pub const fn with_fees(mut self, fees: FeeSchedule) -> Self {
        self.fees = fees;
        self
    }

    #[must_use]
    pub const fn key(&self) -> ListingKey {
        self.key
    }

    #[must_use]
    pub fn symbol(&self) -> &str {
        &self.symbol
    }

    #[must_use]
    pub const fn price_bounds(&self) -> PriceBounds {
        self.price_bounds
    }

    #[must_use]
    pub const fn fees(&self) -> FeeSchedule {
        self.fees
    }
}

/// Immutable economic definition shared by every listing of one instrument.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentDefinition {
    pub instrument_id: InstrumentId,
    pub symbol: String,
    /// Settlement and quotation currency.
    pub currency: CurrencyId,
    pub kind: InstrumentKind,
    /// Positive contract multiplier applied to price-times-quantity notional.
    pub multiplier: i64,
    /// Whether positions may become negative without borrowed inventory.
    #[serde(default)]
    pub shortable: bool,
    /// Opening valuation mark; required when any participant starts with a position.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opening_mark: Option<PriceTicks>,
}

impl InstrumentDefinition {
    /// Unit-multiplier, non-shortable instrument without an opening mark.
    #[must_use]
    pub fn new(
        instrument_id: InstrumentId,
        symbol: impl Into<String>,
        currency: CurrencyId,
        kind: InstrumentKind,
    ) -> Self {
        Self {
            instrument_id,
            symbol: symbol.into(),
            currency,
            kind,
            multiplier: 1,
            shortable: false,
            opening_mark: None,
        }
    }

    #[must_use]
    pub const fn with_multiplier(mut self, multiplier: i64) -> Self {
        self.multiplier = multiplier;
        self
    }

    #[must_use]
    pub const fn shortable(mut self) -> Self {
        self.shortable = true;
        self
    }

    #[must_use]
    pub const fn with_opening_mark(mut self, mark: PriceTicks) -> Self {
        self.opening_mark = Some(mark);
        self
    }

    /// Ledger terms derived from this definition.
    #[must_use]
    pub const fn terms(&self) -> InstrumentTerms {
        InstrumentTerms {
            currency: self.currency,
            multiplier: self.multiplier,
            shortable: self.shortable,
        }
    }

    fn validate(&self) -> Result<(), ScenarioError> {
        if self.instrument_id.get() == 0
            || self.symbol.is_empty()
            || self.symbol.len() > 128
            || self.currency.get() == 0
            || self.multiplier <= 0
            || self.opening_mark.is_some_and(|mark| mark.get() <= 0)
        {
            return Err(ScenarioError::InvalidInstrument);
        }
        Ok(())
    }
}

/// Immutable participant input required by the run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantDefinition {
    participant_id: ParticipantId,
    enabled: bool,
    limits: RiskLimits,
    /// Canonically ordered opening cash per currency.
    initial_cash: BTreeMap<CurrencyId, MoneyMinor>,
    /// Canonically ordered initial positions.
    initial_positions: BTreeMap<InstrumentId, QuantityLots>,
}

impl ParticipantDefinition {
    #[must_use]
    pub const fn new(
        participant_id: ParticipantId,
        enabled: bool,
        limits: RiskLimits,
        initial_cash: BTreeMap<CurrencyId, MoneyMinor>,
        initial_positions: BTreeMap<InstrumentId, QuantityLots>,
    ) -> Self {
        Self {
            participant_id,
            enabled,
            limits,
            initial_cash,
            initial_positions,
        }
    }

    #[must_use]
    pub const fn participant_id(&self) -> ParticipantId {
        self.participant_id
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn limits(&self) -> RiskLimits {
        self.limits
    }

    #[must_use]
    pub fn initial_cash(&self) -> &BTreeMap<CurrencyId, MoneyMinor> {
        &self.initial_cash
    }

    #[must_use]
    pub fn initial_positions(&self) -> &BTreeMap<InstrumentId, QuantityLots> {
        &self.initial_positions
    }
}

mod instrument_list {
    use super::{InstrumentDefinition, InstrumentId};
    use serde::{Deserialize, Deserializer, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<S>(
        value: &BTreeMap<InstrumentId, InstrumentDefinition>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_seq(value.values())
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<InstrumentId, InstrumentDefinition>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut output = BTreeMap::new();
        for value in Vec::<InstrumentDefinition>::deserialize(deserializer)? {
            if output.insert(value.instrument_id, value).is_some() {
                return Err(serde::de::Error::custom("duplicate instrument"));
            }
        }
        Ok(output)
    }
}

/// Immutable scenario definition used to instantiate a run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDefinition {
    schema_version: u16,
    scenario_id: ScenarioId,
    scenario_version: ScenarioVersion,
    /// Currency in which net liquidation value and scores are reported.
    reporting_currency: CurrencyId,
    /// Conversions for every other held currency.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fx_rates: Vec<FxRate>,
    /// Economic instruments; every listed instrument must be defined.
    #[serde(with = "instrument_list")]
    instruments: BTreeMap<InstrumentId, InstrumentDefinition>,
    /// Canonically ordered by listing identity.
    listings: BTreeMap<ListingKey, ListingDefinition>,
    /// Canonically ordered by participant identity.
    participants: BTreeMap<ParticipantId, ParticipantDefinition>,
    #[serde(default)]
    simulation: SimulationScenario,
}

impl ScenarioDefinition {
    /// Builds a validated scenario. The reporting currency defaults to the
    /// lowest instrument currency; use [`Self::with_reporting`] for others.
    pub fn new(
        scenario_id: ScenarioId,
        scenario_version: ScenarioVersion,
        instruments: impl IntoIterator<Item = InstrumentDefinition>,
        listings: impl IntoIterator<Item = ListingDefinition>,
        participants: impl IntoIterator<Item = ParticipantDefinition>,
    ) -> Result<Self, ScenarioError> {
        let mut instrument_map = BTreeMap::new();
        for instrument in instruments {
            if instrument_map
                .insert(instrument.instrument_id, instrument)
                .is_some()
            {
                return Err(ScenarioError::InvalidInstrument);
            }
        }
        let mut listing_map = BTreeMap::new();
        for listing in listings {
            let key = listing.key;
            if listing_map.insert(key, listing).is_some() {
                return Err(ScenarioError::DuplicateListing);
            }
        }
        let mut participant_map = BTreeMap::new();
        for participant in participants {
            let participant_id = participant.participant_id;
            if participant_map
                .insert(participant_id, participant)
                .is_some()
            {
                return Err(ScenarioError::DuplicateParticipant);
            }
        }
        let reporting_currency = instrument_map
            .values()
            .map(|instrument| instrument.currency)
            .min()
            .ok_or(ScenarioError::InvalidInstrument)?;
        let definition = Self {
            schema_version: SCENARIO_SCHEMA_VERSION,
            scenario_id,
            scenario_version,
            reporting_currency,
            fx_rates: Vec::new(),
            instruments: instrument_map,
            listings: listing_map,
            participants: participant_map,
            simulation: SimulationScenario::default(),
        };
        definition.validate()?;
        Ok(definition)
    }

    /// Sets the reporting currency and the conversions for other currencies.
    pub fn with_reporting(
        mut self,
        reporting_currency: CurrencyId,
        fx_rates: Vec<FxRate>,
    ) -> Result<Self, ScenarioError> {
        self.reporting_currency = reporting_currency;
        self.fx_rates = fx_rates;
        self.validate()?;
        Ok(self)
    }

    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.schema_version != SCENARIO_SCHEMA_VERSION {
            return Err(ScenarioError::UnsupportedSchemaVersion);
        }
        if self.scenario_id.get() == 0 || self.scenario_version.get() == 0 {
            return Err(ScenarioError::InvalidScenarioIdentity);
        }
        if self.listings.is_empty() || self.listings.len() > MAX_LISTINGS {
            return Err(ScenarioError::ListingBound);
        }
        if self.instruments.is_empty() || self.instruments.len() > MAX_INSTRUMENTS {
            return Err(ScenarioError::InvalidInstrument);
        }
        if self.participants.len() > MAX_PARTICIPANTS {
            return Err(ScenarioError::ParticipantBound);
        }
        for (id, instrument) in &self.instruments {
            if *id != instrument.instrument_id {
                return Err(ScenarioError::InvalidInstrument);
            }
            instrument.validate()?;
        }
        let currency_supported = |currency: CurrencyId| {
            currency == self.reporting_currency
                || self.fx_rates.iter().any(|rate| rate.currency == currency)
        };
        if self.reporting_currency.get() == 0 {
            return Err(ScenarioError::InvalidCurrency);
        }
        for (index, rate) in self.fx_rates.iter().enumerate() {
            if rate.currency == self.reporting_currency
                || rate.minor_units_per_lot <= 0
                || self.fx_rates[..index]
                    .iter()
                    .any(|other| other.currency == rate.currency)
                || self
                    .instruments
                    .get(&rate.instrument_id)
                    .is_none_or(|instrument| instrument.currency != self.reporting_currency)
            {
                return Err(ScenarioError::InvalidCurrency);
            }
        }
        if self
            .instruments
            .values()
            .any(|instrument| !currency_supported(instrument.currency))
        {
            return Err(ScenarioError::InvalidCurrency);
        }
        for (key, listing) in &self.listings {
            if key.venue_id.get() == 0
                || key.instrument_id.get() == 0
                || *key != listing.key
                || listing.symbol.is_empty()
                || listing.symbol.len() > 128
                || !self.instruments.contains_key(&key.instrument_id)
            {
                return Err(ScenarioError::InvalidListing);
            }
            listing
                .price_bounds
                .validate(listing.price_bounds.min)
                .map_err(|_| ScenarioError::InvalidListing)?;
        }
        for (participant_id, participant) in &self.participants {
            if *participant_id != participant.participant_id
                || participant.initial_positions.len() > MAX_INSTRUMENTS
                || participant.initial_cash.len() > MAX_INSTRUMENTS
                || participant
                    .initial_cash
                    .keys()
                    .any(|currency| currency.get() == 0 || !currency_supported(*currency))
            {
                return Err(ScenarioError::InvalidParticipant);
            }
            for (instrument, quantity) in &participant.initial_positions {
                let definition = self
                    .instruments
                    .get(instrument)
                    .ok_or(ScenarioError::InvalidParticipant)?;
                if quantity.get() != 0 && definition.opening_mark.is_none() {
                    return Err(ScenarioError::InvalidParticipant);
                }
                if quantity.get() < 0 && !definition.shortable {
                    return Err(ScenarioError::InvalidParticipant);
                }
            }
        }
        self.simulation
            .validate()
            .map_err(|_| ScenarioError::InvalidSimulation)?;
        if self
            .simulation
            .referenced_instruments()
            .any(|instrument| !self.instruments.contains_key(&instrument))
        {
            return Err(ScenarioError::InvalidSimulation);
        }
        Ok(())
    }

    pub fn content_hash(&self) -> Result<String, SnapshotError> {
        self.validate()
            .map_err(|_| SnapshotError::InvalidScenario)?;
        hash_serializable(self)
    }

    #[must_use]
    pub fn listings(&self) -> &BTreeMap<ListingKey, ListingDefinition> {
        &self.listings
    }

    #[must_use]
    pub fn instruments(&self) -> &BTreeMap<InstrumentId, InstrumentDefinition> {
        &self.instruments
    }

    #[must_use]
    pub fn participants(&self) -> &BTreeMap<ParticipantId, ParticipantDefinition> {
        &self.participants
    }

    #[must_use]
    pub const fn reporting_currency(&self) -> CurrencyId {
        self.reporting_currency
    }

    /// Attaches validated immutable simulation-domain configuration.
    ///
    /// # Errors
    /// Returns [`ScenarioError::InvalidSimulation`] for invalid policy input.
    pub fn with_simulation(
        mut self,
        simulation: SimulationScenario,
    ) -> Result<Self, ScenarioError> {
        simulation
            .validate()
            .map_err(|_| ScenarioError::InvalidSimulation)?;
        self.simulation = simulation;
        self.validate()?;
        Ok(self)
    }

    /// Returns immutable simulation-domain configuration.
    #[must_use]
    pub const fn simulation(&self) -> &SimulationScenario {
        &self.simulation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenarioError {
    UnsupportedSchemaVersion,
    InvalidInstrument,
    InvalidCurrency,
    ListingBound,
    ParticipantBound,
    DuplicateListing,
    DuplicateParticipant,
    InvalidListing,
    InvalidParticipant,
    InvalidScenarioIdentity,
    InvalidSimulation,
}

/// Immutable published scenario record addressed by identity, version and hash.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishedScenario {
    pub definition: ScenarioDefinition,
    pub content_hash: String,
}

/// Result of idempotently publishing an immutable scenario version.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishScenarioOutcome {
    Published,
    AlreadyPublished,
}

/// Bounded scenario publication catalog; published versions are never overwritten.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioCatalog {
    records: Vec<PublishedScenario>,
}

impl ScenarioCatalog {
    /// Validates and publishes one immutable scenario version idempotently.
    ///
    /// # Errors
    /// Returns an error for invalid input, hash failure, capacity, or a conflicting version.
    pub fn publish(
        &mut self,
        definition: ScenarioDefinition,
    ) -> Result<PublishScenarioOutcome, ScenarioCatalogError> {
        definition
            .validate()
            .map_err(|_| ScenarioCatalogError::InvalidScenario)?;
        let content_hash = definition
            .content_hash()
            .map_err(|_| ScenarioCatalogError::Hash)?;
        if let Some(existing) = self.records.iter().find(|record| {
            record.definition.scenario_id == definition.scenario_id
                && record.definition.scenario_version == definition.scenario_version
        }) {
            return if existing.content_hash == content_hash {
                Ok(PublishScenarioOutcome::AlreadyPublished)
            } else {
                Err(ScenarioCatalogError::VersionConflict)
            };
        }
        if self.records.len() == 4_096 {
            return Err(ScenarioCatalogError::CatalogFull);
        }
        self.records.push(PublishedScenario {
            definition,
            content_hash,
        });
        self.records.sort_by_key(|record| {
            (
                record.definition.scenario_id,
                record.definition.scenario_version,
            )
        });
        Ok(PublishScenarioOutcome::Published)
    }

    /// Returns canonically ordered immutable publication records.
    #[must_use]
    pub fn list(&self) -> &[PublishedScenario] {
        &self.records
    }

    /// Resolves one exact immutable scenario version.
    #[must_use]
    pub fn get(
        &self,
        scenario_id: ScenarioId,
        scenario_version: ScenarioVersion,
    ) -> Option<&PublishedScenario> {
        self.records.iter().find(|record| {
            record.definition.scenario_id == scenario_id
                && record.definition.scenario_version == scenario_version
        })
    }
}

/// Stable scenario publication failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenarioCatalogError {
    InvalidScenario,
    Hash,
    VersionConflict,
    CatalogFull,
}

/// Persisted lifecycle state for an owned order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedOrderState {
    Active,
    Filled,
    Canceled,
}

/// Authoritative private ownership record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OwnedOrder {
    pub order_id: OrderId,
    pub participant_id: ParticipantId,
    pub listing_key: ListingKey,
    pub side: Side,
    /// Limit price, or the listing bound that caps a market order's cash use.
    pub limit_price: PriceTicks,
    pub original_quantity: QuantityLots,
    pub remaining_quantity: QuantityLots,
    pub state: OwnedOrderState,
    /// Exact ledger reservation released on fill, cancel or expiry.
    pub reservation: Reservation,
    /// Logical expiry of a GTD order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<LogicalTimeNs>,
    /// Whether the order expires at its listing's session close.
    #[serde(default)]
    pub day: bool,
}

/// Authoritative state for one venue listing, including its live book.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingState {
    definition: ListingDefinition,
    book: Book,
}

impl ListingState {
    #[must_use]
    pub const fn definition(&self) -> &ListingDefinition {
        &self.definition
    }

    /// Resting orders on one side in matching priority, best price first.
    pub fn resting(&self, side: Side) -> impl Iterator<Item = &BookOrder> + '_ {
        self.book.orders(side)
    }

    /// Displayed quantity per level, best first.
    #[must_use]
    pub fn depth(&self, side: Side) -> Vec<(PriceTicks, QuantityLots)> {
        self.book.depth(side)
    }
}

/// Complete authoritative engine state persisted by origin adapters.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunState {
    run_id: RunId,
    sequence: EventSequence,
    event_sequence: EventSequence,
    iteration_id: IterationId,
    scenario_id: ScenarioId,
    scenario_version: ScenarioVersion,
    scenario_hash: String,
    config: EngineConfig,
    reporting_currency: CurrencyId,
    fx_rates: Vec<FxRate>,
    #[serde(with = "instrument_list")]
    instruments: BTreeMap<InstrumentId, InstrumentDefinition>,
    listings: BTreeMap<ListingKey, ListingState>,
    participants: BTreeMap<ParticipantId, ParticipantDefinition>,
    /// The single authoritative economic ledger.
    ledger: Ledger,
    /// Live orders plus a bounded window of recently terminal orders.
    ownership: BTreeMap<OrderId, OwnedOrder>,
    /// Terminal orders in retirement order; the oldest leave `ownership` first.
    retired: VecDeque<OrderId>,
    /// GTD expiries on the logical clock.
    expiries: BTreeSet<(LogicalTimeNs, OrderId)>,
    /// Live-order count per participant, maintained where orders become live
    /// and in `retire`. Participants with no live orders have no entry.
    live_orders: BTreeMap<ParticipantId, u32>,
    kill_switch: bool,
    #[serde(default)]
    simulation: SimulationState,
}

impl RunState {
    pub fn from_scenario(
        run_id: RunId,
        iteration_id: IterationId,
        scenario: &ScenarioDefinition,
    ) -> Result<Self, EngineError> {
        scenario
            .validate()
            .map_err(|_| EngineError::InvalidScenario)?;
        if iteration_id.get() == 0 {
            return Err(EngineError::InvalidScenario);
        }
        let listings = scenario
            .listings
            .iter()
            .map(|(key, definition)| {
                (
                    *key,
                    ListingState {
                        definition: definition.clone(),
                        book: Book::new(),
                    },
                )
            })
            .collect();
        let mut ledger = Ledger::new();
        for (instrument_id, instrument) in &scenario.instruments {
            ledger.configure_instrument(*instrument_id, instrument.terms())?;
            if let Some(mark) = instrument.opening_mark {
                ledger.set_mark(*instrument_id, mark)?;
            }
        }
        for participant in scenario.participants.values() {
            for (currency, amount) in &participant.initial_cash {
                ledger.post_cash(participant.participant_id, *currency, *amount)?;
            }
            for (instrument_id, quantity) in &participant.initial_positions {
                let instrument = &scenario.instruments[instrument_id];
                let basis = instrument
                    .opening_mark
                    .map_or(Ok(MoneyMinor::new(0)), |mark| {
                        bunting_ledger::notional(mark, *quantity, instrument.multiplier)
                    })?;
                ledger.seed_position(
                    participant.participant_id,
                    *instrument_id,
                    *quantity,
                    basis,
                )?;
            }
        }
        let simulation = SimulationState::from_scenario(&scenario.simulation)
            .map_err(|_| EngineError::InvalidScenario)?;
        Ok(Self {
            run_id,
            sequence: EventSequence::new(0),
            event_sequence: EventSequence::new(0),
            iteration_id,
            scenario_id: scenario.scenario_id,
            scenario_version: scenario.scenario_version,
            scenario_hash: scenario.content_hash()?,
            config: EngineConfig::default(),
            reporting_currency: scenario.reporting_currency,
            fx_rates: scenario.fx_rates.clone(),
            instruments: scenario.instruments.clone(),
            listings,
            participants: scenario.participants.clone(),
            ledger,
            ownership: BTreeMap::new(),
            retired: VecDeque::new(),
            expiries: BTreeSet::new(),
            live_orders: BTreeMap::new(),
            kill_switch: false,
            simulation,
        })
    }

    /// Deterministically resets this run to a new iteration of its pinned scenario.
    ///
    /// # Errors
    /// Returns an error when the supplied scenario is not the exact pinned version/hash.
    pub fn reset_iteration(
        &self,
        iteration_id: IterationId,
        scenario: &ScenarioDefinition,
    ) -> Result<Self, EngineError> {
        if scenario.scenario_id != self.scenario_id
            || scenario.scenario_version != self.scenario_version
            || scenario.content_hash()? != self.scenario_hash
        {
            return Err(EngineError::InvalidScenario);
        }
        Self::from_scenario(self.run_id, iteration_id, scenario)
    }

    #[must_use]
    pub const fn run_id(&self) -> RunId {
        self.run_id
    }

    #[must_use]
    pub const fn sequence(&self) -> EventSequence {
        self.sequence
    }

    #[must_use]
    pub const fn event_sequence(&self) -> EventSequence {
        self.event_sequence
    }

    #[must_use]
    pub const fn scenario_id(&self) -> ScenarioId {
        self.scenario_id
    }

    #[must_use]
    pub const fn scenario_version(&self) -> ScenarioVersion {
        self.scenario_version
    }

    #[must_use]
    pub fn scenario_hash(&self) -> &str {
        &self.scenario_hash
    }

    #[must_use]
    pub const fn config(&self) -> EngineConfig {
        self.config
    }

    #[must_use]
    pub fn listings(&self) -> &BTreeMap<ListingKey, ListingState> {
        &self.listings
    }

    /// The single authoritative economic ledger.
    #[must_use]
    pub const fn ledger(&self) -> &Ledger {
        &self.ledger
    }

    #[must_use]
    pub fn instruments(&self) -> &BTreeMap<InstrumentId, InstrumentDefinition> {
        &self.instruments
    }

    #[must_use]
    pub const fn reporting_currency(&self) -> CurrencyId {
        self.reporting_currency
    }

    #[must_use]
    pub fn fx_rates(&self) -> &[FxRate] {
        &self.fx_rates
    }

    #[must_use]
    pub const fn kill_switch_active(&self) -> bool {
        self.kill_switch
    }

    /// Net liquidation value of one participant in the reporting currency.
    pub fn net_liquidation_value(
        &self,
        participant: ParticipantId,
    ) -> Result<MoneyMinor, EngineError> {
        Ok(self
            .ledger
            .reporting_value(participant, self.reporting_currency, &self.fx_rates)?)
    }

    /// Returns immutable participant configuration pinned by the run scenario.
    #[must_use]
    pub fn participants(&self) -> &BTreeMap<ParticipantId, ParticipantDefinition> {
        &self.participants
    }

    /// Live orders and the bounded window of recently terminal orders.
    #[must_use]
    pub fn ownership(&self) -> &BTreeMap<OrderId, OwnedOrder> {
        &self.ownership
    }

    /// Committed number of live orders for one participant (O(log n)).
    #[must_use]
    pub fn live_order_count(&self, participant: ParticipantId) -> u32 {
        self.live_orders.get(&participant).copied().unwrap_or(0)
    }

    /// Live orders of one participant in identity order.
    pub fn live_orders(
        &self,
        participant: ParticipantId,
    ) -> impl Iterator<Item = &OwnedOrder> + '_ {
        self.ownership.values().filter(move |owned| {
            owned.participant_id == participant && owned.state == OwnedOrderState::Active
        })
    }

    /// Returns the complete authoritative simulation component.
    #[must_use]
    pub const fn simulation(&self) -> &SimulationState {
        &self.simulation
    }

    pub fn listing_key_for_instrument(
        &self,
        instrument_id: InstrumentId,
    ) -> Result<ListingKey, EngineError> {
        let mut keys = self
            .listings
            .keys()
            .copied()
            .filter(|key| key.instrument_id == instrument_id);
        let key = keys.next().ok_or(EngineError::UnknownListing)?;
        if keys.next().is_some() {
            return Err(EngineError::AmbiguousListing);
        }
        Ok(key)
    }

    /// Displayed bid and ask levels of one listing, best first.
    pub fn visible_levels(&self, key: ListingKey) -> Result<VisibleDepth, EngineError> {
        let listing = self.listings.get(&key).ok_or(EngineError::UnknownListing)?;
        Ok((listing.depth(Side::Buy), listing.depth(Side::Sell)))
    }

    pub fn state_hash(&self) -> Result<String, SnapshotError> {
        hash_serializable(self)
    }

    fn validate(&self) -> Result<(), SnapshotError> {
        let live = self
            .ownership
            .values()
            .filter(|owned| owned.state == OwnedOrderState::Active)
            .count();
        let mut live_by_participant: BTreeMap<ParticipantId, u32> = BTreeMap::new();
        for owned in self.ownership.values() {
            if owned.state == OwnedOrderState::Active {
                let count = live_by_participant.entry(owned.participant_id).or_insert(0);
                *count = count
                    .checked_add(1)
                    .ok_or(SnapshotError::UnsupportedVersion)?;
            }
        }
        let resting = self
            .listings
            .values()
            .map(|listing| listing.book.len())
            .sum::<usize>();
        if self.config.engine_version != ENGINE_VERSION
            || self.config.max_listings == 0
            || usize::from(self.config.max_listings) > MAX_LISTINGS
            || self.listings.is_empty()
            || self.listings.len() > usize::from(self.config.max_listings)
            || self.participants.len() > MAX_PARTICIPANTS
            || live > MAX_LIVE_ORDERS
            || self.retired.len() > MAX_RETIRED_ORDERS
            || live != resting
            || live_by_participant != self.live_orders
            || live + self.retired.len() != self.ownership.len()
            || self.ownership.iter().any(|(order_id, owned)| {
                *order_id != owned.order_id
                    || (owned.state == OwnedOrderState::Active)
                        != self
                            .listings
                            .get(&owned.listing_key)
                            .is_some_and(|listing| listing.book.contains(*order_id))
            })
            || self.expiries.iter().any(|(at, order_id)| {
                self.ownership.get(order_id).is_none_or(|owned| {
                    owned.state != OwnedOrderState::Active || owned.expires_at != Some(*at)
                })
            })
            || self.simulation.policy_version != SIMULATION_POLICY_VERSION
            || self.ledger.instruments().len() != self.instruments.len()
            || self
                .instruments
                .iter()
                .any(|(id, definition)| self.ledger.terms(*id) != Ok(definition.terms()))
        {
            return Err(SnapshotError::UnsupportedVersion);
        }
        Ok(())
    }

    pub fn snapshot_envelope(&self) -> Result<EngineSnapshotEnvelope, SnapshotError> {
        EngineSnapshotEnvelope::new(self.clone())
    }

    /// Immutable transition convenience. Use `transition_owned` when a
    /// caller already owns the loaded run.
    pub fn transition(&self, command: &Command) -> Result<TransitionOutcome, EngineError> {
        self.clone().transition_owned(command)
    }

    /// Consuming transition: the outcome carries the new state. An error
    /// drops the state, so callers that must keep it use [`Self::apply`].
    pub fn transition_owned(self, command: &Command) -> Result<TransitionOutcome, EngineError> {
        let mut candidate = self;
        let applied = candidate.apply(command).map_err(ApplyError::into_inner)?;
        Ok(applied.into_outcome(candidate))
    }

    /// Applies one command to this state in place.
    ///
    /// Contract: [`ApplyError::Unchanged`] is returned only before the first
    /// mutation, so the state is exactly as it was. [`ApplyError::Poisoned`]
    /// means a mid-transition invariant failed after mutation began; the
    /// caller must discard this value and rebuild it from committed state.
    pub fn apply(&mut self, command: &Command) -> Result<Applied, ApplyError> {
        let next_sequence = self
            .preflight(command.run_id, command.expected_sequence)
            .map_err(ApplyError::Unchanged)?;
        // Listing resolution depends only on immutable listings, so resolving
        // it before expiries are processed cannot change any outcome.
        let listing_key = match &command.payload {
            CommandPayload::SubmitOrderAtListing { order, listing_key } => {
                if listing_key.instrument_id != order.instrument_id
                    || !self.listings.contains_key(listing_key)
                {
                    return Err(ApplyError::Unchanged(EngineError::UnknownListing));
                }
                Some(*listing_key)
            }
            CommandPayload::SubmitOrder(order) => Some(
                self.listing_key_for_instrument(order.instrument_id)
                    .map_err(ApplyError::Unchanged)?,
            ),
            CommandPayload::CancelOrder(_) | CommandPayload::ActivateKillSwitch => None,
        };
        self.apply_validated(command, next_sequence, listing_key)
            .map_err(ApplyError::Poisoned)
    }

    /// Checks run identity and optimistic sequence without mutating.
    fn preflight(
        &self,
        run_id: RunId,
        expected_sequence: EventSequence,
    ) -> Result<EventSequence, EngineError> {
        if self.run_id != run_id || self.sequence != expected_sequence {
            return Err(EngineError::SequenceConflict {
                current: self.sequence,
            });
        }
        self.sequence
            .checked_add(EventSequence::new(1))
            .ok_or(EngineError::SequenceOverflow)
    }

    fn apply_validated(
        &mut self,
        command: &Command,
        next_sequence: EventSequence,
        listing_key: Option<ListingKey>,
    ) -> Result<Applied, EngineError> {
        let candidate = self;
        let mut payloads = Vec::new();
        let mut changed_listings = BTreeSet::new();
        candidate.expire_due(command.logical_time, &mut payloads, &mut changed_listings)?;
        let (accepted, reject_code, order_id) = match &command.payload {
            CommandPayload::SubmitOrder(order)
            | CommandPayload::SubmitOrderAtListing { order, .. } => {
                let listing_key = listing_key.ok_or(EngineError::UnknownListing)?;
                payloads.push(EventPayload::OrderReceived {
                    order: order.clone(),
                    listing_key: Some(listing_key),
                });
                let outcome = if candidate.simulation.lifecycle != simulation::RunLifecycle::Active
                {
                    Ok(Err(RejectCode::RunNotActive))
                } else if candidate.kill_switch {
                    Ok(Err(RejectCode::KillSwitchActive))
                } else if candidate
                    .simulation
                    .halted_instruments
                    .contains(&order.instrument_id)
                {
                    Ok(Err(RejectCode::ListingHalted))
                } else {
                    changed_listings.insert(listing_key);
                    candidate.submit(order, listing_key, command.logical_time, &mut payloads)
                }?;
                match outcome {
                    Ok(()) => (true, None, Some(order.order_id)),
                    Err(code) => {
                        payloads.push(EventPayload::OrderRejected {
                            order_id: Some(order.order_id),
                            code,
                        });
                        (false, Some(format!("{code:?}")), Some(order.order_id))
                    }
                }
            }
            CommandPayload::CancelOrder(cancel) => {
                let outcome = match candidate.ownership.get(&cancel.order_id) {
                    None => Err(RejectCode::UnknownOrder),
                    Some(owned) if owned.participant_id != cancel.participant_id => {
                        Err(RejectCode::NotOrderOwner)
                    }
                    Some(owned) if owned.state != OwnedOrderState::Active => {
                        Err(RejectCode::UnknownOrder)
                    }
                    Some(owned) => {
                        changed_listings.insert(owned.listing_key);
                        candidate.cancel(
                            cancel.order_id,
                            CancelReason::Requested,
                            &mut payloads,
                        )?;
                        Ok(())
                    }
                };
                match outcome {
                    Ok(()) => (true, None, Some(cancel.order_id)),
                    Err(code) => {
                        payloads.push(EventPayload::OrderRejected {
                            order_id: Some(cancel.order_id),
                            code,
                        });
                        (false, Some(format!("{code:?}")), Some(cancel.order_id))
                    }
                }
            }
            CommandPayload::ActivateKillSwitch => {
                candidate.kill_switch = true;
                payloads.push(EventPayload::KillSwitchActivated);
                (true, None, None)
            }
        };
        candidate.finish(
            command,
            next_sequence,
            payloads,
            changed_listings,
            accepted,
            reject_code,
            order_id,
        )
    }

    /// Projects committed payloads, retires terminal orders and envelopes the
    /// batch as the next run sequence.
    #[expect(
        clippy::too_many_arguments,
        reason = "the shared commit tail receives every staged transition fact"
    )]
    fn finish(
        &mut self,
        command: &Command,
        next_sequence: EventSequence,
        payloads: Vec<EventPayload>,
        changed_listings: BTreeSet<ListingKey>,
        accepted: bool,
        reject_code: Option<String>,
        order_id: Option<OrderId>,
    ) -> Result<Applied, EngineError> {
        for payload in &payloads {
            self.simulation
                .project_event(command.logical_time, payload)
                .map_err(EngineError::Simulation)?;
            match payload {
                EventPayload::OrderCompleted { order_id }
                | EventPayload::OrderCanceled { order_id, .. } => self.retire(*order_id),
                _ => {}
            }
        }
        let events = envelope(command, self.event_sequence, payloads)?;
        self.sequence = next_sequence;
        self.event_sequence = events
            .last()
            .map_or(self.event_sequence, |event| event.sequence);
        Ok(Applied {
            events,
            accepted,
            reject_code,
            order_id,
            changed_listings,
        })
    }

    /// Bounds retained terminal ownership. Order identifiers are namespaced by
    /// the admission layer, so the retained window only serves late cancels and
    /// duplicate submissions racing a terminal order.
    fn retire(&mut self, order_id: OrderId) {
        if let Some(owner) = self
            .ownership
            .get(&order_id)
            .map(|owned| owned.participant_id)
            && let Some(count) = self.live_orders.get_mut(&owner)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.live_orders.remove(&owner);
            }
        }
        self.retired.push_back(order_id);
        while self.retired.len() > MAX_RETIRED_ORDERS {
            if let Some(oldest) = self.retired.pop_front() {
                self.ownership.remove(&oldest);
            }
        }
    }

    /// Cancels every GTD order whose expiry is at or before `now`.
    fn expire_due(
        &mut self,
        now: LogicalTimeNs,
        payloads: &mut Vec<EventPayload>,
        changed_listings: &mut BTreeSet<ListingKey>,
    ) -> Result<(), EngineError> {
        while let Some(&(at, order_id)) = self.expiries.first() {
            if at > now {
                break;
            }
            let listing = self
                .ownership
                .get(&order_id)
                .map(|owned| owned.listing_key)
                .ok_or(EngineError::OwnershipInvariant)?;
            changed_listings.insert(listing);
            self.cancel(order_id, CancelReason::Expired, payloads)?;
        }
        Ok(())
    }

    /// Admits, matches and rests or cancels one order.
    #[expect(
        clippy::too_many_lines,
        reason = "submission keeps admission, matching, settlement and remainder handling in one staged path"
    )]
    fn submit(
        &mut self,
        order: &bunting_market_events::SubmitOrder,
        listing_key: ListingKey,
        now: LogicalTimeNs,
        payloads: &mut Vec<EventPayload>,
    ) -> Result<Result<(), RejectCode>, EngineError> {
        if order.order_id.get() == 0 {
            return Ok(Err(RejectCode::InvalidOrderId));
        }
        if self.ownership.contains_key(&order.order_id) {
            return Ok(Err(RejectCode::DuplicateOrderId));
        }
        if self.ownership.len() - self.retired.len() >= MAX_LIVE_ORDERS {
            return Ok(Err(RejectCode::MaxOpenOrderQuantity));
        }
        if order.quantity.get() <= 0 {
            return Ok(Err(RejectCode::InvalidQuantity));
        }
        let Some(participant) = self.participants.get(&order.participant_id) else {
            return Ok(Err(RejectCode::ParticipantDisabled));
        };
        let listing = &self.listings[&listing_key];
        let definition = &listing.definition;
        let (limit, time_in_force, post_only, display) = match order.kind {
            OrderKind::Limit { price } => (Some(price), TimeInForcePolicy::Gtc, false, None),
            OrderKind::Market => (None, TimeInForcePolicy::Ioc, false, None),
            OrderKind::LimitWithPolicy {
                price,
                time_in_force,
                post_only,
                display_quantity,
            } => (Some(price), time_in_force, post_only, display_quantity),
        };
        let expires_at = match time_in_force {
            TimeInForcePolicy::Gtd { expires_at } if expires_at <= now => {
                return Ok(Err(RejectCode::InvalidTimeInForce));
            }
            TimeInForcePolicy::Gtd { expires_at } => Some(expires_at),
            _ => None,
        };
        let immediate = matches!(
            time_in_force,
            TimeInForcePolicy::Ioc | TimeInForcePolicy::Fok
        );
        if display.is_some_and(|peak| peak.get() <= 0 || peak > order.quantity)
            || (immediate && (post_only || display.is_some()))
        {
            return Ok(Err(RejectCode::InvalidTimeInForce));
        }
        if limit.is_none() && listing.book.best(order.side.opposite()).is_none() {
            return Ok(Err(RejectCode::InsufficientLiquidity));
        }
        if let Some(price) = limit
            && post_only
            && listing.book.would_cross(order.side, price)
        {
            return Ok(Err(RejectCode::PostOnlyWouldCross));
        }
        let market_bound = match order.side {
            Side::Buy => definition.price_bounds.max,
            Side::Sell => definition.price_bounds.min,
        };
        let admission = match bunting_risk_engine::admit(
            &participant.limits,
            participant.enabled,
            order,
            bunting_risk_engine::ListingTerms {
                price_bounds: definition.price_bounds,
                fee_bound: definition.fees.bound(),
            },
            &self.ledger,
            limit.is_none().then_some(market_bound),
        ) {
            Ok(admission) => admission,
            Err(code) => return Ok(Err(code)),
        };
        // IOC, FOK and market orders never rest, so they do not consume a
        // live-order slot; they are counted and retired within this transition.
        if !immediate
            && let Err(code) = bunting_risk_engine::admit_live_order_count(
                &participant.limits,
                self.live_order_count(order.participant_id),
            )
        {
            return Ok(Err(code));
        }
        self.ledger.open_order(
            order.participant_id,
            order.instrument_id,
            order.side,
            order.quantity,
            admission.reservation,
        )?;
        self.ownership.insert(
            order.order_id,
            OwnedOrder {
                order_id: order.order_id,
                participant_id: order.participant_id,
                listing_key,
                side: order.side,
                limit_price: admission.reservation_price,
                original_quantity: order.quantity,
                remaining_quantity: order.quantity,
                state: OwnedOrderState::Active,
                reservation: admission.reservation,
                expires_at,
                day: time_in_force == TimeInForcePolicy::Day,
            },
        );
        let count = self.live_orders.entry(order.participant_id).or_insert(0);
        *count = count
            .checked_add(1)
            .ok_or(EngineError::OwnershipInvariant)?;
        payloads.push(EventPayload::OrderAccepted {
            order_id: order.order_id,
        });
        let fillable = time_in_force != TimeInForcePolicy::Fok
            || self.listings[&listing_key]
                .book
                .executable(order.side, limit)
                >= order.quantity;
        let remaining = if fillable {
            let (matches, remaining) = self
                .listings
                .get_mut(&listing_key)
                .ok_or(EngineError::UnknownListing)?
                .book
                .execute(order.side, limit, order.quantity)
                .map_err(|_| EngineError::OwnershipInvariant)?;
            self.settle_matches(order.order_id, listing_key, &matches, payloads)?;
            remaining
        } else {
            order.quantity
        };
        if remaining.get() == 0 {
            payloads.push(EventPayload::OrderCompleted {
                order_id: order.order_id,
            });
        } else if let (Some(price), false) = (limit, immediate) {
            self.listings
                .get_mut(&listing_key)
                .ok_or(EngineError::UnknownListing)?
                .book
                .rest(order.order_id, order.side, price, remaining, display)
                .map_err(|_| EngineError::OwnershipInvariant)?;
            if let Some(at) = expires_at {
                self.expiries.insert((at, order.order_id));
            }
            payloads.push(EventPayload::OrderRested {
                order_id: order.order_id,
                participant_id: order.participant_id,
                instrument_id: order.instrument_id,
                listing_key: Some(listing_key),
                side: order.side,
                price,
                remaining,
            });
        } else {
            self.ledger.close_order(
                order.participant_id,
                order.instrument_id,
                order.side,
                remaining,
                admission.reservation,
            )?;
            let owned = self
                .ownership
                .get_mut(&order.order_id)
                .ok_or(EngineError::OwnershipInvariant)?;
            owned.state = OwnedOrderState::Canceled;
            owned.remaining_quantity = QuantityLots::new(0);
            payloads.push(EventPayload::OrderCanceled {
                order_id: order.order_id,
                participant_id: order.participant_id,
                instrument_id: order.instrument_id,
                listing_key: Some(listing_key),
                remaining,
                reason: if limit.is_none() {
                    CancelReason::MarketRemainder
                } else {
                    CancelReason::Expired
                },
            });
        }
        Ok(Ok(()))
    }

    /// Settles every book match of one aggressor through the ledger.
    fn settle_matches(
        &mut self,
        taker_id: OrderId,
        listing_key: ListingKey,
        matches: &[Match],
        payloads: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let fees = self.listings[&listing_key].definition.fees;
        let funded = |participants: &BTreeMap<ParticipantId, ParticipantDefinition>,
                      participant: ParticipantId| {
            participants
                .get(&participant)
                .is_some_and(|definition| definition.limits.cash_constrained)
        };
        for fill in matches {
            let maker = self
                .ownership
                .get(&fill.maker)
                .cloned()
                .ok_or(EngineError::OwnershipInvariant)?;
            let taker = self
                .ownership
                .get(&taker_id)
                .cloned()
                .ok_or(EngineError::OwnershipInvariant)?;
            if maker.listing_key != listing_key || maker.state != OwnedOrderState::Active {
                return Err(EngineError::OwnershipInvariant);
            }
            let maker_fee = per_lot_fee(fees.maker_per_lot, fill.quantity)?;
            let taker_fee = per_lot_fee(fees.taker_per_lot, fill.quantity)?;
            let party = |owned: &OwnedOrder, fee: MoneyMinor| FillParty {
                participant: owned.participant_id,
                fee,
                order: Some(owned.reservation),
                enforce_funding: funded(&self.participants, owned.participant_id),
            };
            let (buyer, seller) = if taker.side == Side::Buy {
                (party(&taker, taker_fee), party(&maker, maker_fee))
            } else {
                (party(&maker, maker_fee), party(&taker, taker_fee))
            };
            self.ledger.settle(Fill {
                instrument: listing_key.instrument_id,
                price: fill.price,
                quantity: fill.quantity,
                buyer: Some(buyer),
                seller: Some(seller),
                set_mark: true,
            })?;
            reduce_order(fill.maker, fill.quantity, &mut self.ownership, payloads)?;
            if fill.maker_remaining.get() == 0
                && let Some(at) = maker.expires_at
            {
                self.expiries.remove(&(at, fill.maker));
            }
            reduce_order(
                taker_id,
                fill.quantity,
                &mut self.ownership,
                &mut Vec::new(),
            )?;
            payloads.push(EventPayload::TradeExecuted {
                instrument_id: listing_key.instrument_id,
                listing_key: Some(listing_key),
                maker_order_id: fill.maker,
                taker_order_id: taker_id,
                buyer_id: buyer.participant,
                seller_id: seller.participant,
                price: fill.price,
                quantity: fill.quantity,
                buyer_fee: buyer.fee,
                seller_fee: seller.fee,
            });
        }
        Ok(())
    }

    /// Removes a live order from its book and releases its reservation.
    fn cancel(
        &mut self,
        order_id: OrderId,
        reason: CancelReason,
        payloads: &mut Vec<EventPayload>,
    ) -> Result<(), EngineError> {
        let owned = self
            .ownership
            .get(&order_id)
            .cloned()
            .filter(|owned| owned.state == OwnedOrderState::Active)
            .ok_or(EngineError::OwnershipInvariant)?;
        let resting = self
            .listings
            .get_mut(&owned.listing_key)
            .ok_or(EngineError::UnknownListing)?
            .book
            .cancel(order_id)
            .ok_or(EngineError::OwnershipInvariant)?;
        if resting.remaining != owned.remaining_quantity {
            return Err(EngineError::OwnershipInvariant);
        }
        self.ledger.close_order(
            owned.participant_id,
            owned.listing_key.instrument_id,
            owned.side,
            owned.remaining_quantity,
            owned.reservation,
        )?;
        if let Some(at) = owned.expires_at {
            self.expiries.remove(&(at, order_id));
        }
        if let Some(record) = self.ownership.get_mut(&order_id) {
            record.state = OwnedOrderState::Canceled;
            record.remaining_quantity = QuantityLots::new(0);
        }
        payloads.push(EventPayload::OrderCanceled {
            order_id,
            participant_id: owned.participant_id,
            instrument_id: owned.listing_key.instrument_id,
            listing_key: Some(owned.listing_key),
            remaining: owned.remaining_quantity,
            reason,
        });
        Ok(())
    }

    /// Applies one authoritative simulation-administration command atomically.
    /// Immutable simulation transition convenience; callers already owning a
    /// recovered run may use `transition_simulation_owned` without another copy.
    pub fn transition_simulation(
        &self,
        request: &SimulationCommandRequest,
    ) -> Result<TransitionOutcome, EngineError> {
        self.clone().transition_simulation_owned(request)
    }

    /// Consuming simulation transition; an error drops the state.
    pub fn transition_simulation_owned(
        self,
        request: &SimulationCommandRequest,
    ) -> Result<TransitionOutcome, EngineError> {
        let mut candidate = self;
        let next_sequence = candidate.preflight(request.run_id, request.expected_sequence)?;
        let applied = candidate.apply_simulation_validated(request, next_sequence)?;
        Ok(applied.into_outcome(candidate))
    }

    /// Applies one simulation-administration command in place, with the same
    /// contract as [`Self::apply`]. Mass cancel runs in place; its only
    /// failures are invariant violations. Every other simulation command can
    /// be refused part-way by its domain rules, so it is staged on a copy and
    /// swapped in on success. These are operator-rate commands, never the
    /// order hot path.
    pub fn apply_simulation(
        &mut self,
        request: &SimulationCommandRequest,
    ) -> Result<Applied, ApplyError> {
        let next_sequence = self
            .preflight(request.run_id, request.expected_sequence)
            .map_err(ApplyError::Unchanged)?;
        if matches!(request.payload, SimulationCommand::MassCancel { .. }) {
            return self
                .apply_simulation_validated(request, next_sequence)
                .map_err(ApplyError::Poisoned);
        }
        let mut candidate = self.clone();
        let applied = candidate
            .apply_simulation_validated(request, next_sequence)
            .map_err(ApplyError::Unchanged)?;
        *self = candidate;
        Ok(applied)
    }

    fn apply_simulation_validated(
        &mut self,
        request: &SimulationCommandRequest,
        next_sequence: EventSequence,
    ) -> Result<Applied, EngineError> {
        let metadata = Command {
            run_id: request.run_id,
            command_id: request.command_id,
            correlation_id: request.correlation_id,
            logical_time: request.logical_time,
            expected_sequence: request.expected_sequence,
            actor: request.actor,
            payload: CommandPayload::ActivateKillSwitch,
        };
        let candidate = self;
        let mut payloads = Vec::new();
        let mut changed_listings = BTreeSet::new();
        candidate.expire_due(request.logical_time, &mut payloads, &mut changed_listings)?;
        if let SimulationCommand::MassCancel {
            participant_id,
            instrument_id,
        } = &request.payload
        {
            let order_ids = candidate
                .ownership
                .values()
                .filter(|owned| {
                    owned.state == OwnedOrderState::Active
                        && participant_id.is_none_or(|id| owned.participant_id == id)
                        && instrument_id.is_none_or(|id| owned.listing_key.instrument_id == id)
                })
                .map(|owned| (owned.order_id, owned.listing_key))
                .collect::<Vec<_>>();
            for (order_id, listing_key) in &order_ids {
                changed_listings.insert(*listing_key);
                candidate.cancel(*order_id, CancelReason::MassCancel, &mut payloads)?;
            }
            payloads.push(EventPayload::Simulation(
                bunting_market_events::SimulationEvent::MassCancelCompleted {
                    canceled_orders: order_ids
                        .into_iter()
                        .map(|(order_id, _)| order_id)
                        .collect(),
                },
            ));
        } else {
            let mut context = SimulationContext {
                ledger: &mut candidate.ledger,
                participants: &candidate.participants,
                reporting_currency: candidate.reporting_currency,
                fx_rates: &candidate.fx_rates,
            };
            let domain_events = candidate
                .simulation
                .apply(
                    &mut context,
                    request.actor,
                    request.logical_time,
                    &request.payload,
                )
                .map_err(EngineError::Simulation)?;
            payloads.extend(domain_events.into_iter().map(EventPayload::Simulation));
            // Advancing the clock can pass GTD expiries.
            let now = candidate.simulation.clock.now;
            candidate.expire_due(now, &mut payloads, &mut changed_listings)?;
        }
        candidate.finish(
            &metadata,
            next_sequence,
            payloads,
            changed_listings,
            true,
            None,
            None,
        )
    }
}

/// Committed facts of one in-place transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Applied {
    pub events: Vec<EventEnvelope>,
    pub accepted: bool,
    pub reject_code: Option<String>,
    pub order_id: Option<OrderId>,
    pub changed_listings: BTreeSet<ListingKey>,
}

impl Applied {
    fn into_outcome(self, candidate: RunState) -> TransitionOutcome {
        TransitionOutcome {
            candidate,
            events: self.events,
            accepted: self.accepted,
            reject_code: self.reject_code,
            order_id: self.order_id,
            changed_listings: self.changed_listings,
        }
    }
}

/// Failure of an in-place transition, classified by what happened to the state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplyError {
    /// Refused before any mutation; the state is unchanged.
    Unchanged(EngineError),
    /// Failed after mutation began; the state must be discarded and rebuilt
    /// from the last committed state.
    Poisoned(EngineError),
}

impl ApplyError {
    #[must_use]
    pub const fn error(&self) -> &EngineError {
        match self {
            Self::Unchanged(error) | Self::Poisoned(error) => error,
        }
    }

    #[must_use]
    pub fn into_inner(self) -> EngineError {
        match self {
            Self::Unchanged(error) | Self::Poisoned(error) => error,
        }
    }
}

impl fmt::Display for ApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ApplyError {}

/// Candidate result of one authoritative engine transition.
#[derive(Clone, Debug)]
pub struct TransitionOutcome {
    pub candidate: RunState,
    pub events: Vec<EventEnvelope>,
    pub accepted: bool,
    pub reject_code: Option<String>,
    pub order_id: Option<OrderId>,
    pub changed_listings: BTreeSet<ListingKey>,
}

/// Versioned complete engine snapshot and canonical state hash.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EngineSnapshotEnvelope {
    pub schema_version: u16,
    pub state_hash: String,
    pub state: RunState,
}

impl EngineSnapshotEnvelope {
    pub fn new(state: RunState) -> Result<Self, SnapshotError> {
        let state_hash = state.state_hash()?;
        Ok(Self {
            schema_version: ENGINE_SNAPSHOT_VERSION,
            state_hash,
            state,
        })
    }

    pub fn to_json(&self) -> Result<String, SnapshotError> {
        serde_json::to_string(self).map_err(|_| SnapshotError::Serialization)
    }

    pub fn from_json(json: &str) -> Result<Self, SnapshotError> {
        #[derive(Deserialize)]
        struct VersionHeader {
            schema_version: u16,
        }
        let header: VersionHeader =
            serde_json::from_str(json).map_err(|_| SnapshotError::Serialization)?;
        if header.schema_version != ENGINE_SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        let envelope: Self =
            serde_json::from_str(json).map_err(|_| SnapshotError::Serialization)?;
        envelope.verify()?;
        Ok(envelope)
    }

    /// Checks the schema version, state invariants and canonical hash of an
    /// envelope that was decoded as part of a larger document.
    pub fn verify(&self) -> Result<(), SnapshotError> {
        if self.schema_version != ENGINE_SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        self.state.validate()?;
        if self.state.state_hash()? != self.state_hash {
            return Err(SnapshotError::HashMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Serialization,
    UnsupportedVersion,
    HashMismatch,
    InvalidScenario,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineError {
    InvalidScenario,
    UnknownListing,
    AmbiguousListing,
    SequenceConflict { current: EventSequence },
    SequenceOverflow,
    OwnershipInvariant,
    Accounting,
    Simulation(SimulationError),
    Snapshot(SnapshotError),
}

impl fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for EngineError {}

impl From<LedgerError> for EngineError {
    fn from(_: LedgerError) -> Self {
        Self::Accounting
    }
}

impl From<SnapshotError> for EngineError {
    fn from(error: SnapshotError) -> Self {
        Self::Snapshot(error)
    }
}

fn per_lot_fee(per_lot: MoneyMinor, quantity: QuantityLots) -> Result<MoneyMinor, EngineError> {
    per_lot
        .get()
        .checked_mul(i128::from(quantity.get()))
        .map(MoneyMinor::new)
        .ok_or(EngineError::Accounting)
}

fn reduce_order(
    order_id: OrderId,
    quantity: QuantityLots,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<(), EngineError> {
    let owned = ownership
        .get_mut(&order_id)
        .ok_or(EngineError::OwnershipInvariant)?;
    owned.remaining_quantity = owned
        .remaining_quantity
        .checked_sub(quantity)
        .filter(|remaining| remaining.get() >= 0)
        .ok_or(EngineError::Accounting)?;
    if owned.remaining_quantity.get() == 0 {
        owned.state = OwnedOrderState::Filled;
        payloads.push(EventPayload::OrderCompleted { order_id });
    } else {
        payloads.push(EventPayload::OrderReduced {
            order_id,
            remaining: owned.remaining_quantity,
        });
    }
    Ok(())
}

fn envelope(
    command: &Command,
    current_event_sequence: EventSequence,
    payloads: Vec<EventPayload>,
) -> Result<Vec<EventEnvelope>, EngineError> {
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let offset = u64::try_from(index + 1).map_err(|_| EngineError::SequenceOverflow)?;
            let sequence = current_event_sequence
                .get()
                .checked_add(offset)
                .map(EventSequence::new)
                .ok_or(EngineError::SequenceOverflow)?;
            // The run-wide event sequence is the only identity guaranteed unique
            // across commands; deriving IDs from command IDs collided.
            let event_id = EventId::new(u128::from(sequence.get()));
            Ok(EventEnvelope {
                schema_version: EVENT_SCHEMA_VERSION,
                run_id: command.run_id,
                event_id,
                sequence,
                logical_time: command.logical_time,
                actor: command.actor,
                command_id: command.command_id,
                correlation_id: command.correlation_id,
                causation_sequence: None,
                payload,
            })
        })
        .collect()
}

fn hash_serializable<T: Serialize>(value: &T) -> Result<String, SnapshotError> {
    let bytes = serde_json::to_vec(value).map_err(|_| SnapshotError::Serialization)?;
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bunting_market_events::SubmitOrder;
    use bunting_market_events::TimeInForcePolicy;
    use bunting_market_types::{CommandId, CorrelationId, LogicalTimeNs, VenueId};

    const CASH: CurrencyId = CurrencyId::new(1);

    fn instruments() -> [InstrumentDefinition; 2] {
        [
            InstrumentDefinition::new(InstrumentId::new(1), "ONE", CASH, InstrumentKind::Equity)
                .with_opening_mark(PriceTicks::new(100)),
            InstrumentDefinition::new(InstrumentId::new(2), "TWO", CASH, InstrumentKind::Equity)
                .with_opening_mark(PriceTicks::new(100)),
        ]
    }

    fn participant(id: u128) -> ParticipantDefinition {
        ParticipantDefinition {
            participant_id: ParticipantId::new(id),
            enabled: true,
            limits: RiskLimits::new(
                QuantityLots::new(100),
                QuantityLots::new(1_000),
                QuantityLots::new(1_000),
            ),
            initial_cash: BTreeMap::from([(CASH, MoneyMinor::new(100_000))]),
            initial_positions: BTreeMap::from([
                (InstrumentId::new(1), QuantityLots::new(100)),
                (InstrumentId::new(2), QuantityLots::new(100)),
            ]),
        }
    }

    fn run() -> RunState {
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            instruments(),
            [
                ListingDefinition::new(
                    ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                    "ONE".to_string(),
                    bounds,
                )
                .unwrap(),
                ListingDefinition::new(
                    ListingKey::new(VenueId::new(1), InstrumentId::new(2)),
                    "TWO".to_string(),
                    bounds,
                )
                .unwrap(),
            ],
            [participant(1), participant(2)],
        )
        .unwrap();
        RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "the test builder keeps command facts visible at each call site"
    )]
    fn submit(
        state: &RunState,
        command_id: u128,
        participant: u128,
        order_id: u128,
        instrument: u128,
        side: Side,
        price: i64,
        quantity: i64,
    ) -> Command {
        Command {
            run_id: state.run_id(),
            command_id: CommandId::new(command_id),
            correlation_id: CorrelationId::new(command_id),
            logical_time: LogicalTimeNs::new(u64::try_from(command_id).unwrap() * 1_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(participant),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(order_id),
                instrument_id: InstrumentId::new(instrument),
                participant_id: ParticipantId::new(participant),
                side,
                quantity: QuantityLots::new(quantity),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(price),
                },
            }),
        }
    }

    fn submit_market(
        state: &RunState,
        command_id: u128,
        participant: u128,
        order_id: u128,
        instrument: u128,
        side: Side,
        quantity: i64,
    ) -> Command {
        Command {
            run_id: state.run_id(),
            command_id: CommandId::new(command_id),
            correlation_id: CorrelationId::new(command_id),
            logical_time: LogicalTimeNs::new(u64::try_from(command_id).unwrap() * 1_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(participant),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(order_id),
                instrument_id: InstrumentId::new(instrument),
                participant_id: ParticipantId::new(participant),
                side,
                quantity: QuantityLots::new(quantity),
                kind: OrderKind::Market,
            }),
        }
    }

    fn levels(values: &[(i64, i64)]) -> VisibleLevels {
        values
            .iter()
            .map(|(price, quantity)| (PriceTicks::new(*price), QuantityLots::new(*quantity)))
            .collect()
    }

    fn at_listing(mut command: Command, listing_key: ListingKey) -> Command {
        if let CommandPayload::SubmitOrder(order) = command.payload {
            command.payload = CommandPayload::SubmitOrderAtListing { listing_key, order };
        }
        command
    }

    fn apply(state: RunState, command: &Command) -> TransitionOutcome {
        state.transition_owned(command).unwrap()
    }

    fn total_cash(state: &RunState) -> i128 {
        state
            .participants()
            .keys()
            .map(|participant| state.ledger().cash(*participant, CASH).balance.get())
            .sum()
    }

    #[test]
    fn fees_and_rebates_settle_once_and_conserve_cash_net_of_fees() {
        let fees = FeeSchedule {
            taker_per_lot: MoneyMinor::new(3),
            maker_per_lot: MoneyMinor::new(-1),
        };
        let mut state = run();
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        state.listings.get_mut(&key).unwrap().definition.fees = fees;
        let before = total_cash(&state);
        let maker = submit(&state, 1, 2, 1, 1, Side::Sell, 105, 10);
        state = apply(state, &maker).candidate;
        let taker = submit(&state, 2, 1, 2, 1, Side::Buy, 110, 4);
        let outcome = apply(state, &taker);
        assert!(outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::TradeExecuted { price, quantity, buyer_fee, seller_fee, .. }
                if price == PriceTicks::new(105)
                    && quantity == QuantityLots::new(4)
                    && buyer_fee == MoneyMinor::new(12)
                    && seller_fee == MoneyMinor::new(-4)
        )));
        let state = outcome.candidate;
        let buyer = state.ledger().cash(ParticipantId::new(1), CASH);
        assert_eq!(buyer.balance, MoneyMinor::new(100_000 - 420 - 12));
        assert_eq!(buyer.reserved, MoneyMinor::new(0));
        assert_eq!(buyer.fees, MoneyMinor::new(12));
        let seller = state.ledger().cash(ParticipantId::new(2), CASH);
        assert_eq!(seller.balance, MoneyMinor::new(100_000 + 420 + 4));
        // Six lots stay resting with exactly six lots of inventory reserved.
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(2), InstrumentId::new(1))
                .reserved,
            QuantityLots::new(6)
        );
        assert_eq!(total_cash(&state), before - 12 + 4);
        assert_eq!(
            state.ledger().mark(InstrumentId::new(1)),
            Some(PriceTicks::new(105))
        );
        // Realized P&L: seller sold 4 at 105 against an opening basis of 100.
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(2), InstrumentId::new(1))
                .realized_pnl,
            MoneyMinor::new(20)
        );
    }

    #[test]
    fn ioc_remainder_is_canceled_and_releases_its_reservation_exactly() {
        let mut state = run();
        state = apply(
            state.clone(),
            &submit(&state, 1, 2, 1, 1, Side::Sell, 101, 3),
        )
        .candidate;
        let mut ioc = submit(&state, 2, 1, 2, 1, Side::Buy, 102, 10);
        if let CommandPayload::SubmitOrder(order) = &mut ioc.payload {
            order.kind = OrderKind::LimitWithPolicy {
                price: PriceTicks::new(102),
                time_in_force: TimeInForcePolicy::Ioc,
                post_only: false,
                display_quantity: None,
            };
        }
        let outcome = apply(state, &ioc);
        assert!(outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderCanceled { order_id, remaining, reason: CancelReason::Expired, .. }
                if order_id == OrderId::new(2) && remaining == QuantityLots::new(7)
        )));
        assert!(!outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderRested { order_id, .. } if order_id == OrderId::new(2)
        )));
        let state = outcome.candidate;
        let cash = state.ledger().cash(ParticipantId::new(1), CASH);
        assert_eq!(cash.reserved, MoneyMinor::new(0));
        assert_eq!(cash.balance, MoneyMinor::new(100_000 - 303));
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(1), InstrumentId::new(1))
                .open_buy,
            QuantityLots::new(0)
        );
        assert_eq!(
            state.ownership()[&OrderId::new(2)].state,
            OwnedOrderState::Canceled
        );
    }

    #[test]
    fn infeasible_fok_is_killed_without_touching_the_book() {
        let mut state = run();
        state = apply(
            state.clone(),
            &submit(&state, 1, 2, 1, 1, Side::Sell, 101, 3),
        )
        .candidate;
        let book_before = state
            .visible_levels(ListingKey::new(VenueId::new(1), InstrumentId::new(1)))
            .unwrap();
        let mut fok = submit(&state, 2, 1, 2, 1, Side::Buy, 102, 10);
        if let CommandPayload::SubmitOrder(order) = &mut fok.payload {
            order.kind = OrderKind::LimitWithPolicy {
                price: PriceTicks::new(102),
                time_in_force: TimeInForcePolicy::Fok,
                post_only: false,
                display_quantity: None,
            };
        }
        let outcome = apply(state, &fok);
        assert!(outcome.accepted);
        assert!(
            !outcome
                .events
                .iter()
                .any(|event| matches!(event.payload, EventPayload::TradeExecuted { .. }))
        );
        assert!(outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderCanceled { remaining, reason: CancelReason::Expired, .. }
                if remaining == QuantityLots::new(10)
        )));
        assert_eq!(
            outcome
                .candidate
                .ledger()
                .cash(ParticipantId::new(1), CASH)
                .reserved,
            MoneyMinor::new(0)
        );
        let resting = outcome
            .candidate
            .visible_levels(ListingKey::new(VenueId::new(1), InstrumentId::new(1)))
            .unwrap();
        assert_eq!(resting, book_before);
        assert_eq!(resting.1, levels(&[(101, 3)]));
    }

    #[test]
    fn live_order_cap_counts_only_live_orders_and_survives_validation() {
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let mut trader = participant(1);
        trader.limits = trader.limits.with_max_live_orders(2);
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(41),
            ScenarioVersion::new(1),
            instruments(),
            [ListingDefinition::new(key, "ONE".into(), bounds).unwrap()],
            [trader, participant(2)],
        )
        .unwrap();
        let trader = ParticipantId::new(1);
        let mut state =
            RunState::from_scenario(RunId::new(41), IterationId::new(1), &scenario).unwrap();
        let accept = |state: RunState, command: &Command| {
            let outcome = apply(state, command);
            assert!(outcome.accepted, "{:?}", outcome.reject_code);
            outcome.candidate.validate().unwrap();
            outcome.candidate
        };

        state = accept(
            state.clone(),
            &submit(&state, 1, 1, 1, 1, Side::Buy, 50, 10),
        );
        state = accept(
            state.clone(),
            &submit(&state, 2, 1, 2, 1, Side::Buy, 51, 10),
        );
        assert_eq!(state.live_order_count(trader), 2);

        // At the cap: rejected without touching the count or the ledger.
        let reserved = state.ledger().cash(trader, CASH).reserved;
        let rejected = apply(
            state.clone(),
            &submit(&state, 3, 1, 3, 1, Side::Buy, 52, 10),
        );
        assert!(!rejected.accepted);
        assert_eq!(rejected.reject_code.as_deref(), Some("MaxLiveOrders"));
        assert_eq!(rejected.candidate.live_order_count(trader), 2);
        assert_eq!(
            rejected.candidate.ledger().cash(trader, CASH).reserved,
            reserved
        );
        state = rejected.candidate;

        // A fill that completes order 2 releases one slot.
        state = accept(
            state.clone(),
            &submit(&state, 4, 2, 4, 1, Side::Sell, 51, 10),
        );
        assert_eq!(state.live_order_count(trader), 1);
        state = accept(
            state.clone(),
            &submit(&state, 5, 1, 5, 1, Side::Buy, 49, 10),
        );
        assert_eq!(state.live_order_count(trader), 2);

        // A cancel releases one slot.
        let cancel = Command {
            run_id: state.run_id(),
            command_id: CommandId::new(6),
            correlation_id: CorrelationId::new(6),
            logical_time: LogicalTimeNs::new(6_000_000),
            expected_sequence: state.sequence(),
            actor: trader,
            payload: CommandPayload::CancelOrder(bunting_market_events::CancelOrder {
                order_id: OrderId::new(1),
                participant_id: trader,
            }),
        };
        state = accept(state, &cancel);
        assert_eq!(state.live_order_count(trader), 1);

        // An IOC that never rests leaves the count unchanged, even at the cap.
        state = accept(
            state.clone(),
            &submit(&state, 7, 1, 7, 1, Side::Buy, 48, 10),
        );
        assert_eq!(state.live_order_count(trader), 2);
        state = accept(
            state.clone(),
            &submit(&state, 8, 2, 8, 1, Side::Sell, 60, 5),
        );
        let mut ioc = submit(&state, 9, 1, 9, 1, Side::Buy, 60, 5);
        if let CommandPayload::SubmitOrder(order) = &mut ioc.payload {
            order.kind = OrderKind::LimitWithPolicy {
                price: PriceTicks::new(60),
                time_in_force: TimeInForcePolicy::Ioc,
                post_only: false,
                display_quantity: None,
            };
        }
        state = accept(state, &ioc);
        assert_eq!(state.live_order_count(trader), 2);
        assert_eq!(state.live_order_count(ParticipantId::new(2)), 0);

        // Snapshots carry the count, and a tampered count fails validation.
        let envelope = state.snapshot_envelope().unwrap();
        assert_eq!(
            EngineSnapshotEnvelope::from_json(&envelope.to_json().unwrap()).unwrap(),
            envelope
        );
        let mut tampered = state.clone();
        tampered.live_orders.insert(trader, 1);
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn instrument_risk_aggregates_open_orders_across_venues() {
        let instrument = InstrumentId::new(1);
        let primary = ListingKey::new(VenueId::new(1), instrument);
        let secondary = ListingKey::new(VenueId::new(2), instrument);
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let mut trader = participant(1);
        trader.limits.max_open_order_quantity = QuantityLots::new(15);
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(31),
            ScenarioVersion::new(1),
            instruments(),
            [
                ListingDefinition::new(primary, "P".into(), bounds).unwrap(),
                ListingDefinition::new(secondary, "S".into(), bounds).unwrap(),
            ],
            [trader, participant(2)],
        )
        .unwrap();
        let mut state =
            RunState::from_scenario(RunId::new(31), IterationId::new(1), &scenario).unwrap();
        let first = at_listing(submit(&state, 1, 1, 1, 1, Side::Buy, 50, 10), primary);
        let outcome = apply(state, &first);
        assert!(outcome.accepted);
        state = outcome.candidate;
        let second = at_listing(submit(&state, 2, 1, 2, 1, Side::Buy, 50, 10), secondary);
        let outcome = apply(state, &second);
        assert!(!outcome.accepted);
        assert_eq!(outcome.reject_code.as_deref(), Some("MaxOpenOrderQuantity"));
        assert_eq!(
            outcome
                .candidate
                .ledger()
                .cash(ParticipantId::new(1), CASH)
                .reserved,
            MoneyMinor::new(500)
        );
    }

    #[test]
    fn shortable_instruments_admit_uncovered_sales_and_value_short_positions() {
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let short =
            InstrumentDefinition::new(InstrumentId::new(1), "S", CASH, InstrumentKind::Equity)
                .shortable()
                .with_multiplier(10)
                .with_opening_mark(PriceTicks::new(100));
        let mut seller = participant(2);
        seller.initial_positions.clear();
        let mut buyer = participant(1);
        buyer.initial_positions.clear();
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(32),
            ScenarioVersion::new(1),
            [short],
            [ListingDefinition::new(key, "S".into(), bounds).unwrap()],
            [buyer, seller],
        )
        .unwrap();
        let mut state =
            RunState::from_scenario(RunId::new(32), IterationId::new(1), &scenario).unwrap();
        state = apply(
            state.clone(),
            &submit(&state, 1, 2, 1, 1, Side::Sell, 90, 5),
        )
        .candidate;
        state = apply(state.clone(), &submit(&state, 2, 1, 2, 1, Side::Buy, 90, 5)).candidate;
        let seller = state
            .ledger()
            .position(ParticipantId::new(2), InstrumentId::new(1));
        assert_eq!(seller.quantity, QuantityLots::new(-5));
        assert_eq!(seller.cost_basis, MoneyMinor::new(-4_500));
        assert_eq!(
            state.ledger().cash(ParticipantId::new(2), CASH).balance,
            MoneyMinor::new(104_500)
        );
        assert_eq!(
            state.net_liquidation_value(ParticipantId::new(2)).unwrap(),
            MoneyMinor::new(100_000)
        );
    }

    #[test]
    fn distinct_orders_sharing_low_64_bits_never_alias() {
        let low = OrderId::new(7);
        let high = OrderId::new((1_u128 << 64) | 7);
        let mut state = run();
        state = apply(
            state.clone(),
            &submit(&state, 1, 2, low.get(), 1, Side::Sell, 110, 2),
        )
        .candidate;
        state = apply(
            state.clone(),
            &submit(&state, 2, 2, high.get(), 1, Side::Sell, 111, 3),
        )
        .candidate;
        let cancel = Command {
            run_id: state.run_id(),
            command_id: CommandId::new(3),
            correlation_id: CorrelationId::new(3),
            logical_time: LogicalTimeNs::new(3_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(2),
            payload: CommandPayload::CancelOrder(bunting_market_events::CancelOrder {
                order_id: high,
                participant_id: ParticipantId::new(2),
            }),
        };
        let outcome = apply(state, &cancel);
        assert!(outcome.accepted);
        let state = outcome.candidate;
        assert_eq!(state.ownership()[&high].state, OwnedOrderState::Canceled);
        assert_eq!(state.ownership()[&low].state, OwnedOrderState::Active);
        assert_eq!(
            state
                .visible_levels(ListingKey::new(VenueId::new(1), InstrumentId::new(1)))
                .unwrap()
                .1,
            levels(&[(110, 2)])
        );
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(2), InstrumentId::new(1))
                .reserved,
            QuantityLots::new(2)
        );
        let buy = submit(&state, 4, 1, 99, 1, Side::Buy, 111, 2);
        let filled = apply(state, &buy);
        assert!(filled.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::TradeExecuted { maker_order_id, .. } if maker_order_id == low
        )));
    }

    #[test]
    fn accepted_tender_settles_against_the_house_and_scores_every_participant() {
        let mut state = run();
        let request = |state: &RunState, id: u128, actor: u128, payload| SimulationCommandRequest {
            run_id: state.run_id(),
            command_id: CommandId::new(id),
            correlation_id: CorrelationId::new(id),
            logical_time: LogicalTimeNs::new(u64::try_from(id).unwrap()),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(actor),
            payload,
        };
        let open = request(
            &state,
            1,
            9,
            SimulationCommand::OpenTender {
                tender_id: bunting_market_types::TenderId::new(1),
                participant_id: ParticipantId::new(1),
                instrument_id: InstrumentId::new(1),
                side: Side::Sell,
                quantity: QuantityLots::new(50),
                price: PriceTicks::new(120),
                expires_at: LogicalTimeNs::new(100),
            },
        );
        state = state.transition_simulation_owned(&open).unwrap().candidate;
        let accept = request(
            &state,
            2,
            1,
            SimulationCommand::DecideTender {
                tender_id: bunting_market_types::TenderId::new(1),
                decision: bunting_market_events::TenderDecision::Accept,
            },
        );
        let outcome = state.transition_simulation_owned(&accept).unwrap();
        assert!(outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::Simulation(bunting_market_events::SimulationEvent::TenderSettled { .. })
        )));
        state = outcome.candidate;
        assert_eq!(
            state.ledger().cash(ParticipantId::new(1), CASH).balance,
            MoneyMinor::new(106_000)
        );
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(1), InstrumentId::new(1))
                .quantity,
            QuantityLots::new(50)
        );
        // An off-book tender price never becomes the public valuation mark.
        assert_eq!(
            state.ledger().mark(InstrumentId::new(1)),
            Some(PriceTicks::new(100))
        );
        let score = request(&state, 3, 9, SimulationCommand::ScoreIteration);
        state = state.transition_simulation_owned(&score).unwrap().candidate;
        let report = state.simulation().reports.last().unwrap();
        assert_eq!(report.entries.len(), 2);
        assert_eq!(report.entries[0].participant_id, ParticipantId::new(1));
        assert_eq!(
            report.entries[0].score,
            MoneyMinor::new(106_000 + 5_000 + 10_000)
        );
        assert_eq!(
            report.entries[1].score,
            MoneyMinor::new(100_000 + 10_000 + 10_000)
        );
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "single scenario asserts routing, independent depth and trade accounting"
    )]
    fn explicit_listing_commands_keep_cross_listed_books_and_trades_separate() {
        let instrument = InstrumentId::new(1);
        let primary = ListingKey::new(VenueId::new(1), instrument);
        let secondary = ListingKey::new(VenueId::new(2), instrument);
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let mut buyer = participant(1);
        let mut seller = participant(2);
        buyer
            .initial_positions
            .retain(|asset, _| *asset == instrument);
        seller
            .initial_positions
            .retain(|asset, _| *asset == instrument);
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(21),
            ScenarioVersion::new(1),
            instruments(),
            [
                ListingDefinition::new(primary, "PRIMARY".to_owned(), bounds).unwrap(),
                ListingDefinition::new(secondary, "SECONDARY".to_owned(), bounds).unwrap(),
            ],
            [buyer, seller],
        )
        .unwrap();
        let mut state =
            RunState::from_scenario(RunId::new(21), IterationId::new(1), &scenario).unwrap();
        let legacy = submit(&state, 1, 2, 100, 1, Side::Sell, 120, 1);
        assert!(matches!(
            state.transition(&legacy),
            Err(EngineError::AmbiguousListing)
        ));
        let target = |state: &RunState, venue: ListingKey, id: u128, actor: u128, side: Side| {
            let mut command = submit(state, id, actor, id, 1, side, 120, 1);
            if let CommandPayload::SubmitOrder(order) = command.payload {
                command.payload = CommandPayload::SubmitOrderAtListing {
                    listing_key: venue,
                    order,
                };
            }
            command
        };
        let sell = target(&state, primary, 201, 2, Side::Sell);
        let outcome = state.transition(&sell).unwrap();
        assert!(outcome.accepted);
        state = outcome.candidate;
        let buy_elsewhere = target(&state, secondary, 202, 1, Side::Buy);
        let outcome = state.transition(&buy_elsewhere).unwrap();
        assert!(outcome.accepted);
        assert!(
            !outcome
                .events
                .iter()
                .any(|event| { matches!(event.payload, EventPayload::TradeExecuted { .. }) })
        );
        state = outcome.candidate;
        assert_eq!(
            state.visible_levels(primary).unwrap().1,
            vec![(PriceTicks::new(120), QuantityLots::new(1))]
        );
        assert_eq!(
            state.visible_levels(secondary).unwrap().0,
            vec![(PriceTicks::new(120), QuantityLots::new(1))]
        );
        let buy_primary = target(&state, primary, 203, 1, Side::Buy);
        let outcome = state.transition(&buy_primary).unwrap();
        assert!(outcome.accepted);
        assert!(outcome.events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::TradeExecuted {
                    listing_key: Some(key),
                    instrument_id,
                    ..
                } if *key == primary && *instrument_id == instrument
            )
        }));
        state = outcome.candidate;
        assert_eq!(
            state.simulation().market_by_listing[&primary].trades.len(),
            1
        );
        assert!(
            state
                .simulation()
                .market_by_listing
                .get(&secondary)
                .is_none_or(|market| market.trades.is_empty())
        );
        assert_eq!(
            state.visible_levels(secondary).unwrap().0,
            vec![(PriceTicks::new(120), QuantityLots::new(1))]
        );
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(1), instrument)
                .quantity,
            QuantityLots::new(101)
        );
        let restored = EngineSnapshotEnvelope::from_json(
            &state.snapshot_envelope().unwrap().to_json().unwrap(),
        )
        .unwrap();
        assert_eq!(
            restored.state.state_hash().unwrap(),
            state.state_hash().unwrap()
        );
    }

    #[test]
    fn owned_transition_matches_immutable_command_cancel_and_simulation_paths() {
        let state = run();
        let sell = submit(&state, 1, 2, 41, 1, Side::Sell, 110, 3);
        let borrowed = state.transition(&sell).unwrap();
        let owned = state.clone().transition_owned(&sell).unwrap();
        assert_eq!(borrowed.candidate, owned.candidate);
        assert_eq!(borrowed.events, owned.events);
        assert_eq!(borrowed.changed_listings, owned.changed_listings);
        let after_submit = borrowed.candidate;

        let cancel = Command {
            run_id: after_submit.run_id(),
            command_id: CommandId::new(2),
            correlation_id: CorrelationId::new(2),
            logical_time: LogicalTimeNs::new(2_000_000),
            expected_sequence: after_submit.sequence(),
            actor: ParticipantId::new(2),
            payload: CommandPayload::CancelOrder(bunting_market_events::CancelOrder {
                order_id: OrderId::new(41),
                participant_id: ParticipantId::new(2),
            }),
        };
        let borrowed = after_submit.transition(&cancel).unwrap();
        let owned = after_submit.clone().transition_owned(&cancel).unwrap();
        assert_eq!(borrowed.candidate, owned.candidate);
        assert_eq!(borrowed.events, owned.events);
        let after_cancel = borrowed.candidate;

        let request = SimulationCommandRequest {
            run_id: after_cancel.run_id(),
            command_id: CommandId::new(3),
            correlation_id: CorrelationId::new(3),
            logical_time: LogicalTimeNs::new(3_000_000),
            expected_sequence: after_cancel.sequence(),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::MassCancel {
                participant_id: None,
                instrument_id: Some(InstrumentId::new(1)),
            },
        };
        let borrowed = after_cancel.transition_simulation(&request).unwrap();
        let owned = after_cancel
            .clone()
            .transition_simulation_owned(&request)
            .unwrap();
        assert_eq!(borrowed.candidate, owned.candidate);
        assert_eq!(borrowed.events, owned.events);
        assert_eq!(borrowed.changed_listings, owned.changed_listings);
        assert_eq!(
            borrowed.candidate.state_hash().unwrap(),
            owned.candidate.state_hash().unwrap()
        );
    }

    #[test]
    fn canonical_full_width_identities_drive_matching_replay_and_cancel() {
        let large = OrderId::new(u128::from(u64::MAX) + 123);
        let small = OrderId::new(1);
        let mut state = run();
        let first = submit(&state, 1, 2, large.get(), 1, Side::Sell, 110, 2);
        let accepted = state.transition(&first).unwrap();
        assert!(accepted.accepted);
        state = accepted.candidate;

        let next = submit(&state, 2, 2, small.get(), 1, Side::Sell, 120, 1);
        let accepted = state.transition(&next).unwrap();
        assert!(accepted.accepted);
        state = accepted.candidate;

        let snapshot = state.snapshot_envelope().unwrap();
        assert_eq!(snapshot.schema_version, ENGINE_SNAPSHOT_VERSION);
        let replay_start = EngineSnapshotEnvelope::from_json(&snapshot.to_json().unwrap())
            .unwrap()
            .state;
        assert_eq!(
            replay_start.state_hash().unwrap(),
            state.state_hash().unwrap()
        );

        let first_buy = submit(&state, 3, 1, 123, 1, Side::Buy, 110, 2);
        let filled = state.transition(&first_buy).unwrap();
        assert!(filled.accepted);
        assert!(filled.events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::TradeExecuted {
                maker_order_id,
                taker_order_id,
                ..
            } if *maker_order_id == large && *taker_order_id == OrderId::new(123)
        )));
        let replayed = replay_start.transition(&first_buy).unwrap();
        assert_eq!(
            replayed.candidate.state_hash().unwrap(),
            filled.candidate.state_hash().unwrap()
        );
        state = filled.candidate;

        let cancel = Command {
            run_id: state.run_id(),
            command_id: CommandId::new(4),
            correlation_id: CorrelationId::new(4),
            logical_time: LogicalTimeNs::new(4_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(2),
            payload: CommandPayload::CancelOrder(bunting_market_events::CancelOrder {
                order_id: small,
                participant_id: ParticipantId::new(2),
            }),
        };
        let cancelled = state.transition(&cancel).unwrap();
        assert!(cancelled.accepted);
        assert!(cancelled.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderCanceled { order_id, .. } if order_id == small
        )));
        assert_eq!(
            cancelled.candidate.ownership()[&small].state,
            OwnedOrderState::Canceled
        );
    }

    #[test]
    fn mass_cancel_stages_one_book_per_listing_and_is_replay_deterministic() {
        let mut state = run();
        for (command_id, owner, instrument, side, price) in [
            (1, 2, 1, Side::Sell, 101),
            (2, 2, 1, Side::Sell, 102),
            (3, 2, 1, Side::Sell, 103),
            (4, 1, 2, Side::Buy, 95),
        ] {
            let next = submit(
                &state, command_id, owner, command_id, instrument, side, price, 1,
            );
            let accepted = state.transition(&next).unwrap();
            assert!(accepted.accepted);
            state = accepted.candidate;
        }
        let untouched = ListingKey::new(VenueId::new(1), InstrumentId::new(2));
        let removed = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let untouched_depth = state.visible_levels(untouched).unwrap();
        let previous = state.clone();
        let request = SimulationCommandRequest {
            run_id: state.run_id(),
            command_id: CommandId::new(5),
            correlation_id: CorrelationId::new(5),
            logical_time: LogicalTimeNs::new(5_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::MassCancel {
                participant_id: Some(ParticipantId::new(2)),
                instrument_id: Some(InstrumentId::new(1)),
            },
        };
        let finished = state.transition_simulation(&request).unwrap();
        assert!(finished.accepted);
        assert_eq!(finished.changed_listings, BTreeSet::from([removed]));
        assert_eq!(
            finished.candidate.visible_levels(untouched).unwrap(),
            untouched_depth
        );
        assert!(
            finished
                .candidate
                .visible_levels(removed)
                .unwrap()
                .1
                .is_empty()
        );
        assert_eq!(
            finished.candidate.visible_levels(untouched).unwrap().0,
            levels(&[(95, 1)])
        );
        for id in [1, 2, 3] {
            assert_eq!(
                finished.candidate.ownership()[&OrderId::new(id)].state,
                OwnedOrderState::Canceled
            );
        }
        assert_eq!(
            finished.candidate.ownership()[&OrderId::new(4)].state,
            OwnedOrderState::Active
        );
        assert_eq!(
            previous
                .transition_simulation(&request)
                .unwrap()
                .candidate
                .state_hash()
                .unwrap(),
            finished.candidate.state_hash().unwrap()
        );
        let snapshot = finished
            .candidate
            .snapshot_envelope()
            .unwrap()
            .to_json()
            .unwrap();
        assert_eq!(
            EngineSnapshotEnvelope::from_json(&snapshot).unwrap().state,
            finished.candidate
        );
    }

    #[test]
    fn old_snapshot_versions_are_rejected_not_migrated() {
        let state = run();
        let mut value = serde_json::to_value(state.snapshot_envelope().unwrap()).unwrap();
        value["schema_version"] = serde_json::json!(2);
        value["state"].as_object_mut().unwrap().remove("expiries");
        assert_eq!(
            EngineSnapshotEnvelope::from_json(&value.to_string()),
            Err(SnapshotError::UnsupportedVersion)
        );
    }

    #[test]
    fn two_listings_are_isolated_and_iteration_is_deterministic() {
        let state = run();
        let before_two =
            state.listings()[&ListingKey::new(VenueId::new(1), InstrumentId::new(2))].clone();
        let outcome = state
            .transition(&submit(&state, 1, 1, 1, 1, Side::Buy, 100, 10))
            .unwrap();
        assert_eq!(outcome.candidate.sequence(), EventSequence::new(1));
        assert_eq!(
            outcome.candidate.listings()[&ListingKey::new(VenueId::new(1), InstrumentId::new(2))],
            before_two
        );
        let keys: Vec<_> = outcome.candidate.listings().keys().copied().collect();
        assert_eq!(
            keys,
            vec![
                ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                ListingKey::new(VenueId::new(1), InstrumentId::new(2))
            ]
        );
    }

    #[test]
    fn one_command_advances_one_run_sequence_and_snapshot_round_trips() {
        let state = run();
        let outcome = state
            .transition(&submit(&state, 1, 1, 1, 1, Side::Sell, 100, 10))
            .unwrap();
        assert!(outcome.events.len() > 1);
        assert_eq!(outcome.candidate.sequence(), EventSequence::new(1));
        let envelope = outcome.candidate.snapshot_envelope().unwrap();
        let restored = EngineSnapshotEnvelope::from_json(&envelope.to_json().unwrap()).unwrap();
        assert_eq!(restored.state.state_hash(), outcome.candidate.state_hash());
    }

    #[test]
    fn staged_failure_leaves_full_state_unchanged() {
        let mut state = run();
        state = apply(
            state.clone(),
            &submit(&state, 1, 2, 1, 1, Side::Sell, 100, 1),
        )
        .candidate;
        // Corrupt ownership so the cancel fails after staging begins.
        state
            .ownership
            .get_mut(&OrderId::new(1))
            .unwrap()
            .remaining_quantity = QuantityLots::new(9);
        let before = state.state_hash().unwrap();
        let command = Command {
            run_id: state.run_id(),
            command_id: CommandId::new(2),
            correlation_id: CorrelationId::new(2),
            logical_time: LogicalTimeNs::new(2_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(2),
            payload: CommandPayload::CancelOrder(bunting_market_events::CancelOrder {
                order_id: OrderId::new(1),
                participant_id: ParticipantId::new(2),
            }),
        };
        assert_eq!(
            state.transition(&command).unwrap_err(),
            EngineError::OwnershipInvariant
        );
        assert_eq!(state.state_hash().unwrap(), before);
    }

    #[test]
    fn apply_refusals_before_mutation_leave_state_unchanged() {
        let mut state = run();
        // A GTD order that is already due when each refused command arrives,
        // so any refusal after expiry processing would be visible.
        let mut gtd = submit(&state, 1, 1, 1, 1, Side::Buy, 100, 1);
        if let CommandPayload::SubmitOrder(order) = &mut gtd.payload {
            order.kind = OrderKind::LimitWithPolicy {
                price: PriceTicks::new(100),
                time_in_force: TimeInForcePolicy::Gtd {
                    expires_at: LogicalTimeNs::new(1_500_000),
                },
                post_only: false,
                display_quantity: None,
            };
        }
        state.apply(&gtd).unwrap();
        let before = state.clone();

        let mut unknown = submit(&state, 2, 1, 2, 1, Side::Buy, 100, 1);
        let CommandPayload::SubmitOrder(order) = unknown.payload.clone() else {
            unreachable!("submit builds a SubmitOrder");
        };
        unknown.payload = CommandPayload::SubmitOrderAtListing {
            order,
            listing_key: ListingKey::new(VenueId::new(9), InstrumentId::new(1)),
        };
        assert_eq!(
            state.apply(&unknown),
            Err(ApplyError::Unchanged(EngineError::UnknownListing))
        );
        assert_eq!(state, before);

        let mut outdated = submit(&state, 3, 1, 3, 1, Side::Buy, 100, 1);
        outdated.expected_sequence = EventSequence::new(0);
        assert!(matches!(
            state.apply(&outdated),
            Err(ApplyError::Unchanged(EngineError::SequenceConflict { .. }))
        ));
        assert_eq!(state, before);

        // Refused by a simulation domain rule after the expiry would have run.
        let resume = SimulationCommandRequest {
            run_id: state.run_id(),
            command_id: CommandId::new(4),
            correlation_id: CorrelationId::new(4),
            logical_time: LogicalTimeNs::new(4_000_000),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::ResumeRun,
        };
        assert!(matches!(
            state.apply_simulation(&resume),
            Err(ApplyError::Unchanged(EngineError::Simulation(_)))
        ));
        assert_eq!(state, before);

        // The expiry still happens, once, on the next accepted command.
        let next = submit(&state, 5, 1, 5, 1, Side::Buy, 100, 1);
        let applied = state.apply(&next).unwrap();
        assert!(applied.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderCanceled {
                reason: CancelReason::Expired,
                ..
            }
        )));
    }

    #[test]
    fn apply_matches_consuming_transition_and_classifies_poisoning() {
        let mut live = run();
        let mut owned = run();
        let orders = [
            (1, 1, Side::Sell, 101, 5),
            (2, 1, Side::Sell, 100, 5),
            (2, 2, Side::Buy, 101, 7),
            (1, 2, Side::Buy, 99, 3),
            (2, 1, Side::Sell, 99, 4),
        ];
        for (index, (participant, instrument, side, price, quantity)) in
            orders.into_iter().enumerate()
        {
            let id = u128::try_from(index + 1).unwrap();
            let command = submit(
                &live,
                id,
                participant,
                id,
                instrument,
                side,
                price,
                quantity,
            );
            let applied = live.apply(&command).unwrap();
            let outcome = owned.transition_owned(&command).unwrap();
            assert_eq!(applied.events, outcome.events);
            assert_eq!(applied.changed_listings, outcome.changed_listings);
            owned = outcome.candidate;
            assert_eq!(live, owned);
        }

        // An invariant failure after staging began poisons the live value.
        let resting = live
            .ownership
            .values()
            .find(|order| order.state == OwnedOrderState::Active)
            .map(|order| (order.order_id, order.participant_id))
            .unwrap();
        live.ownership
            .get_mut(&resting.0)
            .unwrap()
            .remaining_quantity = QuantityLots::new(999);
        let cancel = Command {
            run_id: live.run_id(),
            command_id: CommandId::new(99),
            correlation_id: CorrelationId::new(99),
            logical_time: LogicalTimeNs::new(99_000_000),
            expected_sequence: live.sequence(),
            actor: resting.1,
            payload: CommandPayload::CancelOrder(bunting_market_events::CancelOrder {
                order_id: resting.0,
                participant_id: resting.1,
            }),
        };
        assert_eq!(
            live.apply(&cancel),
            Err(ApplyError::Poisoned(EngineError::OwnershipInvariant))
        );
    }

    #[test]
    fn snapshot_plus_replayed_commands_matches_uninterrupted_state() {
        let state = run();
        let first_command = submit(&state, 1, 1, 1, 1, Side::Sell, 100, 10);
        let first = state.transition(&first_command).unwrap().candidate;
        let restored = EngineSnapshotEnvelope::from_json(
            &first.snapshot_envelope().unwrap().to_json().unwrap(),
        )
        .unwrap()
        .state;
        let second_command = submit(&first, 2, 2, 2, 1, Side::Buy, 100, 4);
        let uninterrupted = first.transition(&second_command).unwrap().candidate;
        let replayed = restored.transition(&second_command).unwrap().candidate;
        assert_eq!(uninterrupted.state_hash(), replayed.state_hash());
    }

    #[test]
    fn market_order_executes_through_orderbook_rs_and_completes() {
        let state = run();
        let resting = state
            .transition(&submit(&state, 1, 1, 1, 1, Side::Sell, 101, 10))
            .unwrap()
            .candidate;
        let outcome = resting
            .transition(&submit_market(&resting, 2, 2, 2, 1, Side::Buy, 4))
            .unwrap();

        assert!(outcome.accepted);
        assert!(outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::TradeExecuted {
                taker_order_id,
                price,
                quantity,
                ..
            } if taker_order_id == OrderId::new(2)
                && price == PriceTicks::new(101)
                && quantity == QuantityLots::new(4)
        )));
        assert!(outcome.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderCompleted { order_id } if order_id == OrderId::new(2)
        )));
        assert_eq!(
            outcome
                .candidate
                .visible_levels(ListingKey::new(VenueId::new(1), InstrumentId::new(1)))
                .unwrap()
                .1,
            levels(&[(101, 6)])
        );
        assert_eq!(
            outcome
                .candidate
                .ownership()
                .get(&OrderId::new(2))
                .unwrap()
                .state,
            OwnedOrderState::Filled
        );
    }

    #[test]
    fn unsupported_snapshot_versions_are_rejected() {
        let state = run();
        let mut envelope = state.snapshot_envelope().unwrap();
        envelope.schema_version += 1;
        let json = serde_json::to_string(&envelope).unwrap();
        assert_eq!(
            EngineSnapshotEnvelope::from_json(&json),
            Err(SnapshotError::UnsupportedVersion)
        );
    }

    #[test]
    fn scenario_hash_is_canonical_and_decoding_is_strict() {
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let one = ListingDefinition::new(
            ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
            "ONE".to_string(),
            bounds,
        )
        .unwrap();
        let two = ListingDefinition::new(
            ListingKey::new(VenueId::new(1), InstrumentId::new(2)),
            "TWO".to_string(),
            bounds,
        )
        .unwrap();
        let [first_instrument, second_instrument] = instruments();
        let first = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [first_instrument.clone(), second_instrument.clone()],
            [one.clone(), two.clone()],
            [participant(1), participant(2)],
        )
        .unwrap();
        let second = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [second_instrument, first_instrument],
            [two, one],
            [participant(2), participant(1)],
        )
        .unwrap();
        assert_eq!(first.content_hash(), second.content_hash());
        let mut value = serde_json::to_value(first).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert("unknown".to_string(), serde_json::Value::Bool(true));
        assert!(serde_json::from_value::<ScenarioDefinition>(value).is_err());
    }
}
