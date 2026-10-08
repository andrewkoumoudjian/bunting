#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Authoritative sans-I/O Bunting market-simulation engine.

pub mod compatibility;
mod matching;
pub mod simulation;

use bunting_ledger::{Fill, FillParty, LedgerError};
pub use bunting_ledger::{FxRate, InstrumentTerms, Ledger, Reservation};
use bunting_market_events::{
    CancelReason, Command, CommandPayload, EVENT_SCHEMA_VERSION, EventEnvelope, EventPayload,
    OrderKind, RejectCode, Side, SimulationCommand, SimulationCommandRequest,
};
use bunting_market_types::{
    CurrencyId, EventId, EventSequence, InstrumentId, IterationId, ListingKey, MoneyMinor, OrderId,
    ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId, ScenarioVersion,
};
use bunting_risk_engine::RiskLimits;
use compatibility::nbc::{
    NBC_TRANSLATION_VERSION, NbcCompatibilityState, RunStatus as NbcRunStatus,
    ScenarioConfig as NbcScenarioConfig, ScheduledEvent as NbcScheduledEvent,
};
use matching::{
    KernelBook, SnapshotPackage, TimeInForce, TradeInfo, sequential_id_from_text,
    to_upstream_price, to_upstream_quantity, to_upstream_side, to_upstream_time_in_force,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use simulation::InstrumentKind;
use simulation::{
    SIMULATION_POLICY_VERSION, SimulationContext, SimulationError, SimulationScenario,
    SimulationState,
};
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub use matching::{ORDERBOOK_RS_AUDIT_COMMIT, ORDERBOOK_RS_VERSION};

/// Version of the central engine behavior established by this foundation slice.
pub const ENGINE_VERSION: u16 = 3;
/// Version of the complete persisted engine snapshot envelope.
pub const ENGINE_SNAPSHOT_VERSION: u16 = 3;
/// Version of the Bunting-native scenario schema.
pub const SCENARIO_SCHEMA_VERSION: u16 = 2;
/// Maximum economic instruments admitted into one run.
pub const MAX_INSTRUMENTS: usize = 256;
/// Version of each nested listing snapshot record.
pub const LISTING_SNAPSHOT_VERSION: u16 = 1;
/// Maximum listings admitted into one foundation run.
pub const MAX_LISTINGS: usize = 64;
/// Maximum participants admitted into one foundation run.
pub const MAX_PARTICIPANTS: usize = 1_024;
/// Maximum retained order ownership records in one foundation run.
pub const MAX_ORDERS: usize = 100_000;
/// Maximum canonical events emitted by one command.
pub const MAX_EVENTS_PER_TRANSITION: usize = 256;
/// Maximum depth captured from the upstream matcher.
pub const SNAPSHOT_DEPTH: usize = 10_000;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Visible price and quantity levels for one side of a listing.
pub type VisibleLevels = Vec<(u128, u64)>;
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
    pub upstream_order_id: u64,
    pub participant_id: ParticipantId,
    pub listing_key: ListingKey,
    pub side: Side,
    pub limit_price: PriceTicks,
    pub original_quantity: QuantityLots,
    pub remaining_quantity: QuantityLots,
    pub state: OwnedOrderState,
    /// Exact ledger reservation released on fill, cancel or expiry.
    pub reservation: Reservation,
}

/// Versioned snapshot for one private matcher boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingSnapshot {
    pub schema_version: u16,
    pub represented_sequence: EventSequence,
    pub checksum: String,
    pub package_json: String,
}

/// Authoritative state for one venue listing. The live matcher never escapes this crate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingState {
    definition: ListingDefinition,
    snapshot: ListingSnapshot,
}

impl ListingState {
    #[must_use]
    pub const fn definition(&self) -> &ListingDefinition {
        &self.definition
    }

