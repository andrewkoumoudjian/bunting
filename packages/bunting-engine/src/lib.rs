#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Authoritative sans-I/O Bunting market-simulation engine.

pub mod compatibility;
mod matching;

use bunting_ledger::{
    Account, AccountProjection, Holding, HoldingProjection, Ledger, LedgerError, TradeSettlement,
};
use bunting_market_events::{
    CancelReason, Command, CommandPayload, EVENT_SCHEMA_VERSION, EventEnvelope, EventPayload,
    OrderKind, RejectCode, Side,
};
use bunting_market_types::{
    EventId, EventSequence, InstrumentId, IterationId, ListingKey, MoneyMinor, OrderId,
    ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId, ScenarioVersion,
    VenueId,
};
use bunting_risk_engine::{RiskLimits, RiskState};
use compatibility::nbc::{
    NBC_TRANSLATION_VERSION, NbcCompatibilityState, RunStatus as NbcRunStatus,
    ScenarioConfig as NbcScenarioConfig, ScheduledEvent as NbcScheduledEvent,
};
use matching::{
    KernelBook, SnapshotPackage, TimeInForce, TradeInfo, sequential_id_from_text,
    to_upstream_order_id, to_upstream_price, to_upstream_quantity, to_upstream_side,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub use matching::{ORDERBOOK_RS_AUDIT_COMMIT, ORDERBOOK_RS_VERSION};

/// Version of the central engine behavior established by this foundation slice.
pub const ENGINE_VERSION: u16 = 1;
/// Version of the complete persisted engine snapshot envelope.
pub const ENGINE_SNAPSHOT_VERSION: u16 = 1;
/// Version of the minimal Bunting-native scenario schema.
pub const SCENARIO_SCHEMA_VERSION: u16 = 1;
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

/// Immutable venue-specific listing input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListingDefinition {
    key: ListingKey,
    symbol: String,
    price_bounds: PriceBounds,
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
        })
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
}

/// Immutable participant input required by the foundation run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ParticipantDefinition {
    participant_id: ParticipantId,
    enabled: bool,
    limits: RiskLimits,
    initial_cash: MoneyMinor,
    /// Canonically ordered initial positions.
    initial_positions: BTreeMap<InstrumentId, QuantityLots>,
}

impl ParticipantDefinition {
    #[must_use]
    pub const fn new(
        participant_id: ParticipantId,
        enabled: bool,
        limits: RiskLimits,
        initial_cash: MoneyMinor,
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
    pub const fn initial_cash(&self) -> MoneyMinor {
        self.initial_cash
    }

    #[must_use]
    pub fn initial_positions(&self) -> &BTreeMap<InstrumentId, QuantityLots> {
        &self.initial_positions
    }
}

/// Minimal immutable scenario definition used to instantiate a run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioDefinition {
    schema_version: u16,
    scenario_id: ScenarioId,
    scenario_version: ScenarioVersion,
    /// Canonically ordered by listing identity.
    listings: BTreeMap<ListingKey, ListingDefinition>,
    /// Canonically ordered by participant identity.
    participants: BTreeMap<ParticipantId, ParticipantDefinition>,
}