    #[must_use]
    pub const fn snapshot(&self) -> &ListingSnapshot {
        &self.snapshot
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
    ownership: BTreeMap<OrderId, OwnedOrder>,
    /// Single authoritative upstream-to-canonical identity index, including
    /// every accepted order regardless of external identifier size.
    upstream_to_canonical: BTreeMap<u64, OrderId>,
    /// Next globally unique sequential ID for any venue in this run.
    next_upstream_order_id: u64,
    #[serde(default)]
    nbc_compatibility: Option<NbcCompatibilityState>,
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
        let mut listings = BTreeMap::new();
        for (key, definition) in &scenario.listings {
            let book = KernelBook::new(&definition.symbol);
            let snapshot = snapshot_from_package(
                EventSequence::new(0),
                book.snapshot_package(SNAPSHOT_DEPTH)
                    .map_err(|_| EngineError::Upstream)?,
            );
            listings.insert(
                *key,
                ListingState {
                    definition: definition.clone(),
                    snapshot,
                },
            );
        }
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
            upstream_to_canonical: BTreeMap::new(),
            next_upstream_order_id: 1,
            nbc_compatibility: None,
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

    #[must_use]
    pub fn ownership(&self) -> &BTreeMap<OrderId, OwnedOrder> {
        &self.ownership
    }

    #[must_use]
    pub const fn nbc_compatibility(&self) -> Option<&NbcCompatibilityState> {
        self.nbc_compatibility.as_ref()
    }

    /// Returns the complete authoritative simulation component.
    #[must_use]
    pub const fn simulation(&self) -> &SimulationState {
        &self.simulation
    }

    pub fn with_nbc_compatibility(
        mut self,
        config: NbcScenarioConfig,
        events: Vec<NbcScheduledEvent>,
    ) -> Result<Self, EngineError> {
        self.nbc_compatibility = Some(
            NbcCompatibilityState::new(
                self.run_id.to_string(),
                config,
                events,
                self.participants.keys().copied(),
            )
            .map_err(|_| EngineError::NbcCompatibility)?,
        );
        Ok(self)
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

    pub fn listing_snapshot(&self, key: ListingKey) -> Result<&ListingSnapshot, EngineError> {
        self.listings
            .get(&key)
            .map(ListingState::snapshot)
            .ok_or(EngineError::UnknownListing)
    }

    pub fn visible_levels(&self, key: ListingKey) -> Result<VisibleDepth, EngineError> {
        matching::visible_levels_from_snapshot_json(&self.listing_snapshot(key)?.package_json)
            .map_err(|_| EngineError::InvalidSnapshot)
    }

    pub fn state_hash(&self) -> Result<String, SnapshotError> {
        hash_serializable(self)
    }

    fn validate(&self) -> Result<(), SnapshotError> {
        if self.config.engine_version != ENGINE_VERSION
            || self.config.max_listings == 0
            || usize::from(self.config.max_listings) > MAX_LISTINGS
            || self.listings.is_empty()
            || self.listings.len() > usize::from(self.config.max_listings)
            || self.participants.len() > MAX_PARTICIPANTS
            || self.ownership.len() > MAX_ORDERS
            || self.next_upstream_order_id == 0
            || self.upstream_to_canonical.len() != self.ownership.len()
            || self
                .upstream_to_canonical
                .iter()
                .any(|(upstream, canonical)| {
                    *upstream == 0
                        || *upstream >= self.next_upstream_order_id
                        || self.ownership.get(canonical).is_none_or(|owned| {
                            owned.order_id != *canonical || owned.upstream_order_id != *upstream
                        })
                })
            || self.ownership.iter().any(|(canonical, owned)| {
                *canonical != owned.order_id
                    || self.upstream_to_canonical.get(&owned.upstream_order_id) != Some(canonical)
            })
            || self
                .nbc_compatibility
                .as_ref()
                .is_some_and(|compatibility| {
                    compatibility.profile_version != NBC_TRANSLATION_VERSION
                })
            || self
                .listings
                .values()
                .any(|listing| listing.snapshot.schema_version != LISTING_SNAPSHOT_VERSION)
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
    /// caller already owns the loaded run to avoid cloning the full history.
    pub fn transition(
        &self,
        command: &Command,
        cached: Option<&CachedListingSnapshot>,
    ) -> Result<TransitionOutcome, EngineError> {
        self.clone().transition_owned(command, cached)
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one explicit match keeps every order-flow command on the same staged transition path"
    )]
    pub fn transition_owned(
        self,
        command: &Command,
        cached: Option<&CachedListingSnapshot>,
    ) -> Result<TransitionOutcome, EngineError> {
        let mut candidate = self;
        if candidate.run_id != command.run_id || candidate.sequence != command.expected_sequence {
            return Err(EngineError::SequenceConflict {
                current: candidate.sequence,
            });
        }
        let next_sequence = candidate
            .sequence
            .checked_add(EventSequence::new(1))
            .ok_or(EngineError::SequenceOverflow)?;
        let mut payloads = Vec::new();
        let mut changed_listings = BTreeSet::new();
        let mut changed_depth = BTreeMap::new();
        let (accepted, reject_code, order_id) = match &command.payload {
            CommandPayload::SubmitOrder(order)
            | CommandPayload::SubmitOrderAtListing { order, .. } => {
                let listing_key = match &command.payload {
                    CommandPayload::SubmitOrderAtListing { listing_key, .. } => {
                        if listing_key.instrument_id != order.instrument_id
                            || !candidate.listings.contains_key(listing_key)
                        {
                            return Err(EngineError::UnknownListing);
                        }
                        *listing_key
                    }
                    _ => candidate.listing_key_for_instrument(order.instrument_id)?,
                };
                let definition = candidate
                    .listings
                    .get(&listing_key)
                    .ok_or(EngineError::UnknownListing)?
                    .definition
                    .clone();
                let book = candidate.restore_book(listing_key, cached, command)?;
                payloads.push(EventPayload::OrderReceived {
                    order: order.clone(),
                    listing_key: Some(listing_key),
                });
                let outcome = if candidate.simulation.lifecycle != simulation::RunLifecycle::Active
                {
                    Err(RejectCode::RunNotActive)
                } else if candidate
                    .simulation
                    .halted_instruments
                    .contains(&order.instrument_id)
                {
                    Err(RejectCode::ListingHalted)
                } else {
                    let participant = candidate.participants.get(&order.participant_id).cloned();
                    let outcome = prepare_submit(
                        order,
                        &definition,
                        participant.as_ref(),
                        &book,
                        &mut candidate.ledger,
                        &candidate.participants,
                        &mut candidate.ownership,
                        &mut candidate.upstream_to_canonical,
                        &mut candidate.next_upstream_order_id,
                        &mut payloads,
                    )?;
                    let depth = candidate.replace_snapshot(listing_key, next_sequence, &book)?;
                    changed_depth.insert(listing_key, depth);
                    changed_listings.insert(listing_key);
                    outcome
                };
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
                if let Some(listing_key) = candidate
                    .ownership
                    .get(&cancel.order_id)
                    .map(|owned| owned.listing_key)
                {
                    let book = candidate.restore_book(listing_key, cached, command)?;
                    let outcome = prepare_cancel(
                        cancel,
                        CancelReason::Requested,
                        &book,
                        &mut candidate.ledger,
                        &mut candidate.ownership,
                        &mut payloads,
                    )?;
                    let depth = candidate.replace_snapshot(listing_key, next_sequence, &book)?;
                    changed_depth.insert(listing_key, depth);
                    changed_listings.insert(listing_key);
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
                } else {
                    payloads.push(EventPayload::OrderRejected {
                        order_id: Some(cancel.order_id),
                        code: RejectCode::UnknownOrder,
                    });
                    (
                        false,
                        Some("UnknownOrder".to_string()),
                        Some(cancel.order_id),
                    )
                }
            }
            CommandPayload::ActivateKillSwitch => {
                for listing_key in candidate.listings.keys().copied().collect::<Vec<_>>() {
                    let book = candidate.restore_book(listing_key, cached, command)?;
                    book.engage_kill_switch();
                    let depth = candidate.replace_snapshot(listing_key, next_sequence, &book)?;
                    changed_depth.insert(listing_key, depth);
                    changed_listings.insert(listing_key);
                }
                payloads.push(EventPayload::KillSwitchActivated);
                (true, None, None)
            }
            CommandPayload::NbcDone(done) => {
                if done.participant_id != command.actor {
                    return Err(EngineError::OwnershipInvariant);
                }
                let compatibility = candidate
                    .nbc_compatibility
                    .as_mut()
                    .ok_or(EngineError::NbcCompatibility)?;
                let advance = compatibility
                    .acknowledge_and_advance(done.participant_id, done.step)
                    .map_err(|_| EngineError::NbcCompatibility)?;
                payloads.push(EventPayload::NbcParticipantDone {
                    participant_id: done.participant_id,
                    step: done.step,
                });
                if let Some(advance) = advance {
                    payloads.push(EventPayload::NbcStepAdvanced {
                        executed_step: advance.executed_step(),
                        current_step: advance.current_step(),
                        triggered_event_ids: advance.triggered_event_ids().to_vec(),
                        completed: matches!(advance.status(), NbcRunStatus::Completed),
                    });
                }
                (true, None, None)
            }
        };
        candidate.finish(
            command,
            next_sequence,
            payloads,
            changed_depth,
            changed_listings,
            accepted,
            reject_code,
            order_id,
        )
    }

    /// Projects committed payloads and envelopes them as the next run sequence.
    #[expect(
        clippy::too_many_arguments,
        reason = "the shared commit tail receives every staged transition fact"
    )]
    fn finish(
        mut self,
        command: &Command,
        next_sequence: EventSequence,
        payloads: Vec<EventPayload>,
        changed_depth: BTreeMap<ListingKey, VisibleDepth>,
        changed_listings: BTreeSet<ListingKey>,
        accepted: bool,
        reject_code: Option<String>,
        order_id: Option<OrderId>,
    ) -> Result<TransitionOutcome, EngineError> {
        if payloads.len() > MAX_EVENTS_PER_TRANSITION {
            return Err(EngineError::EventBatchTooLarge);
        }
        for payload in &payloads {
            self.simulation
                .project_event(command.logical_time, payload)
                .map_err(EngineError::Simulation)?;
        }
        for (listing_key, depth) in changed_depth {
            self.refresh_market_projection(listing_key, depth)?;
        }
        let events = envelope(command, self.event_sequence, payloads)?;
        self.sequence = next_sequence;
        self.event_sequence = events
            .last()
            .map_or(self.event_sequence, |event| event.sequence);
        let snapshot_checksum = changed_listings
            .iter()
            .next()
            .and_then(|key| self.listings.get(key))
            .or_else(|| {
                (self.listings.len() == 1)
                    .then(|| self.listings.values().next())
                    .flatten()
            })
            .map(|listing| listing.snapshot.checksum.clone());
        Ok(TransitionOutcome {
            candidate: self,
            events,
            accepted,
            reject_code,
            order_id,
            snapshot_checksum,
            changed_listings,
        })
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

    #[expect(
        clippy::too_many_lines,
        reason = "mass cancel stages matcher books while other commands share the domain reducer"
    )]
    pub fn transition_simulation_owned(
        self,
        request: &SimulationCommandRequest,
    ) -> Result<TransitionOutcome, EngineError> {
        let metadata = Command {
            run_id: request.run_id,
            command_id: request.command_id,
            correlation_id: request.correlation_id,
            logical_time: request.logical_time,
            expected_sequence: request.expected_sequence,
            actor: request.actor,
            payload: CommandPayload::ActivateKillSwitch,
        };
        let mut candidate = self;
        if candidate.run_id != request.run_id || candidate.sequence != request.expected_sequence {
            return Err(EngineError::SequenceConflict {
                current: candidate.sequence,
            });
        }
        let next_sequence = candidate
            .sequence
            .checked_add(EventSequence::new(1))
            .ok_or(EngineError::SequenceOverflow)?;
        let mut payloads = Vec::new();
        let mut changed_listings = BTreeSet::new();
        let mut changed_depth = BTreeMap::new();
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
                .map(|owned| owned.order_id)
                .collect::<Vec<_>>();
            // Stage each touched matcher once. Reconstructing and snapshotting
            // after every canceled order turned one batch into O(orders)
            // full-book serializations; the authoritative candidate is still
            // discarded on any error before the transaction can commit.
            let mut staged_books = BTreeMap::new();
            for order_id in &order_ids {
                let owned = candidate
                    .ownership
                    .get(order_id)
                    .cloned()
                    .ok_or(EngineError::OwnershipInvariant)?;
                if let Entry::Vacant(vacant) = staged_books.entry(owned.listing_key) {
                    let book = candidate.restore_book(owned.listing_key, None, &metadata)?;
                    vacant.insert(book);
                }
                let book = staged_books
                    .get(&owned.listing_key)
                    .ok_or(EngineError::OwnershipInvariant)?;
                prepare_cancel(
                    &bunting_market_events::CancelOrder {
                        order_id: *order_id,
                        participant_id: owned.participant_id,
                    },
                    CancelReason::MassCancel,
                    book,
                    &mut candidate.ledger,
                    &mut candidate.ownership,
                    &mut payloads,
                )?
                .map_err(|_| EngineError::OwnershipInvariant)?;
            }
            for (listing_key, book) in staged_books {
                let depth = candidate.replace_snapshot(listing_key, next_sequence, &book)?;
                changed_depth.insert(listing_key, depth);
                changed_listings.insert(listing_key);
            }
            payloads.push(EventPayload::Simulation(
                bunting_market_events::SimulationEvent::MassCancelCompleted {
                    canceled_orders: order_ids,
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
        }
        candidate.finish(
            &metadata,
            next_sequence,
            payloads,
            changed_depth,
            changed_listings,
            true,
            None,
            None,
        )
    }

    fn refresh_market_projection(
        &mut self,
        listing_key: ListingKey,
        (bids, asks): VisibleDepth,
    ) -> Result<(), EngineError> {
        let convert = |levels: VisibleLevels| {
            levels
                .into_iter()
                .map(|(price, quantity)| {
                    Ok((
                        PriceTicks::new(i64::try_from(price).map_err(|_| EngineError::Accounting)?),
                        QuantityLots::new(
                            i64::try_from(quantity).map_err(|_| EngineError::Accounting)?,
                        ),
                    ))
                })
                .collect::<Result<Vec<_>, EngineError>>()
        };
        let mut raw_bids = Vec::new();
        let mut raw_asks = Vec::new();
        // One deterministic ownership scan supplies both raw venue depth and
        // private live/history projections. The old path scanned every owned
        // order twice for each changed listing.
        for owned in self
            .ownership
            .values()
            .filter(|owned| owned.listing_key == listing_key)
        {
            let private = self
                .simulation
                .private
                .entry(owned.participant_id)
                .or_default();
            if owned.state == OwnedOrderState::Active {
                let row = (owned.limit_price, owned.remaining_quantity, owned.order_id);
                match owned.side {
                    Side::Buy => raw_bids.push(row),
                    Side::Sell => raw_asks.push(row),
                }
                private.live_orders.insert(owned.order_id);
            } else {
                private.live_orders.remove(&owned.order_id);
                if !private.historical_orders.contains(&owned.order_id) {
                    if private.historical_orders.len() == simulation::MAX_TRADE_HISTORY {
                        private.historical_orders.pop_front();
                    }
                    private.historical_orders.push_back(owned.order_id);
                }
            }
        }
        raw_bids.sort_by(|left, right| right.0.cmp(&left.0).then(left.2.cmp(&right.2)));
        raw_asks.sort_by(|left, right| left.0.cmp(&right.0).then(left.2.cmp(&right.2)));
        let unique_listing = self
            .listings
            .keys()
            .filter(|key| key.instrument_id == listing_key.instrument_id)
            .count()
            == 1;
        self.simulation.set_depth(
            listing_key,
            unique_listing,
            raw_bids,
            raw_asks,
            convert(bids)?,
            convert(asks)?,
        );
        Ok(())
    }

    fn restore_book(
        &self,
        key: ListingKey,
        cached: Option<&CachedListingSnapshot>,
        command: &Command,
    ) -> Result<KernelBook, EngineError> {
        let listing = self.listings.get(&key).ok_or(EngineError::UnknownListing)?;
        let logical_millis = command.logical_time.get() / 1_000_000;
        if let Some(cached) = cached
            && cached.listing_key == key
            && cached.represented_sequence == listing.snapshot.represented_sequence
            && cached.checksum == listing.snapshot.checksum
            && let Ok(book) = KernelBook::restore_snapshot_json_at(
                &listing.definition.symbol,
                &cached.package_json,
                logical_millis,
            )
        {
            return Ok(book);
        }
        if listing.snapshot.schema_version != LISTING_SNAPSHOT_VERSION {
            return Err(EngineError::InvalidSnapshot);
        }
        KernelBook::restore_snapshot_json_at(
            &listing.definition.symbol,
            &listing.snapshot.package_json,
            logical_millis,
        )
        .map_err(|_| EngineError::InvalidSnapshot)
    }

    fn replace_snapshot(
        &mut self,
        key: ListingKey,
        sequence: EventSequence,
        book: &KernelBook,
    ) -> Result<VisibleDepth, EngineError> {
        let package = book
            .snapshot_package(SNAPSHOT_DEPTH)
            .map_err(|_| EngineError::Upstream)?;
        let depth = package.visible_depth.clone();
        let listing = self
            .listings
            .get_mut(&key)
            .ok_or(EngineError::UnknownListing)?;
        listing.snapshot = snapshot_from_package(sequence, package);
        Ok(depth)
    }
}

/// Optional immutable cache input for one listing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedListingSnapshot {
    pub listing_key: ListingKey,
    pub represented_sequence: EventSequence,
    pub checksum: String,
    pub package_json: String,
}

/// Candidate result of one authoritative engine transition.
#[derive(Clone, Debug)]
pub struct TransitionOutcome {
    pub candidate: RunState,
    pub events: Vec<EventEnvelope>,
    pub accepted: bool,
    pub reject_code: Option<String>,
    pub order_id: Option<OrderId>,
    pub snapshot_checksum: Option<String>,
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
        envelope.state.validate()?;
        if envelope.state.state_hash()? != envelope.state_hash {
            return Err(SnapshotError::HashMismatch);
        }
        Ok(envelope)
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
    InvalidSnapshot,
    SequenceConflict { current: EventSequence },
    SequenceOverflow,
    OwnershipInvariant,
    Accounting,
    EventBatchTooLarge,
    Upstream,
    NbcCompatibility,
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

#[expect(
    clippy::too_many_arguments,
    clippy::too_many_lines,
    reason = "submission stages admission, ledger reservation, OrderBook-rs matching and canonical events atomically"
)]
fn prepare_submit(
    order: &bunting_market_events::SubmitOrder,
    listing: &ListingDefinition,
    participant: Option<&ParticipantDefinition>,
    book: &KernelBook,
    ledger: &mut Ledger,
    participants: &BTreeMap<ParticipantId, ParticipantDefinition>,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    upstream_to_canonical: &mut BTreeMap<u64, OrderId>,
    next_upstream_order_id: &mut u64,
    payloads: &mut Vec<EventPayload>,
) -> Result<Result<(), RejectCode>, EngineError> {
    if ownership.contains_key(&order.order_id) {
        return Ok(Err(RejectCode::DuplicateOrderId));
    }
    if ownership.len() >= MAX_ORDERS {
        return Ok(Err(RejectCode::MaxOpenOrderQuantity));
    }
    if order.order_id.get() == 0 {
        return Ok(Err(RejectCode::InvalidOrderId));
    }
    let Some(participant) = participant else {
        return Ok(Err(RejectCode::ParticipantDisabled));
    };
    let Ok(upstream_quantity) = to_upstream_quantity(order.quantity) else {
        return Ok(Err(RejectCode::InvalidQuantity));
    };
    let listing_key = listing.key;
    let upstream_price = match order.kind {
        OrderKind::Limit { price }
        | OrderKind::LimitWithPolicy { price, .. }
        | OrderKind::AdvancedLimit { price, .. } => {
            let Ok(upstream_price) = to_upstream_price(price) else {
                return Ok(Err(RejectCode::PriceOutOfBounds));
            };
            Some(upstream_price)
        }
        OrderKind::Market => {
            if !book.has_opposite_liquidity(order.side) {
                return Ok(Err(RejectCode::InsufficientLiquidity));
            }
            None
        }
    };
    let market_bound = match order.side {
        Side::Buy => listing.price_bounds.max,
        Side::Sell => listing.price_bounds.min,
    };
    let admission = match bunting_risk_engine::admit(
        &participant.limits,
        participant.enabled,
        order,
        bunting_risk_engine::ListingTerms {
            price_bounds: listing.price_bounds,
            fee_bound: listing.fees.bound(),
        },
        ledger,
        order.kind.is_market().then_some(market_bound),
    ) {
        Ok(admission) => admission,
        Err(code) => return Ok(Err(code)),
    };
    let upstream_id = *next_upstream_order_id;
    let next_id = upstream_id
        .checked_add(1)
        .ok_or(EngineError::OwnershipInvariant)?;
    if upstream_to_canonical.contains_key(&upstream_id) {
        return Err(EngineError::OwnershipInvariant);
    }
    ledger.open_order(
        order.participant_id,
        order.instrument_id,
        order.side,
        order.quantity,
        admission.reservation,
    )?;
    *next_upstream_order_id = next_id;
    upstream_to_canonical.insert(upstream_id, order.order_id);
    ownership.insert(
        order.order_id,
        OwnedOrder {
            order_id: order.order_id,
            upstream_order_id: upstream_id,
            participant_id: order.participant_id,
            listing_key,
            side: order.side,
            limit_price: admission.reservation_price,
            original_quantity: order.quantity,
            remaining_quantity: order.quantity,
            state: OwnedOrderState::Active,
            reservation: admission.reservation,
        },
    );
    let trade_result = if let Some(price) = upstream_price {
        match order.kind {
            OrderKind::Limit { .. } => book.submit_limit(
                upstream_id,
                price,
                upstream_quantity,
                to_upstream_side(order.side),
                TimeInForce::Gtc,
            ),
            OrderKind::LimitWithPolicy { time_in_force, .. } => book.submit_limit(
                upstream_id,
                price,
                upstream_quantity,
                to_upstream_side(order.side),
                to_upstream_time_in_force(time_in_force),
            ),
            OrderKind::AdvancedLimit {
                time_in_force,
                policy,
                ..
            } => book.submit_advanced_limit(
                upstream_id,
                price,
                upstream_quantity,
                to_upstream_side(order.side),
                time_in_force,
                policy,
            ),
            OrderKind::Market => unreachable!("market orders have no upstream price"),
        }
        .map_err(|_| EngineError::Upstream)?
        .trade_result
    } else {
        Some(
            book.submit_market(upstream_id, upstream_quantity, to_upstream_side(order.side))
                .map_err(|_| EngineError::Upstream)?,
        )
    };
    payloads.push(EventPayload::OrderAccepted {
        order_id: order.order_id,
    });
    if let Some(trade_result) = trade_result {
        let engine_sequence = trade_result.engine_seq;
        let trade_info = TradeInfo::from_trade_result(&trade_result, None);
        apply_trades(
            order.order_id,
            engine_sequence,
            &trade_info,
            listing,
            ledger,
            participants,
            ownership,
            upstream_to_canonical,
            payloads,
        )?;
    }
    let remaining = ownership
        .get(&order.order_id)
        .ok_or(EngineError::OwnershipInvariant)?
        .remaining_quantity;
    if remaining.get() == 0 {
        payloads.push(EventPayload::OrderCompleted {
            order_id: order.order_id,
        });
    } else if let (Some(price), true) = (order.kind.limit_price(), book.contains(upstream_id)) {
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
        // Market remainders and IOC/FOK remainders never rest upstream.
        ledger.close_order(
            order.participant_id,
            order.instrument_id,
            order.side,
            remaining,
            admission.reservation,
        )?;
        if let Some(record) = ownership.get_mut(&order.order_id) {
            record.state = OwnedOrderState::Canceled;
            record.remaining_quantity = QuantityLots::new(0);
        }
        payloads.push(EventPayload::OrderCanceled {
            order_id: order.order_id,
            participant_id: order.participant_id,
            instrument_id: order.instrument_id,
            listing_key: Some(listing_key),
            remaining,
            reason: if order.kind.is_market() {
                CancelReason::MarketRemainder
            } else {
                CancelReason::Expired
            },
        });
    }
    Ok(Ok(()))
}