impl ScenarioDefinition {
    pub fn new(
        scenario_id: ScenarioId,
        scenario_version: ScenarioVersion,
        listings: impl IntoIterator<Item = ListingDefinition>,
        participants: impl IntoIterator<Item = ParticipantDefinition>,
    ) -> Result<Self, ScenarioError> {
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
        let definition = Self {
            schema_version: SCENARIO_SCHEMA_VERSION,
            scenario_id,
            scenario_version,
            listings: listing_map,
            participants: participant_map,
        };
        definition.validate()?;
        Ok(definition)
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
        if self.participants.len() > MAX_PARTICIPANTS {
            return Err(ScenarioError::ParticipantBound);
        }
        for (key, listing) in &self.listings {
            if key.venue_id.get() == 0
                || key.instrument_id.get() == 0
                || *key != listing.key
                || listing.symbol.is_empty()
                || listing.symbol.len() > 128
            {
                return Err(ScenarioError::InvalidListing);
            }
            listing
                .price_bounds
                .validate(listing.price_bounds.min)
                .map_err(|_| ScenarioError::InvalidListing)?;
        }
        for (participant_id, participant) in &self.participants {
            if *participant_id != participant.participant_id {
                return Err(ScenarioError::InvalidParticipant);
            }
            if participant.initial_positions.len() > MAX_LISTINGS
                || participant.initial_positions.keys().any(|instrument| {
                    !self
                        .listings
                        .keys()
                        .any(|key| key.instrument_id == *instrument)
                })
            {
                return Err(ScenarioError::InvalidParticipant);
            }
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
    pub fn participants(&self) -> &BTreeMap<ParticipantId, ParticipantDefinition> {
        &self.participants
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScenarioError {
    UnsupportedSchemaVersion,
    ListingBound,
    ParticipantBound,
    DuplicateListing,
    DuplicateParticipant,
    InvalidListing,
    InvalidParticipant,
    InvalidScenarioIdentity,
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
    listings: BTreeMap<ListingKey, ListingState>,
    participants: BTreeMap<ParticipantId, ParticipantDefinition>,
    accounts: AccountProjection,
    holdings: HoldingProjection,
    ownership: BTreeMap<OrderId, OwnedOrder>,
    #[serde(default)]
    nbc_compatibility: Option<NbcCompatibilityState>,
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
        let accounts = scenario
            .participants
            .values()
            .map(|participant| {
                (
                    participant.participant_id,
                    Account {
                        cash: participant.initial_cash,
                        reserved_cash: MoneyMinor::new(0),
                    },
                )
            })
            .collect();
        let holdings = scenario
            .participants
            .values()
            .flat_map(|participant| {
                participant
                    .initial_positions
                    .iter()
                    .map(|(instrument, quantity)| {
                        (
                            participant.participant_id,
                            *instrument,
                            Holding {
                                position: *quantity,
                                reserved_inventory: QuantityLots::new(0),
                            },
                        )
                    })
            })
            .collect();
        Ok(Self {
            run_id,
            sequence: EventSequence::new(0),
            event_sequence: EventSequence::new(0),
            iteration_id,
            scenario_id: scenario.scenario_id,
            scenario_version: scenario.scenario_version,
            scenario_hash: scenario.content_hash()?,
            config: EngineConfig::default(),
            listings,
            participants: scenario.participants.clone(),
            accounts,
            holdings,
            ownership: BTreeMap::new(),
            nbc_compatibility: None,
        })
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

    #[must_use]
    pub fn accounts(&self) -> &AccountProjection {
        &self.accounts
    }

    #[must_use]
    pub fn holdings(&self) -> &HoldingProjection {
        &self.holdings
    }

    #[must_use]
    pub fn ownership(&self) -> &BTreeMap<OrderId, OwnedOrder> {
        &self.ownership
    }

    #[must_use]
    pub const fn nbc_compatibility(&self) -> Option<&NbcCompatibilityState> {
        self.nbc_compatibility.as_ref()
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
        {
            return Err(SnapshotError::UnsupportedVersion);
        }
        Ok(())
    }

    pub fn snapshot_envelope(&self) -> Result<EngineSnapshotEnvelope, SnapshotError> {
        EngineSnapshotEnvelope::new(self.clone())
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one explicit match keeps every foundation command on the same staged transition path"
    )]
    pub fn transition(
        &self,
        command: &Command,
        cached: Option<&CachedListingSnapshot>,
    ) -> Result<TransitionOutcome, EngineError> {
        if self.run_id != command.run_id || self.sequence != command.expected_sequence {
            return Err(EngineError::SequenceConflict {
                current: self.sequence,
            });
        }
        let next_sequence = self
            .sequence
            .checked_add(EventSequence::new(1))
            .ok_or(EngineError::SequenceOverflow)?;
        let mut candidate = self.clone();
        let mut ledger = Ledger::from_projection(self.accounts.clone(), self.holdings.clone());
        let risk = self.restore_risk();
        let mut payloads = Vec::new();
        let mut changed_listings = BTreeSet::new();
        let (accepted, reject_code, order_id) = match &command.payload {
            CommandPayload::SubmitOrder(order) => {
                let listing_key = self.listing_key_for_instrument(order.instrument_id)?;
                let price_bounds = self
                    .listings
                    .get(&listing_key)
                    .ok_or(EngineError::UnknownListing)?
                    .definition
                    .price_bounds;
                let book = self.restore_book(listing_key, cached, command)?;
                payloads.push(EventPayload::OrderReceived {
                    order: order.clone(),
                });
                let outcome = prepare_submit(
                    order,
                    listing_key,
                    price_bounds,
                    &book,
                    &mut ledger,
                    &risk,
                    &mut candidate.ownership,
                    &mut payloads,
                )?;
                candidate.replace_snapshot(listing_key, next_sequence, &book)?;
                changed_listings.insert(listing_key);
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
                if let Some(listing_key) = self
                    .ownership
                    .get(&cancel.order_id)
                    .map(|owned| owned.listing_key)
                {
                    let book = self.restore_book(listing_key, cached, command)?;
                    let outcome = prepare_cancel(
                        cancel,
                        &book,
                        &mut ledger,
                        &mut candidate.ownership,
                        &mut payloads,
                    )?;
                    candidate.replace_snapshot(listing_key, next_sequence, &book)?;
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
                for listing_key in self.listings.keys().copied() {
                    let book = self.restore_book(listing_key, cached, command)?;
                    book.engage_kill_switch();
                    candidate.replace_snapshot(listing_key, next_sequence, &book)?;
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
        if payloads.len() > MAX_EVENTS_PER_TRANSITION {
            return Err(EngineError::EventBatchTooLarge);
        }
        let events = envelope(command, self.event_sequence, payloads)?;
        candidate.sequence = next_sequence;
        candidate.event_sequence = events
            .last()
            .map_or(self.event_sequence, |event| event.sequence);
        (candidate.accounts, candidate.holdings) = ledger.projection();
        let snapshot_checksum = changed_listings
            .iter()
            .next()
            .and_then(|key| candidate.listings.get(key))
            .or_else(|| {
                (candidate.listings.len() == 1)
                    .then(|| candidate.listings.values().next())
                    .flatten()
            })
            .map(|listing| listing.snapshot.checksum.clone());
        Ok(TransitionOutcome {
            candidate,
            events,
            accepted,
            reject_code,
            order_id,
            snapshot_checksum,
            changed_listings,
        })
    }

    fn restore_risk(&self) -> RiskState {
        let mut risk = RiskState::new();
        for listing in self.listings.values() {
            risk.configure_instrument(
                listing.definition.key.instrument_id,
                listing.definition.price_bounds,
            );
        }
        for participant in self.participants.values() {
            risk.configure_participant(participant.participant_id, participant.limits);
            risk.set_enabled(participant.participant_id, participant.enabled);
        }
        risk
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
    ) -> Result<(), EngineError> {
        let listing = self
            .listings
            .get_mut(&key)
            .ok_or(EngineError::UnknownListing)?;
        listing.snapshot = snapshot_from_package(
            sequence,
            book.snapshot_package(SNAPSHOT_DEPTH)
                .map_err(|_| EngineError::Upstream)?,
        );
        Ok(())
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
        let envelope: Self =
            serde_json::from_str(json).map_err(|_| SnapshotError::Serialization)?;
        if envelope.schema_version != ENGINE_SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion);
        }
        envelope.state.validate()?;
        if envelope.state.state_hash()? != envelope.state_hash {
            return Err(SnapshotError::HashMismatch);
        }
        Ok(envelope)
    }

    /// Restores the current envelope or deterministically migrates the legacy
    /// one-listing projection written before the unified engine existed.
    pub fn from_persisted_json(json: &str) -> Result<Self, SnapshotError> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|_| SnapshotError::Serialization)?;
        if value.get("schema_version").is_some() {
            return Self::from_json(json);
        }
        let legacy: LegacyRunState =
            serde_json::from_value(value).map_err(|_| SnapshotError::Serialization)?;
        if legacy.snapshot.instrument_id != legacy.instrument_id
            || legacy.snapshot.represented_sequence != legacy.version
            || legacy
                .ownership
                .iter()
                .any(|owned| owned.instrument_id != legacy.instrument_id)
        {
            return Err(SnapshotError::InvalidScenario);
        }
        let listing_key = ListingKey::new(VenueId::new(1), legacy.instrument_id);
        let participants = legacy
            .participants
            .iter()
            .map(|participant| {
                let cash = legacy
                    .accounts
                    .iter()
                    .find(|(id, _)| id == &participant.participant_id)
                    .map_or(MoneyMinor::new(0), |(_, account)| account.cash);
                let initial_positions = legacy
                    .holdings
                    .iter()
                    .filter(|(id, _, _)| id == &participant.participant_id)
                    .map(|(_, instrument, holding)| (*instrument, holding.position))
                    .collect();
                ParticipantDefinition {
                    participant_id: participant.participant_id,
                    enabled: participant.enabled,
                    limits: participant.limits,
                    initial_cash: cash,
                    initial_positions,
                }
            })
            .collect::<Vec<_>>();
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(legacy.run_id.get()),
            ScenarioVersion::new(1),
            [ListingDefinition {
                key: listing_key,
                symbol: legacy.symbol,
                price_bounds: legacy.price_bounds,
            }],
            participants,
        )
        .map_err(|_| SnapshotError::InvalidScenario)?;
        let mut state = RunState::from_scenario(legacy.run_id, IterationId::new(1), &scenario)
            .map_err(|_| SnapshotError::InvalidScenario)?;
        state.sequence = legacy.version;
        state.event_sequence = legacy.version;
        state.accounts = legacy.accounts;
        state.holdings = legacy.holdings;
        state.ownership = legacy
            .ownership
            .into_iter()
            .map(|owned| {
                (
                    owned.order_id,
                    OwnedOrder {
                        order_id: owned.order_id,
                        upstream_order_id: owned.upstream_order_id,
                        participant_id: owned.participant_id,
                        listing_key,
                        side: owned.side,
                        limit_price: owned.limit_price,
                        original_quantity: owned.original_quantity,
                        remaining_quantity: owned.remaining_quantity,
                        state: owned.state,
                    },
                )
            })
            .collect();
        state.nbc_compatibility = None;
        let listing = state
            .listings
            .get_mut(&listing_key)
            .ok_or(SnapshotError::InvalidScenario)?;
        listing.snapshot = ListingSnapshot {
            schema_version: LISTING_SNAPSHOT_VERSION,
            represented_sequence: legacy.snapshot.represented_sequence,
            checksum: legacy.snapshot.checksum,
            package_json: legacy.snapshot.package_json,
        };
        Self::new(state)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyRunState {
    run_id: RunId,
    version: EventSequence,
    instrument_id: InstrumentId,
    symbol: String,
    price_bounds: PriceBounds,
    participants: Vec<LegacyParticipantConfig>,
    accounts: AccountProjection,
    holdings: HoldingProjection,
    ownership: Vec<LegacyOwnedOrder>,
    snapshot: LegacySnapshotRecord,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyParticipantConfig {
    participant_id: ParticipantId,
    enabled: bool,
    limits: RiskLimits,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyOwnedOrder {
    order_id: OrderId,
    upstream_order_id: u64,
    participant_id: ParticipantId,
    instrument_id: InstrumentId,
    side: Side,
    limit_price: PriceTicks,
    original_quantity: QuantityLots,
    remaining_quantity: QuantityLots,
    state: OwnedOrderState,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacySnapshotRecord {
    instrument_id: InstrumentId,
    represented_sequence: EventSequence,
    checksum: String,
    package_json: String,
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
    reason = "submission stages risk, OrderBook-rs matching, ledger effects, and canonical events atomically"
)]
fn prepare_submit(
    order: &bunting_market_events::SubmitOrder,
    listing_key: ListingKey,
    price_bounds: PriceBounds,
    book: &KernelBook,
    ledger: &mut Ledger,
    risk: &RiskState,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<Result<(), RejectCode>, EngineError> {
    if ownership.contains_key(&order.order_id) {
        return Ok(Err(RejectCode::DuplicateOrderId));
    }
    if ownership.len() >= MAX_ORDERS {
        return Ok(Err(RejectCode::MaxOpenOrderQuantity));
    }
    let Ok(upstream_id) = to_upstream_order_id(order.order_id) else {
        return Ok(Err(RejectCode::InvalidOrderId));
    };
    let Ok(upstream_quantity) = to_upstream_quantity(order.quantity) else {
        return Ok(Err(RejectCode::InvalidQuantity));
    };
    let (reservation_price, upstream_price) = match order.kind {
        OrderKind::Limit { price } => {
            let Ok(upstream_price) = to_upstream_price(price) else {
                return Ok(Err(RejectCode::PriceOutOfBounds));
            };
            (price, Some(upstream_price))
        }
        OrderKind::Market => {
            if !book.has_opposite_liquidity(order.side) {
                return Ok(Err(RejectCode::InsufficientLiquidity));
            }
            let price = match order.side {
                Side::Buy => price_bounds.max,
                Side::Sell => price_bounds.min,
            };
            (price, None)
        }
    };
    let open_quantity = ownership
        .values()
        .filter(|owned| {
            owned.participant_id == order.participant_id
                && owned.listing_key.instrument_id == order.instrument_id
                && owned.state == OwnedOrderState::Active
        })
        .try_fold(QuantityLots::new(0), |total, owned| {
            total.checked_add(owned.remaining_quantity)
        })
        .ok_or(EngineError::Accounting)?;
    let reservation_price = match risk.check(
        order,
        open_quantity,
        ledger,
        (order.kind == OrderKind::Market).then_some(reservation_price),
    ) {
        Ok(value) => value,
        Err(code) => return Ok(Err(code)),
    };
    ledger.reserve(
        order.participant_id,
        order.instrument_id,
        order.side,
        reservation_price,
        order.quantity,
    )?;
    ownership.insert(
        order.order_id,
        OwnedOrder {
            order_id: order.order_id,
            upstream_order_id: upstream_id,
            participant_id: order.participant_id,
            listing_key,
            side: order.side,
            limit_price: reservation_price,
            original_quantity: order.quantity,
            remaining_quantity: order.quantity,
            state: OwnedOrderState::Active,
        },
    );
    let trade_result = if let Some(price) = upstream_price {
        book.submit_limit(
            upstream_id,
            price,
            upstream_quantity,
            to_upstream_side(order.side),
            TimeInForce::Gtc,
        )
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
            ledger,
            ownership,
            payloads,
        )?;
    }
    let remaining = ownership
        .get(&order.order_id)
        .ok_or(EngineError::OwnershipInvariant)?
        .remaining_quantity;
    if remaining.get() == 0 {
        if let Some(taker) = ownership.get_mut(&order.order_id) {
            taker.state = OwnedOrderState::Filled;
        }
        payloads.push(EventPayload::OrderCompleted {
            order_id: order.order_id,
        });
    } else if let OrderKind::Limit { price } = order.kind {
        payloads.push(EventPayload::OrderRested {
            order_id: order.order_id,
            participant_id: order.participant_id,
            instrument_id: order.instrument_id,
            side: order.side,
            price,
            remaining,
        });
    } else {
        ledger.release(
            order.participant_id,
            order.instrument_id,
            order.side,
            reservation_price,
            remaining,
        )?;
        if let Some(record) = ownership.get_mut(&order.order_id) {
            record.state = OwnedOrderState::Canceled;
            record.remaining_quantity = QuantityLots::new(0);
        }
        payloads.push(EventPayload::OrderCanceled {
            order_id: order.order_id,
            participant_id: order.participant_id,
            instrument_id: order.instrument_id,
            remaining,
            reason: CancelReason::MarketRemainder,
        });
    }
    Ok(Ok(()))
}

fn apply_trades(
    taker_order_id: OrderId,
    engine_sequence: u64,
    trade_info: &TradeInfo,
    ledger: &mut Ledger,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<(), EngineError> {
    for transaction in &trade_info.transactions {
        let maker_upstream = sequential_id_from_text(&transaction.maker_order_id)
            .ok_or(EngineError::OwnershipInvariant)?;
        let maker_id = ownership
            .values()
            .find(|owned| owned.upstream_order_id == maker_upstream)
            .map(|owned| owned.order_id)
            .ok_or(EngineError::OwnershipInvariant)?;
        let maker = ownership
            .get(&maker_id)
            .cloned()
            .ok_or(EngineError::OwnershipInvariant)?;
        let taker = ownership
            .get(&taker_order_id)
            .cloned()
            .ok_or(EngineError::OwnershipInvariant)?;
        if maker.listing_key != taker.listing_key {
            return Err(EngineError::OwnershipInvariant);
        }
        let quantity = QuantityLots::new(
            i64::try_from(transaction.quantity).map_err(|_| EngineError::Accounting)?,
        );
        let execution_price =
            PriceTicks::new(i64::try_from(transaction.price).map_err(|_| EngineError::Accounting)?);
        let (buyer, seller, buyer_limit, seller_limit) = if taker.side == Side::Buy {
            (
                taker.participant_id,
                maker.participant_id,
                taker.limit_price,
                maker.limit_price,
            )
        } else {
            (
                maker.participant_id,
                taker.participant_id,
                maker.limit_price,
                taker.limit_price,
            )
        };
        ledger.settle_trade(TradeSettlement {
            buyer,
            seller,
            instrument: taker.listing_key.instrument_id,
            buyer_limit,
            seller_limit,
            execution_price,
            quantity,
        })?;
        reduce_order(maker_id, quantity, ownership, payloads)?;
        reduce_order(taker_order_id, quantity, ownership, &mut Vec::new())?;
        payloads.push(EventPayload::TradeExecuted {
            instrument_id: taker.listing_key.instrument_id,
            maker_order_id: maker_id,
            taker_order_id,
            buyer_id: buyer,
            seller_id: seller,
            price: execution_price,
            quantity,
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
    ledger.release(
        owned.participant_id,
        owned.listing_key.instrument_id,
        owned.side,
        owned.limit_price,
        owned.remaining_quantity,
    )?;
    if let Some(record) = ownership.get_mut(&cancel.order_id) {
        record.state = OwnedOrderState::Canceled;
        record.remaining_quantity = QuantityLots::new(0);
    }
    payloads.push(EventPayload::OrderCanceled {
        order_id: owned.order_id,
        participant_id: owned.participant_id,
        instrument_id: owned.listing_key.instrument_id,
        remaining: owned.remaining_quantity,
        reason: CancelReason::Requested,
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
    use bunting_market_events::{NbcDone, SubmitOrder};
    use bunting_market_types::{CommandId, CorrelationId, LogicalTimeNs};

    fn participant(id: u128) -> ParticipantDefinition {
        ParticipantDefinition {
            participant_id: ParticipantId::new(id),
            enabled: true,
            limits: RiskLimits {
                max_order_quantity: QuantityLots::new(100),
                max_open_order_quantity: QuantityLots::new(1_000),
                max_absolute_position: QuantityLots::new(1_000),
            },
            initial_cash: MoneyMinor::new(100_000),
            initial_positions: BTreeMap::from([
                (InstrumentId::new(1), QuantityLots::new(100)),
                (InstrumentId::new(2), QuantityLots::new(100)),
            ]),
        }
    }

    fn run() -> RunState {
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [
                ListingDefinition {
                    key: ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                    symbol: "ONE".to_string(),
                    price_bounds: PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000))
                        .unwrap(),
                },
                ListingDefinition {
                    key: ListingKey::new(VenueId::new(1), InstrumentId::new(2)),
                    symbol: "TWO".to_string(),
                    price_bounds: PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000))
                        .unwrap(),
                },
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
        let one = ListingDefinition {
            key: ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
            symbol: "ONE".to_string(),
            price_bounds: PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
        };
        let two = ListingDefinition {
            key: ListingKey::new(VenueId::new(1), InstrumentId::new(2)),
            symbol: "TWO".to_string(),
            price_bounds: PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
        };
        let first = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [one.clone(), two.clone()],
            [participant(1), participant(2)],
        )
        .unwrap();
        let second = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
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
    fn legacy_one_listing_projection_migrates_deterministically() {
        let state = run();
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let snapshot = state.listing_snapshot(key).unwrap();
        let legacy_holdings: Vec<_> = state
            .holdings()
            .iter()
            .filter(|(_, instrument, _)| *instrument == InstrumentId::new(1))
            .copied()
            .collect();
        let legacy = serde_json::json!({
            "run_id": 1,
            "version": 0,
            "instrument_id": 1,
            "symbol": "ONE",
            "price_bounds": { "min": 1, "max": 1000 },
            "participants": [
                {
                    "participant_id": 1,
                    "enabled": true,
                    "limits": {
                        "max_order_quantity": 100,
                        "max_open_order_quantity": 1000,
                        "max_absolute_position": 1000
                    }
                },
                {
                    "participant_id": 2,
                    "enabled": true,
                    "limits": {
                        "max_order_quantity": 100,
                        "max_open_order_quantity": 1000,
                        "max_absolute_position": 1000
                    }
                }
            ],
            "accounts": state.accounts(),
            "holdings": legacy_holdings,
            "ownership": [],
            "snapshot": {
                "instrument_id": 1,
                "represented_sequence": 0,
                "checksum": snapshot.checksum,
                "package_json": snapshot.package_json
            }
        });
        let first = EngineSnapshotEnvelope::from_persisted_json(&legacy.to_string()).unwrap();
        let second = EngineSnapshotEnvelope::from_persisted_json(&legacy.to_string()).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.state.listings().len(), 1);
        assert_eq!(first.state.sequence(), EventSequence::new(0));
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