fn per_lot_fee(per_lot: MoneyMinor, quantity: QuantityLots) -> Result<MoneyMinor, EngineError> {
    per_lot
        .get()
        .checked_mul(i128::from(quantity.get()))
        .map(MoneyMinor::new)
        .ok_or(EngineError::Accounting)
}

#[expect(
    clippy::too_many_arguments,
    reason = "one fill couples ownership, settlement, the upstream map, and canonical trade events"
)]
fn apply_trades(
    taker_order_id: OrderId,
    engine_sequence: u64,
    trade_info: &TradeInfo,
    listing: &ListingDefinition,
    ledger: &mut Ledger,
    participants: &BTreeMap<ParticipantId, ParticipantDefinition>,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    upstream_to_canonical: &BTreeMap<u64, OrderId>,
    payloads: &mut Vec<EventPayload>,
) -> Result<(), EngineError> {
    let funded = |participant: ParticipantId| {
        participants
            .get(&participant)
            .is_some_and(|definition| definition.limits.cash_constrained)
    };
    for transaction in &trade_info.transactions {
        let maker_upstream = sequential_id_from_text(&transaction.maker_order_id)
            .ok_or(EngineError::OwnershipInvariant)?;
        let maker_id = upstream_to_canonical
            .get(&maker_upstream)
            .copied()
            .ok_or(EngineError::OwnershipInvariant)?;
        let maker = ownership
            .get(&maker_id)
            .filter(|owned| owned.upstream_order_id == maker_upstream)
            .cloned()
            .ok_or(EngineError::OwnershipInvariant)?;
        let taker = ownership
            .get(&taker_order_id)
            .cloned()
            .ok_or(EngineError::OwnershipInvariant)?;
        if maker.listing_key != taker.listing_key || maker.listing_key != listing.key {
            return Err(EngineError::OwnershipInvariant);
        }
        let quantity = QuantityLots::new(
            i64::try_from(transaction.quantity).map_err(|_| EngineError::Accounting)?,
        );
        let execution_price =
            PriceTicks::new(i64::try_from(transaction.price).map_err(|_| EngineError::Accounting)?);
        let maker_fee = per_lot_fee(listing.fees.maker_per_lot, quantity)?;
        let taker_fee = per_lot_fee(listing.fees.taker_per_lot, quantity)?;
        let party = |owned: &OwnedOrder, fee: MoneyMinor| FillParty {
            participant: owned.participant_id,
            fee,
            order: Some(owned.reservation),
            enforce_funding: funded(owned.participant_id),
        };
        let (buyer, seller, buyer_fee, seller_fee) = if taker.side == Side::Buy {
            (
                party(&taker, taker_fee),
                party(&maker, maker_fee),
                taker_fee,
                maker_fee,
            )
        } else {
            (
                party(&maker, maker_fee),
                party(&taker, taker_fee),
                maker_fee,
                taker_fee,
            )
        };
        ledger.settle(Fill {
            instrument: listing.key.instrument_id,
            price: execution_price,
            quantity,
            buyer: Some(buyer),
            seller: Some(seller),
            set_mark: true,
        })?;
        reduce_order(maker_id, quantity, ownership, payloads)?;
        reduce_order(taker_order_id, quantity, ownership, &mut Vec::new())?;
        payloads.push(EventPayload::TradeExecuted {
            instrument_id: listing.key.instrument_id,
            listing_key: Some(listing.key),
            maker_order_id: maker_id,
            taker_order_id,
            buyer_id: buyer.participant,
            seller_id: seller.participant,
            price: execution_price,
            quantity,
            buyer_fee,
            seller_fee,
            upstream_engine_sequence: engine_sequence,
        });
    }
    Ok(())
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

fn prepare_cancel(
    cancel: &bunting_market_events::CancelOrder,
    reason: CancelReason,
    book: &KernelBook,
    ledger: &mut Ledger,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<Result<(), RejectCode>, EngineError> {
    let Some(owned) = ownership.get(&cancel.order_id).cloned() else {
        return Ok(Err(RejectCode::UnknownOrder));
    };
    if owned.participant_id != cancel.participant_id {
        return Ok(Err(RejectCode::NotOrderOwner));
    }
    if owned.state != OwnedOrderState::Active {
        return Ok(Err(RejectCode::UnknownOrder));
    }
    let canceled = book
        .cancel_remaining(owned.upstream_order_id)
        .map_err(|_| EngineError::Upstream)?;
    let Some(upstream_remaining) = canceled else {
        return Err(EngineError::OwnershipInvariant);
    };
    if upstream_remaining
        != u64::try_from(owned.remaining_quantity.get()).map_err(|_| EngineError::Accounting)?
    {
        return Err(EngineError::OwnershipInvariant);
    }
    ledger.close_order(
        owned.participant_id,
        owned.listing_key.instrument_id,
        owned.side,
        owned.remaining_quantity,
        owned.reservation,
    )?;
    if let Some(record) = ownership.get_mut(&cancel.order_id) {
        record.state = OwnedOrderState::Canceled;
        record.remaining_quantity = QuantityLots::new(0);
    }
    payloads.push(EventPayload::OrderCanceled {
        order_id: owned.order_id,
        participant_id: owned.participant_id,
        instrument_id: owned.listing_key.instrument_id,
        listing_key: Some(owned.listing_key),
        remaining: owned.remaining_quantity,
        reason,
    });
    Ok(Ok(()))
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
            let offset = u64::try_from(index + 1).map_err(|_| EngineError::EventBatchTooLarge)?;
            let sequence = current_event_sequence
                .get()
                .checked_add(offset)
                .map(EventSequence::new)
                .ok_or(EngineError::EventBatchTooLarge)?;
            let event_id = command
                .command_id
                .get()
                .checked_add(u128::from(offset))
                .map(EventId::new)
                .ok_or(EngineError::EventBatchTooLarge)?;
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

fn snapshot_from_package(sequence: EventSequence, package: SnapshotPackage) -> ListingSnapshot {
    ListingSnapshot {
        schema_version: LISTING_SNAPSHOT_VERSION,
        represented_sequence: sequence,
        checksum: package.checksum,
        package_json: package.json,
    }
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
    use bunting_market_events::TimeInForcePolicy;
    use bunting_market_events::{NbcDone, SubmitOrder};
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

    fn at_listing(mut command: Command, listing_key: ListingKey) -> Command {
        if let CommandPayload::SubmitOrder(order) = command.payload {
            command.payload = CommandPayload::SubmitOrderAtListing { listing_key, order };
        }
        command
    }

    fn apply(state: RunState, command: &Command) -> TransitionOutcome {
        state.transition_owned(command, None).unwrap()
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
            .listing_snapshot(ListingKey::new(VenueId::new(1), InstrumentId::new(1)))
            .unwrap()
            .package_json
            .clone();
        let mut fok = submit(&state, 2, 1, 2, 1, Side::Buy, 102, 10);
        if let CommandPayload::SubmitOrder(order) = &mut fok.payload {
            order.kind = OrderKind::LimitWithPolicy {
                price: PriceTicks::new(102),
                time_in_force: TimeInForcePolicy::Fok,
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
        assert_eq!(resting.1, vec![(101, 3)]);
        assert!(!book_before.is_empty());
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
        assert_ne!(
            state.ownership()[&low].upstream_order_id,
            state.ownership()[&high].upstream_order_id
        );
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
            vec![(110, 2)]
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
            state.transition(&legacy, None),
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
        let outcome = state.transition(&sell, None).unwrap();
        assert!(outcome.accepted);
        state = outcome.candidate;
        let buy_elsewhere = target(&state, secondary, 202, 1, Side::Buy);
        let outcome = state.transition(&buy_elsewhere, None).unwrap();
        assert!(outcome.accepted);
        assert!(
            !outcome
                .events
                .iter()
                .any(|event| { matches!(event.payload, EventPayload::TradeExecuted { .. }) })
        );
        state = outcome.candidate;
        assert_eq!(
            state.simulation().market_by_listing[&primary].aggregated_asks,
            vec![(PriceTicks::new(120), QuantityLots::new(1))]
        );
        assert_eq!(
            state.simulation().market_by_listing[&secondary].aggregated_bids,
            vec![(PriceTicks::new(120), QuantityLots::new(1))]
        );
        let buy_primary = target(&state, primary, 203, 1, Side::Buy);
        let outcome = state.transition(&buy_primary, None).unwrap();
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
            state.simulation().market_by_listing[&secondary]
                .trades
                .is_empty()
        );
        assert_eq!(
            state.simulation().market_by_listing[&secondary].aggregated_bids,
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
        let borrowed = state.transition(&sell, None).unwrap();
        let owned = state.clone().transition_owned(&sell, None).unwrap();
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
        let borrowed = after_submit.transition(&cancel, None).unwrap();
        let owned = after_submit
            .clone()
            .transition_owned(&cancel, None)
            .unwrap();
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
    fn every_order_uses_one_sequential_upstream_allocator() {
        let large = OrderId::new(u128::from(u64::MAX) + 123);
        let small = OrderId::new(1);
        let mut state = run();
        let first = submit(&state, 1, 2, large.get(), 1, Side::Sell, 110, 2);
        let accepted = state.transition(&first, None).unwrap();
        assert!(accepted.accepted);
        state = accepted.candidate;
        assert_eq!(state.ownership()[&large].upstream_order_id, 1);
        assert_eq!(state.upstream_to_canonical.get(&1), Some(&large));
        assert_eq!(state.next_upstream_order_id, 2);

        let next = submit(&state, 2, 2, small.get(), 1, Side::Sell, 120, 1);
        let accepted = state.transition(&next, None).unwrap();
        assert!(accepted.accepted);
        state = accepted.candidate;
        assert_eq!(state.ownership()[&small].upstream_order_id, 2);
        assert_eq!(state.upstream_to_canonical.get(&2), Some(&small));

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
        let filled = state.transition(&first_buy, None).unwrap();
        assert!(filled.accepted);
        assert!(filled.events.iter().any(|event| matches!(
            &event.payload,
            EventPayload::TradeExecuted {
                maker_order_id,
                taker_order_id,
                ..
            } if *maker_order_id == large && *taker_order_id == OrderId::new(123)
        )));
        let replayed = replay_start.transition(&first_buy, None).unwrap();
        assert_eq!(
            replayed.candidate.state_hash().unwrap(),
            filled.candidate.state_hash().unwrap()
        );
        state = filled.candidate;
        assert_eq!(
            state.upstream_to_canonical.get(&3),
            Some(&OrderId::new(123))
        );
        assert_eq!(state.next_upstream_order_id, 4);

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
        let cancelled = state.transition(&cancel, None).unwrap();
        assert!(cancelled.accepted);
        assert!(cancelled.events.iter().any(|event| matches!(
            event.payload,
            EventPayload::OrderCanceled { order_id, .. } if order_id == small
        )));
        assert_eq!(
            cancelled.candidate.ownership()[&small].state,
            OwnedOrderState::Canceled
        );
        assert_eq!(
            cancelled.candidate.upstream_to_canonical.get(&1),
            Some(&large)
        );
        assert_eq!(cancelled.candidate.next_upstream_order_id, 4);
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
            let accepted = state.transition(&next, None).unwrap();
            assert!(accepted.accepted);
            state = accepted.candidate;
        }
        let untouched = ListingKey::new(VenueId::new(1), InstrumentId::new(2));
        let removed = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let untouched_snapshot = state.listing_snapshot(untouched).unwrap().clone();
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
            finished.candidate.listing_snapshot(untouched).unwrap(),
            &untouched_snapshot
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
            vec![(95, 1)]
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
        value["schema_version"] = serde_json::json!(1);
        value["state"]
            .as_object_mut()
            .unwrap()
            .remove("next_upstream_order_id");
        value["state"]
            .as_object_mut()
            .unwrap()
            .remove("upstream_to_canonical");
        assert_eq!(
            EngineSnapshotEnvelope::from_json(&value.to_string()),
            Err(SnapshotError::UnsupportedVersion)
        );
    }

    #[test]
    fn two_listings_are_isolated_and_iteration_is_deterministic() {
        let state = run();
        let before_two = state
            .listing_snapshot(ListingKey::new(VenueId::new(1), InstrumentId::new(2)))
            .unwrap()
            .clone();
        let outcome = state
            .transition(&submit(&state, 1, 1, 1, 1, Side::Buy, 100, 10), None)
            .unwrap();
        assert_eq!(outcome.candidate.sequence(), EventSequence::new(1));
        assert_eq!(
            outcome
                .candidate
                .listing_snapshot(ListingKey::new(VenueId::new(1), InstrumentId::new(2)))
                .unwrap(),
            &before_two
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
            .transition(&submit(&state, 1, 1, 1, 1, Side::Sell, 100, 10), None)
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
        state
            .listings
            .get_mut(&ListingKey::new(VenueId::new(1), InstrumentId::new(1)))
            .unwrap()
            .snapshot
            .package_json = "{}".to_string();
        let before = state.state_hash().unwrap();
        let command = submit(&state, 1, 1, 1, 1, Side::Buy, 100, 10);
        assert!(state.transition(&command, None).is_err());
        assert_eq!(state.state_hash().unwrap(), before);
    }

    #[test]
    fn snapshot_plus_replayed_commands_matches_uninterrupted_state() {
        let state = run();
        let first_command = submit(&state, 1, 1, 1, 1, Side::Sell, 100, 10);
        let first = state.transition(&first_command, None).unwrap().candidate;
        let restored = EngineSnapshotEnvelope::from_json(
            &first.snapshot_envelope().unwrap().to_json().unwrap(),
        )
        .unwrap()
        .state;
        let second_command = submit(&first, 2, 2, 2, 1, Side::Buy, 100, 4);
        let uninterrupted = first.transition(&second_command, None).unwrap().candidate;
        let replayed = restored
            .transition(&second_command, None)
            .unwrap()
            .candidate;
        assert_eq!(uninterrupted.state_hash(), replayed.state_hash());
    }

    #[test]
    fn market_order_executes_through_orderbook_rs_and_completes() {
        let state = run();
        let resting = state
            .transition(&submit(&state, 1, 1, 1, 1, Side::Sell, 101, 10), None)
            .unwrap()
            .candidate;
        let outcome = resting
            .transition(&submit_market(&resting, 2, 2, 2, 1, Side::Buy, 4), None)
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
            vec![(101, 6)]
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

    #[test]
    fn nbc_done_barrier_advances_only_after_every_participant() {
        let config = NbcScenarioConfig::from_json(include_bytes!(
            "../../../tests/conformance/nbc/config/normal-market.input.v1.json"
        ))
        .unwrap();
        let state = run()
            .with_nbc_compatibility(
                config,
                vec![NbcScheduledEvent::new("event-before-traders", 0).unwrap()],
            )
            .unwrap();
        let done = |state: &RunState, command_id: u128, participant_id: u128| Command {
            run_id: state.run_id(),
            command_id: CommandId::new(command_id),
            correlation_id: CorrelationId::new(command_id),
            logical_time: LogicalTimeNs::new(u64::try_from(command_id).unwrap()),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(participant_id),
            payload: CommandPayload::NbcDone(NbcDone {
                participant_id: ParticipantId::new(participant_id),
                step: 0,
            }),
        };

        let first = state.transition(&done(&state, 800, 1), None).unwrap();
        assert_eq!(first.events.len(), 1);
        assert_eq!(
            first
                .candidate
                .nbc_compatibility()
                .unwrap()
                .scheduler
                .current_step(),
            0
        );
        let second = first
            .candidate
            .transition(&done(&first.candidate, 801, 2), None)
            .unwrap();
        assert_eq!(second.events.len(), 2);
        assert!(matches!(
            &second.events[1].payload,
            EventPayload::NbcStepAdvanced {
                executed_step: 0,
                current_step: 1,
                triggered_event_ids,
                completed: false,
            } if triggered_event_ids == &["event-before-traders"]
        ));
        let envelope = second.candidate.snapshot_envelope().unwrap();
        assert_eq!(
            EngineSnapshotEnvelope::from_json(&envelope.to_json().unwrap()).unwrap(),
            envelope
        );
    }
}
