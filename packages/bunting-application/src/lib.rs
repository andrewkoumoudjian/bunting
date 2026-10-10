#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Transport-neutral application service around the authoritative Bunting engine.

pub mod bnp;
pub mod competition;

use bunting_api_contract::{ActorIdentity, ActorRole};
use bunting_command_transaction::{CommandTransaction, ExecutedTransaction, TransactionError};
use bunting_engine::RunState;
use bunting_market_events::{
    Command, CommandPayload, EventEnvelope, EventPayload, Side, SimulationCommand,
    SimulationCommandRequest,
};
use bunting_market_types::{
    CorrelationId, EventSequence, InstrumentId, ListingKey, LogicalTimeNs, OrderId, ParticipantId,
    PriceTicks, QuantityLots, RunId, SessionId,
};
use bunting_origin_store::OriginStore;
use quarcc_bunting_adapter::{AdapterError, BuntingCommandContext, BuntingExecutionAdapter};
use quarcc_execution_engine::{
    ExecutionAction, ExecutionActionBuffer, ExecutionConfig, ExecutionEngine, ExecutionError,
    ExecutionIntent, ExecutionSnapshot, QuarccExecutionEngine,
    ids::{ClientOrderId, IntentId, LocalOrderId},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use simfix_mapping::{
    CompetitionRequest, InboundApplication, MappingContext, MappingError, map_inbound,
};
pub use simfix_mapping::{MarketDataEntryType, MarketDataRequestType};
use simfix_wire::FixMessage;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationError {
    Unauthenticated,
    Unauthorized,
    ActorMismatch,
    InvalidIdentity,
    UnknownInstrument,
    UnknownListing,
    Transaction(TransactionError),
    FixMapping(MappingError),
    Execution(ExecutionError),
    ExecutionAdapter(AdapterError),
    InvalidFixActionCount,
}

impl fmt::Display for ApplicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ApplicationError {}

impl From<TransactionError> for ApplicationError {
    fn from(value: TransactionError) -> Self {
        Self::Transaction(value)
    }
}

impl From<MappingError> for ApplicationError {
    fn from(value: MappingError) -> Self {
        Self::FixMapping(value)
    }
}

impl From<ExecutionError> for ApplicationError {
    fn from(value: ExecutionError) -> Self {
        Self::Execution(value)
    }
}

impl From<AdapterError> for ApplicationError {
    fn from(value: AdapterError) -> Self {
        Self::ExecutionAdapter(value)
    }
}

/// Verified application actor. Adapters construct this only from authenticated claims.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedActor {
    identity: ActorIdentity,
    participant_id: Option<ParticipantId>,
}

impl VerifiedActor {
    pub fn try_from_identity(identity: ActorIdentity) -> Result<Self, ApplicationError> {
        let participant_id = identity
            .participant_id
            .as_ref()
            .map(|value| ParticipantId::new(value.get()));
        match identity.role {
            ActorRole::Participant | ActorRole::BuiltInAgent if participant_id.is_none() => {
                Err(ApplicationError::InvalidIdentity)
            }
            ActorRole::Instructor | ActorRole::Administrator | ActorRole::Team
                if participant_id.is_some() =>
            {
                Err(ApplicationError::InvalidIdentity)
            }
            _ => Ok(Self {
                identity,
                participant_id,
            }),
        }
    }

    #[must_use]
    pub fn identity(&self) -> &ActorIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn participant_id(&self) -> Option<ParticipantId> {
        self.participant_id
    }
}

/// Enforces the product's participant command authority before engine recovery.
pub fn authorize_command(actor: &VerifiedActor, command: &Command) -> Result<(), ApplicationError> {
    let participant = actor.participant_id.ok_or(ApplicationError::Unauthorized)?;
    if !matches!(
        actor.identity.role,
        ActorRole::Participant | ActorRole::BuiltInAgent
    ) {
        return Err(ApplicationError::Unauthorized);
    }
    if command.actor != participant {
        return Err(ApplicationError::ActorMismatch);
    }
    let payload_participant = match &command.payload {
        CommandPayload::SubmitOrder(order) | CommandPayload::SubmitOrderAtListing { order, .. } => {
            Some(order.participant_id)
        }
        CommandPayload::CancelOrder(cancel) => Some(cancel.participant_id),
        CommandPayload::ActivateKillSwitch => None,
    };
    if payload_participant.is_some_and(|value| value != participant) {
        return Err(ApplicationError::ActorMismatch);
    }
    Ok(())
}

/// Enforces participant versus operator authority for simulation-domain commands.
pub fn authorize_simulation_command(
    actor: &VerifiedActor,
    request: &SimulationCommandRequest,
) -> Result<(), ApplicationError> {
    let participant_action = matches!(
        request.payload,
        SimulationCommand::DecideTender { .. }
            | SimulationCommand::CounterOtc { .. }
            | SimulationCommand::DecideOtc { .. }
    );
    if participant_action {
        let participant = actor.participant_id.ok_or(ApplicationError::Unauthorized)?;
        if request.actor != participant
            || !matches!(
                actor.identity.role,
                ActorRole::Participant | ActorRole::BuiltInAgent
            )
        {
            return Err(ApplicationError::ActorMismatch);
        }
    } else if !matches!(
        actor.identity.role,
        ActorRole::Instructor | ActorRole::Administrator
    ) {
        return Err(ApplicationError::Unauthorized);
    }
    Ok(())
}

#[derive(Debug)]
pub struct ApplicationService<'a, O> {
    origin: &'a O,
}

impl<'a, O> ApplicationService<'a, O>
where
    O: OriginStore,
{
    #[must_use]
    pub const fn new(origin: &'a O) -> Self {
        Self { origin }
    }

    /// Executes one authenticated command and returns only origin-committed facts.
    pub fn execute(
        &self,
        actor: &VerifiedActor,
        command: &Command,
    ) -> Result<ExecutedTransaction, ApplicationError> {
        authorize_command(actor, command)?;
        CommandTransaction::new(self.origin)
            .execute_detailed(command)
            .map_err(ApplicationError::from)
    }

    /// [`Self::execute`] for a command ordered by the admission sequencer.
    pub fn execute_admitted(
        &self,
        actor: &VerifiedActor,
        command: &Command,
        admission: &bunting_origin_store::AdmissionRecord,
    ) -> Result<ExecutedTransaction, ApplicationError> {
        authorize_command(actor, command)?;
        CommandTransaction::new(self.origin)
            .execute_admitted(command, admission)
            .map_err(ApplicationError::from)
    }

    /// Executes one authenticated simulation-domain command and returns committed facts.
    pub fn execute_simulation(
        &self,
        actor: &VerifiedActor,
        request: &SimulationCommandRequest,
    ) -> Result<ExecutedTransaction, ApplicationError> {
        authorize_simulation_command(actor, request)?;
        CommandTransaction::new(self.origin)
            .execute_simulation_detailed(request)
            .map_err(ApplicationError::from)
    }

    /// Reads the committed live run without copying it. `read` must not
    /// execute commands through this service.
    pub fn read<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, ApplicationError> {
        self.origin
            .read_run(run_id, read)
            .map_err(TransactionError::from)
            .map_err(ApplicationError::from)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketProjection {
    pub run_id: RunId,
    pub listing_key: ListingKey,
    pub instrument_id: InstrumentId,
    pub sequence: EventSequence,
    pub bids: Vec<(i64, i64)>,
    pub asks: Vec<(i64, i64)>,
}

/// A distinct instrument-level view of venue quotes. No order books are merged,
/// and equal-price ties select the lowest canonical `ListingKey`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsolidatedBbo {
    pub run_id: RunId,
    pub instrument_id: InstrumentId,
    pub sequence: EventSequence,
    pub bid: Option<(ListingKey, PriceTicks, QuantityLots)>,
    pub ask: Option<(ListingKey, PriceTicks, QuantityLots)>,
}

/// Allowlisted public fact. Canonical envelopes and participant/order identity
/// deliberately cannot cross this transport-neutral projection boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PublicTrade {
    pub sequence: EventSequence,
    pub logical_time: LogicalTimeNs,
    pub instrument_id: InstrumentId,
    /// Explicit venue identity; no instrument-only fallback is published.
    pub listing_key: ListingKey,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
    /// Public reference of the resting order this trade executed against,
    /// on order-by-order feeds only (see [`DisplayedOrder`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maker_reference: Option<u64>,
    /// Broker identifiers of the buyer and seller, on venues that publish
    /// them, unless that side's order was anonymous (see [`trade_brokers`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buyer_broker: Option<ParticipantId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seller_broker: Option<ParticipantId>,
}

/// Projects only facts that are safe to publish without participant, order,
/// command, correlation, position, or reserve-quantity metadata.
#[must_use]
pub fn project_public_event(event: &EventEnvelope, listing_key: ListingKey) -> Option<PublicTrade> {
    match event.payload {
        EventPayload::TradeExecuted {
            instrument_id,
            listing_key: Some(executed_at),
            price,
            quantity,
            ..
        } if executed_at == listing_key && instrument_id == listing_key.instrument_id => {
            Some(PublicTrade {
                sequence: event.sequence,
                logical_time: event.logical_time,
                instrument_id,
                listing_key,
                price,
                quantity,
                maker_reference: None,
                buyer_broker: None,
                seller_broker: None,
            })
        }
        _ => None,
    }
}

/// One price level's visible quantity changing in a public feed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct LevelChange {
    pub side: Side,
    pub price: PriceTicks,
    /// Visible quantity before the change; zero for a new level.
    pub previous: QuantityLots,
    /// Visible quantity after the change; zero when the level was removed.
    pub quantity: QuantityLots,
}

/// The public, anonymous changes one commit made to one listing: its
/// trades in execution order, then its visible depth changes by level.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PublicListingUpdate {
    pub listing_key: ListingKey,
    pub trades: Vec<PublicTrade>,
    pub levels: Vec<LevelChange>,
    /// Best visible bid and offer after the commit.
    pub best_bid: Option<(PriceTicks, QuantityLots)>,
    pub best_ask: Option<(PriceTicks, QuantityLots)>,
    /// Order-by-order changes, when order-by-order feeds are enabled.
    pub orders: Vec<OrderChange>,
}

/// One displayed order of a listing, for order-by-order (L3) feeds.
///
/// `reference` is the order's time priority in its book: anonymous,
/// unrelated to any participant, client or venue order ID, and new
/// whenever the order loses priority (an iceberg refresh shows as the old
/// reference deleted and a new one added, as on real order-by-order
/// feeds). Only the displayed quantity is shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayedOrder {
    /// Internal key for diffing; never published.
    pub order_id: OrderId,
    pub reference: u64,
    pub side: Side,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
    /// The owner's broker identifier, on venues that publish them, unless
    /// the order is anonymous.
    pub broker: Option<ParticipantId>,
}

/// The broker identifier a venue publishes for one order: its owner, if
/// the listing's venue publishes broker identifiers and the order is not
/// anonymous.
#[must_use]
pub fn order_broker(
    state: &RunState,
    listing_key: ListingKey,
    order_id: OrderId,
) -> Option<ParticipantId> {
    let publishes = state
        .listings()
        .get(&listing_key)
        .is_some_and(|listing| listing.definition().broker_ids());
    if !publishes {
        return None;
    }
    state
        .ownership()
        .get(&order_id)
        .filter(|owned| !owned.anonymous)
        .map(|owned| owned.participant_id)
}

/// Buyer and seller broker identifiers of one committed trade, read right
/// after the commit (terminal orders stay in the retained ownership
/// window).
#[must_use]
pub fn trade_brokers(
    state: &RunState,
    event: &EventEnvelope,
) -> (Option<ParticipantId>, Option<ParticipantId>) {
    let EventPayload::TradeExecuted {
        listing_key: Some(listing_key),
        maker_order_id,
        taker_order_id,
        ..
    } = &event.payload
    else {
        return (None, None);
    };
    let (mut buyer, mut seller) = (None, None);
    for order_id in [maker_order_id, taker_order_id] {
        let broker = order_broker(state, *listing_key, *order_id);
        match state.ownership().get(order_id).map(|owned| owned.side) {
            Some(Side::Buy) => buyer = broker,
            Some(Side::Sell) => seller = broker,
            None => {}
        }
    }
    (buyer, seller)
}

/// Every displayed order of one listing: bids then asks, each in matching
/// priority (best price first, then time).
///
/// # Errors
/// Returns an error for an unknown listing.
pub fn displayed_orders(
    state: &RunState,
    listing_key: ListingKey,
) -> Result<Vec<DisplayedOrder>, bunting_engine::EngineError> {
    let listing = state
        .listings()
        .get(&listing_key)
        .ok_or(bunting_engine::EngineError::UnknownListing)?;
    Ok([Side::Buy, Side::Sell]
        .into_iter()
        .flat_map(|side| listing.resting(side))
        .filter(|order| order.visible.get() > 0)
        .map(|order| DisplayedOrder {
            order_id: order.order_id,
            reference: order.priority,
            side: order.side,
            price: order.price,
            quantity: order.visible,
            broker: order_broker(state, listing_key, order.order_id),
        })
        .collect())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum OrderAction {
    Added,
    /// The displayed quantity changed (a partial execution or reduction);
    /// the order keeps its reference and priority.
    Changed,
    Deleted,
}

/// One order-by-order change. `quantity` is the displayed quantity after
/// the change (zero when deleted).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct OrderChange {
    pub action: OrderAction,
    pub side: Side,
    pub reference: u64,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
    pub broker: Option<ParticipantId>,
}

/// Order-by-order difference between two committed views of one listing:
/// deletions first, then changes, then additions in priority order, so a
/// client applying them in order never holds two orders with one
/// reference.
#[must_use]
pub fn diff_orders(before: &[DisplayedOrder], after: &[DisplayedOrder]) -> Vec<OrderChange> {
    let old: BTreeMap<u64, &DisplayedOrder> = before
        .iter()
        .map(|order| (order.reference, order))
        .collect();
    let new: BTreeMap<u64, &DisplayedOrder> =
        after.iter().map(|order| (order.reference, order)).collect();
    let change = |action, order: &DisplayedOrder, quantity| OrderChange {
        action,
        side: order.side,
        reference: order.reference,
        price: order.price,
        quantity,
        broker: order.broker,
    };
    let mut changes: Vec<_> = before
        .iter()
        .filter(|order| !new.contains_key(&order.reference))
        .map(|order| change(OrderAction::Deleted, order, QuantityLots::new(0)))
        .collect();
    for order in after {
        if let Some(previous) = old.get(&order.reference)
            && previous.quantity != order.quantity
        {
            changes.push(change(OrderAction::Changed, order, order.quantity));
        }
    }
    changes.extend(
        after
            .iter()
            .filter(|order| !old.contains_key(&order.reference))
            .map(|order| change(OrderAction::Added, order, order.quantity)),
    );
    changes
}

/// Visible depth of one listing: bids best first, asks best first.
pub type PublicDepth = (
    Vec<(PriceTicks, QuantityLots)>,
    Vec<(PriceTicks, QuantityLots)>,
);

/// Level-by-level difference between two committed views of one book:
/// every price whose visible quantity changed, with its new quantity (zero
/// for a removed level). Bids are listed before asks, each in price order.
#[must_use]
pub fn diff_levels(before: &PublicDepth, after: &PublicDepth) -> Vec<LevelChange> {
    let mut changes = Vec::new();
    for (side, old, new) in [
        (Side::Buy, &before.0, &after.0),
        (Side::Sell, &before.1, &after.1),
    ] {
        let old: BTreeMap<PriceTicks, QuantityLots> = old.iter().copied().collect();
        let new: BTreeMap<PriceTicks, QuantityLots> = new.iter().copied().collect();
        let prices: std::collections::BTreeSet<PriceTicks> =
            old.keys().chain(new.keys()).copied().collect();
        for price in prices {
            let previous = old.get(&price).copied().unwrap_or(QuantityLots::new(0));
            let current = new.get(&price).copied().unwrap_or(QuantityLots::new(0));
            if previous != current {
                changes.push(LevelChange {
                    side,
                    price,
                    previous,
                    quantity: current,
                });
            }
        }
    }
    changes
}

/// Derives an opaque stable session identifier without persisting bearer or
/// transport credentials.
#[must_use]
pub fn derive_session_id(authenticated_session: &[u8]) -> SessionId {
    let digest = Sha256::digest(authenticated_session);
    SessionId::new(nonzero_u128(&digest))
}

/// Maps one transport-local command ID into the authoritative `u128` domain.
#[must_use]
pub fn namespace_command_id(
    run_id: RunId,
    actor: ParticipantId,
    session_id: SessionId,
    local_id: u128,
) -> bunting_market_types::CommandId {
    bunting_market_types::CommandId::new(namespace_id(
        b"command", run_id, actor, session_id, local_id,
    ))
}

/// Maps one transport-local order ID into the authoritative `u128` domain.
#[must_use]
pub fn namespace_order_id(
    run_id: RunId,
    actor: ParticipantId,
    session_id: SessionId,
    local_id: u128,
) -> bunting_market_types::OrderId {
    bunting_market_types::OrderId::new(namespace_id(b"order", run_id, actor, session_id, local_id))
}

fn namespace_id(
    domain: &[u8],
    run_id: RunId,
    actor: ParticipantId,
    session_id: SessionId,
    local_id: u128,
) -> u128 {
    let mut hasher = Sha256::new();
    hasher.update(b"bunting.client-identity.v1");
    hasher.update(domain);
    hasher.update(run_id.get().to_be_bytes());
    hasher.update(actor.get().to_be_bytes());
    hasher.update(session_id.get().to_be_bytes());
    hasher.update(local_id.to_be_bytes());
    nonzero_u128(&hasher.finalize())
}

fn nonzero_u128(bytes: &[u8]) -> u128 {
    let mut value = [0_u8; 16];
    value.copy_from_slice(&bytes[..16]);
    u128::from_be_bytes(value).max(1)
}

/// Reads committed depth for exactly one exchange listing. Missing or corrupt
/// listing state never falls back to another venue.
pub fn project_market(
    state: &RunState,
    listing_key: ListingKey,
) -> Result<MarketProjection, ApplicationError> {
    let (bids, asks) = state
        .visible_levels(listing_key)
        .map_err(|_| ApplicationError::UnknownListing)?;
    let convert =
        |levels: bunting_engine::VisibleLevels| -> Result<Vec<(i64, i64)>, ApplicationError> {
            Ok(levels
                .into_iter()
                .map(|(price, quantity)| (price.get(), quantity.get()))
                .collect())
        };
    Ok(MarketProjection {
        run_id: state.run_id(),
        instrument_id: listing_key.instrument_id,
        listing_key,
        sequence: state.sequence(),
        bids: convert(bids)?,
        asks: convert(asks)?,
    })
}

/// Deterministic cross-venue best bid/offer, with venue identity on each side.
/// Prices on different listings are never combined into one fictitious queue.
pub fn project_consolidated_bbo(
    state: &RunState,
    instrument_id: InstrumentId,
) -> Result<ConsolidatedBbo, ApplicationError> {
    let mut found = false;
    let mut best_bid: Option<(ListingKey, PriceTicks, QuantityLots)> = None;
    let mut best_ask: Option<(ListingKey, PriceTicks, QuantityLots)> = None;
    for &key in state.listings().keys() {
        if key.instrument_id != instrument_id {
            continue;
        }
        found = true;
        let (bids, asks) = state
            .visible_levels(key)
            .map_err(|_| ApplicationError::UnknownListing)?;
        if let Some(&(price, quantity)) = bids.first() {
            if best_bid
                .is_none_or(|(winner, value, _)| price > value || (price == value && key < winner))
            {
                best_bid = Some((key, price, quantity));
            }
        }
        if let Some(&(price, quantity)) = asks.first() {
            if best_ask
                .is_none_or(|(winner, value, _)| price < value || (price == value && key < winner))
            {
                best_ask = Some((key, price, quantity));
            }
        }
    }
    if !found {
        return Err(ApplicationError::UnknownInstrument);
    }
    Ok(ConsolidatedBbo {
        run_id: state.run_id(),
        instrument_id,
        sequence: state.sequence(),
        bid: best_bid,
        ask: best_ask,
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixApplicationSnapshot {
    pub version: u16,
    pub next_intent_id: u128,
    pub execution: ExecutionSnapshot,
    pub adapter: BuntingExecutionAdapter,
    #[serde(default)]
    pub client_order_ids: BTreeMap<LocalOrderId, ClientOrderId>,
    /// Host-assigned identity epoch; see [`FixApplicationState::with_identity_epoch`].
    #[serde(default)]
    pub identity_epoch: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixCommandContext {
    pub actor: ParticipantId,
    pub run_id: RunId,
    pub expected_sequence: EventSequence,
    pub logical_time: LogicalTimeNs,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FixApplicationRequest {
    Command(Command),
    MarketData {
        request_id: String,
        listing_key: ListingKey,
        request_type: MarketDataRequestType,
        market_depth: usize,
        entry_types: Vec<MarketDataEntryType>,
        /// `false` for an order-by-order (L3) request (FIX 266=N).
        aggregated: bool,
    },
    Competition(CompetitionRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixApplicationState {
    execution: QuarccExecutionEngine,
    adapter: BuntingExecutionAdapter,
    next_intent_id: u128,
    client_order_ids: BTreeMap<LocalOrderId, ClientOrderId>,
    identity_epoch: u128,
}

impl FixApplicationState {
    #[must_use]
    pub fn new(config: ExecutionConfig) -> Self {
        Self {
            execution: QuarccExecutionEngine::new(config),
            adapter: BuntingExecutionAdapter::default(),
            next_intent_id: 1,
            client_order_ids: BTreeMap::new(),
            identity_epoch: 0,
        }
    }

    /// Sets the identity epoch that namespaces this session's canonical
    /// command and order IDs. The host assigns a value unique to each newly
    /// created application state; a restored state keeps its persisted epoch,
    /// so orders from earlier connections still map back to this session.
    #[must_use]
    pub const fn with_identity_epoch(mut self, identity_epoch: u128) -> Self {
        self.identity_epoch = identity_epoch;
        self
    }

    pub fn restore(snapshot: FixApplicationSnapshot) -> Result<Self, ApplicationError> {
        if snapshot.version != 1 || snapshot.next_intent_id == 0 {
            return Err(ApplicationError::InvalidIdentity);
        }
        Ok(Self {
            execution: QuarccExecutionEngine::restore(snapshot.execution)?,
            adapter: snapshot.adapter,
            next_intent_id: snapshot.next_intent_id,
            client_order_ids: snapshot.client_order_ids,
            identity_epoch: snapshot.identity_epoch,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> FixApplicationSnapshot {
        FixApplicationSnapshot {
            version: 1,
            next_intent_id: self.next_intent_id,
            execution: self.execution.snapshot(),
            adapter: self.adapter.clone(),
            client_order_ids: self.client_order_ids.clone(),
            identity_epoch: self.identity_epoch,
        }
    }

    pub fn map_message(
        &mut self,
        message: &FixMessage,
        context: &FixCommandContext,
    ) -> Result<FixApplicationRequest, ApplicationError> {
        let mapped = map_inbound(
            message,
            MappingContext {
                participant_id: context.actor,
                next_intent_id: IntentId::new(self.next_intent_id),
            },
        )?;
        self.next_intent_id = self
            .next_intent_id
            .checked_add(1)
            .ok_or(ApplicationError::InvalidIdentity)?;
        match mapped {
            InboundApplication::Competition(request) => {
                Ok(FixApplicationRequest::Competition(request))
            }
            InboundApplication::MarketDataRequest {
                request_id,
                listing_key,
                request_type,
                market_depth,
                entry_types,
                aggregated,
            } => Ok(FixApplicationRequest::MarketData {
                request_id,
                listing_key,
                request_type,
                market_depth,
                entry_types,
                aggregated,
            }),
            InboundApplication::Intent(intent) => {
                let client_order_id = match &intent {
                    ExecutionIntent::Submit { order, .. } => Some(order.client_order_id),
                    ExecutionIntent::Cancel { .. }
                    | ExecutionIntent::Replace { .. }
                    | ExecutionIntent::Query { .. }
                    | ExecutionIntent::ActivateKillSwitch { .. } => None,
                };
                let mut actions = ExecutionActionBuffer::with_limit(2);
                self.execution.submit_intent(intent, &mut actions)?;
                let [action] = actions.as_slice() else {
                    return Err(ApplicationError::InvalidFixActionCount);
                };
                if let (Some(client_order_id), ExecutionAction::Submit { local_order_id, .. }) =
                    (client_order_id, action)
                {
                    self.client_order_ids
                        .insert(*local_order_id, client_order_id);
                }
                let mut command = self.adapter.command_for_action(
                    action,
                    &BuntingCommandContext {
                        run_id: context.run_id,
                        actor: context.actor,
                        expected_sequence: context.expected_sequence,
                        logical_time: context.logical_time,
                        correlation_id: context.correlation_id,
                    },
                )?;
                let namespace =
                    session_namespace(context.run_id, context.actor, self.identity_epoch);
                command.command_id = bunting_market_types::CommandId::new(canonical_id(
                    namespace,
                    command.command_id.get(),
                )?);
                match &mut command.payload {
                    CommandPayload::SubmitOrder(order)
                    | CommandPayload::SubmitOrderAtListing { order, .. } => {
                        order.order_id =
                            OrderId::new(canonical_id(namespace, order.order_id.get())?);
                    }
                    CommandPayload::CancelOrder(cancel) => {
                        cancel.order_id =
                            OrderId::new(canonical_id(namespace, cancel.order_id.get())?);
                    }
                    CommandPayload::ActivateKillSwitch => {}
                }
                if let CommandPayload::SubmitOrder(order) = &command.payload {
                    let listing_key = simfix_mapping::fix_order_listing(message)?
                        .ok_or(ApplicationError::UnknownListing)?;
                    if listing_key.instrument_id != order.instrument_id {
                        return Err(ApplicationError::UnknownListing);
                    }
                    command.payload = CommandPayload::SubmitOrderAtListing {
                        listing_key,
                        order: order.clone(),
                    };
                }
                Ok(FixApplicationRequest::Command(command))
            }
        }
    }

    /// Converts committed private facts to FIX and advances participant execution state.
    pub fn committed_messages(
        &mut self,
        actor: ParticipantId,
        events: &[bunting_market_events::EventEnvelope],
    ) -> Result<Vec<FixMessage>, ApplicationError> {
        let Some(first) = events.first() else {
            return Ok(Vec::new());
        };
        let namespace = session_namespace(first.run_id, actor, self.identity_epoch);
        let localized: Vec<_> = events
            .iter()
            .map(|event| localize_event(namespace, event))
            .collect();
        let mut reports = self.adapter.normalize_committed_events(actor, &localized)?;
        let mut messages = Vec::with_capacity(reports.len());
        for report in &mut reports {
            if report.client_order_id.is_none() {
                report.client_order_id = report
                    .local_order_id
                    .and_then(|local| self.client_order_ids.get(&local).copied());
            }
            let mut actions = ExecutionActionBuffer::with_limit(2);
            self.execution.apply_venue_report(report, &mut actions)?;
            messages.push(simfix_mapping::map_execution_report(report)?);
        }
        Ok(messages)
    }
}

/// 64-bit namespace for one participant session's canonical IDs.
fn session_namespace(run_id: RunId, actor: ParticipantId, identity_epoch: u128) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(b"bunting.fix-session-namespace.v1");
    hasher.update(run_id.get().to_be_bytes());
    hasher.update(actor.get().to_be_bytes());
    hasher.update(identity_epoch.to_be_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes).max(1)
}

/// Places a session-local ID in its session's namespace. Local IDs are
/// bounded counters; anything wider is rejected rather than truncated.
fn canonical_id(namespace: u64, local: u128) -> Result<u128, ApplicationError> {
    if local == 0 || local > u128::from(u64::MAX) {
        return Err(ApplicationError::InvalidIdentity);
    }
    Ok((u128::from(namespace) << 64) | local)
}

/// Maps a canonical order ID back to this session's local ID. IDs from other
/// namespaces stay at or above 2^64, so they can never equal a local ID.
const fn local_id(namespace: u64, canonical: u128) -> u128 {
    let high = canonical >> 64;
    if high == namespace as u128 {
        canonical & (u64::MAX as u128)
    } else if high == 0 {
        canonical | (1 << 127)
    } else {
        canonical
    }
}

fn localize_event(
    namespace: u64,
    event: &bunting_market_events::EventEnvelope,
) -> bunting_market_events::EventEnvelope {
    let local = |id: OrderId| OrderId::new(local_id(namespace, id.get()));
    let mut event = event.clone();
    match &mut event.payload {
        EventPayload::OrderReceived { order, .. } => order.order_id = local(order.order_id),
        EventPayload::OrderAccepted { order_id }
        | EventPayload::OrderRested { order_id, .. }
        | EventPayload::OrderReduced { order_id, .. }
        | EventPayload::OrderCompleted { order_id }
        | EventPayload::OrderCanceled { order_id, .. } => *order_id = local(*order_id),
        EventPayload::OrderRejected { order_id, .. } => *order_id = order_id.map(local),
        EventPayload::TradeExecuted {
            maker_order_id,
            taker_order_id,
            ..
        } => {
            *maker_order_id = local(*maker_order_id);
            *taker_order_id = local(*taker_order_id);
        }
        _ => {}
    }
    event
}

#[must_use]
pub fn listing_for_command(state: &RunState, command: &Command) -> Option<ListingKey> {
    match &command.payload {
        CommandPayload::SubmitOrder(order) => {
            state.listing_key_for_instrument(order.instrument_id).ok()
        }
        CommandPayload::SubmitOrderAtListing { listing_key, .. } => Some(*listing_key),
        CommandPayload::CancelOrder(cancel) => state
            .ownership()
            .get(&cancel.order_id)
            .map(|owned| owned.listing_key),
        CommandPayload::ActivateKillSwitch => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn level_diffs_name_every_changed_price_with_before_and_after() {
        let level = |price, quantity| (PriceTicks::new(price), QuantityLots::new(quantity));
        let before = (vec![level(99, 5), level(98, 2)], vec![level(101, 3)]);
        let after = (vec![level(99, 4), level(98, 2)], vec![level(102, 1)]);
        let change = |side, price, previous, quantity| LevelChange {
            side,
            price: PriceTicks::new(price),
            previous: QuantityLots::new(previous),
            quantity: QuantityLots::new(quantity),
        };
        assert_eq!(
            diff_levels(&before, &after),
            vec![
                change(Side::Buy, 99, 5, 4),
                change(Side::Sell, 101, 3, 0),
                change(Side::Sell, 102, 0, 1),
            ]
        );
        assert!(diff_levels(&after, &after).is_empty());
    }

    #[test]
    fn order_diffs_delete_change_then_add_by_public_reference() {
        let order = |id, reference, side, price, quantity| DisplayedOrder {
            order_id: OrderId::new(id),
            reference,
            side,
            price: PriceTicks::new(price),
            quantity: QuantityLots::new(quantity),
            broker: None,
        };
        let before = vec![
            order(1, 1, Side::Buy, 99, 5),
            order(2, 2, Side::Sell, 101, 3),
            order(3, 3, Side::Sell, 101, 2),
        ];
        // Order 1 partly filled, order 2 gone, order 3's iceberg slice
        // refreshed behind a new order 4.
        let after = vec![
            order(1, 1, Side::Buy, 99, 4),
            order(4, 4, Side::Sell, 101, 1),
            order(3, 5, Side::Sell, 101, 2),
        ];
        let change = |action, side, reference, price, quantity| OrderChange {
            action,
            side,
            reference,
            price: PriceTicks::new(price),
            quantity: QuantityLots::new(quantity),
            broker: None,
        };
        assert_eq!(
            diff_orders(&before, &after),
            vec![
                change(OrderAction::Deleted, Side::Sell, 2, 101, 0),
                change(OrderAction::Deleted, Side::Sell, 3, 101, 0),
                change(OrderAction::Changed, Side::Buy, 1, 99, 4),
                change(OrderAction::Added, Side::Sell, 4, 101, 1),
                change(OrderAction::Added, Side::Sell, 5, 101, 2),
            ]
        );
        assert!(diff_orders(&after, &after).is_empty());
    }
    use bunting_api_contract::UnsignedDecimalString;
    use bunting_engine::{ListingDefinition, ParticipantDefinition, ScenarioDefinition};
    use bunting_market_events::{
        NewsAudience, OrderKind, Side, SimulationCommand, SimulationCommandRequest, SubmitOrder,
    };
    use bunting_market_types::CurrencyId;
    use bunting_market_types::{
        CommandId, InstrumentId, IterationId, MoneyMinor, NewsId, OrderId, PriceBounds, PriceTicks,
        QuantityLots, ScenarioId, ScenarioVersion, VenueId,
    };
    use bunting_origin_store::InMemoryOrigin;
    use bunting_risk_engine::RiskLimits;
    use std::collections::BTreeMap;

    fn actor(id: u128) -> VerifiedActor {
        VerifiedActor::try_from_identity(ActorIdentity {
            actor_id: UnsignedDecimalString::new(id),
            role: ActorRole::Participant,
            participant_id: Some(UnsignedDecimalString::new(id)),
            team_id: None,
        })
        .unwrap()
    }

    fn run() -> RunState {
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [bunting_engine::InstrumentDefinition::new(
                InstrumentId::new(1),
                "BNT",
                CurrencyId::new(1),
                bunting_engine::InstrumentKind::Equity,
            )
            .with_opening_mark(PriceTicks::new(100))],
            [ListingDefinition::new(
                ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                "ONE".to_owned(),
                PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
            )
            .unwrap()],
            [ParticipantDefinition::new(
                ParticipantId::new(7),
                true,
                RiskLimits::new(
                    QuantityLots::new(100),
                    QuantityLots::new(1_000),
                    QuantityLots::new(1_000),
                ),
                BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(100_000))]),
                BTreeMap::new(),
            )],
        )
        .unwrap();
        RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap()
    }

    fn command() -> Command {
        Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(1),
            correlation_id: CorrelationId::new(1),
            logical_time: LogicalTimeNs::new(1),
            expected_sequence: EventSequence::new(0),
            actor: ParticipantId::new(7),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(1),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(7),
                side: Side::Buy,
                quantity: QuantityLots::new(1),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(10),
                },
                anonymous: false,
            }),
        }
    }

    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one cross-venue scenario exercises venue depth, NBBO ties and public feed isolation"
    )]
    fn listing_depth_nbbo_and_public_trades_are_unambiguous() {
        let instrument = InstrumentId::new(1);
        let primary = ListingKey::new(VenueId::new(1), instrument);
        let alternate = ListingKey::new(VenueId::new(2), instrument);
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap();
        let limits = RiskLimits::new(
            QuantityLots::new(100),
            QuantityLots::new(100),
            QuantityLots::new(100),
        );
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(3),
            ScenarioVersion::new(1),
            [bunting_engine::InstrumentDefinition::new(
                InstrumentId::new(1),
                "BNT",
                CurrencyId::new(1),
                bunting_engine::InstrumentKind::Equity,
            )
            .with_opening_mark(PriceTicks::new(100))],
            [
                ListingDefinition::new(primary, "PRIMARY".into(), bounds).unwrap(),
                ListingDefinition::new(alternate, "ALTERNATE".into(), bounds).unwrap(),
            ],
            [
                ParticipantDefinition::new(
                    ParticipantId::new(7),
                    true,
                    limits,
                    BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(100_000))]),
                    BTreeMap::from([(instrument, QuantityLots::new(10))]),
                ),
                ParticipantDefinition::new(
                    ParticipantId::new(8),
                    true,
                    limits,
                    BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(100_000))]),
                    BTreeMap::new(),
                ),
            ],
        )
        .unwrap();
        let mut state =
            RunState::from_scenario(RunId::new(3), IterationId::new(1), &scenario).unwrap();
        let command = |state: &RunState,
                       id: u128,
                       actor: u128,
                       listing_key: ListingKey,
                       side: Side,
                       price: i64| Command {
            run_id: state.run_id(),
            command_id: CommandId::new(id),
            correlation_id: CorrelationId::new(id),
            logical_time: LogicalTimeNs::new(u64::try_from(id).unwrap()),
            expected_sequence: state.sequence(),
            actor: ParticipantId::new(actor),
            payload: CommandPayload::SubmitOrderAtListing {
                listing_key,
                order: SubmitOrder {
                    order_id: OrderId::new(id),
                    instrument_id: instrument,
                    participant_id: ParticipantId::new(actor),
                    side,
                    quantity: QuantityLots::new(1),
                    kind: OrderKind::Limit {
                        price: PriceTicks::new(price),
                    },
                    anonymous: false,
                },
            },
        };
        for (id, actor, venue, side, price) in [
            (1, 8, primary, Side::Buy, 99),
            (2, 8, alternate, Side::Buy, 99),
            (3, 7, primary, Side::Sell, 105),
            (4, 7, alternate, Side::Sell, 104),
        ] {
            let result = state
                .transition(&command(&state, id, actor, venue, side, price))
                .unwrap();
            assert!(result.accepted);
            state = result.candidate;
        }
        let a = project_market(&state, primary).unwrap();
        let b = project_market(&state, alternate).unwrap();
        assert_eq!(a.listing_key, primary);
        assert_eq!(a.asks, vec![(105, 1)]);
        assert_eq!(b.asks, vec![(104, 1)]);
        assert_eq!(a.bids, vec![(99, 1)]);
        assert_eq!(b.bids, vec![(99, 1)]);
        let nbbo = project_consolidated_bbo(&state, instrument).unwrap();
        assert_eq!(
            nbbo.bid,
            Some((primary, PriceTicks::new(99), QuantityLots::new(1)))
        );
        assert_eq!(
            nbbo.ask,
            Some((alternate, PriceTicks::new(104), QuantityLots::new(1)))
        );
        assert_eq!(
            project_market(&state, ListingKey::new(VenueId::new(3), instrument)),
            Err(ApplicationError::UnknownListing)
        );
        let executed = state
            .transition(&command(&state, 5, 8, primary, Side::Buy, 105))
            .unwrap();
        assert!(executed.accepted);
        let trade = executed
            .events
            .iter()
            .find_map(|event| project_public_event(event, primary))
            .unwrap();
        assert_eq!(trade.listing_key, primary);
        assert_eq!(trade.price, PriceTicks::new(105));
        assert!(
            executed
                .events
                .iter()
                .all(|event| project_public_event(event, alternate).is_none())
        );
        let json = serde_json::to_string(&trade).unwrap();
        for private in [
            "buyer_id",
            "seller_id",
            "maker_order_id",
            "taker_order_id",
            "actor",
        ] {
            assert!(!json.contains(private));
        }
        assert_eq!(
            project_market(&executed.candidate, alternate).unwrap().asks,
            b.asks
        );
    }

    #[test]
    fn commits_before_returning_and_recovers_same_projection() {
        let origin = InMemoryOrigin::new();
        origin.insert_run(run()).unwrap();
        let service = ApplicationService::new(&origin);
        let executed = service.execute(&actor(7), &command()).unwrap();
        assert!(!executed.duplicate);
        assert_eq!(executed.result.committed_sequence, EventSequence::new(1));
        let mut expected = run();
        expected.apply(&command()).unwrap();
        assert_eq!(
            service.read(RunId::new(1), RunState::clone).unwrap(),
            expected
        );
    }

    #[test]
    fn identity_cannot_override_command_participant() {
        assert_eq!(
            authorize_command(&actor(8), &command()),
            Err(ApplicationError::ActorMismatch)
        );
    }

    #[test]
    fn competition_projections_do_not_leak_another_participants_news() {
        let initial = run();
        let publish = |sequence, command_id, audience| SimulationCommandRequest {
            run_id: RunId::new(1),
            command_id: CommandId::new(command_id),
            correlation_id: CorrelationId::new(command_id),
            logical_time: LogicalTimeNs::new(u64::try_from(command_id).unwrap()),
            expected_sequence: EventSequence::new(sequence),
            actor: ParticipantId::new(99),
            payload: SimulationCommand::PublishNews {
                news_id: NewsId::new(command_id),
                audience,
                headline: format!("news-{command_id}"),
                body: "bounded".to_owned(),
            },
        };
        let public = initial
            .transition_simulation(&publish(0, 1, NewsAudience::Public))
            .unwrap()
            .candidate;
        let private = public
            .transition_simulation(&publish(
                1,
                2,
                NewsAudience::Participant(ParticipantId::new(8)),
            ))
            .unwrap()
            .candidate;
        let view = competition::news_tenders(&private, &actor(7)).unwrap();
        assert_eq!(view.news.len(), 1);
        assert_eq!(view.news[0].news_id, NewsId::new(1));
        let account = competition::account(&private, &actor(7)).unwrap();
        assert_eq!(account.participant_id, ParticipantId::new(7));
        assert_eq!(
            account.policies.score,
            "bunting.score.nlv-last-trade-rank.v2"
        );
    }

    #[test]
    fn client_ids_are_stable_within_a_session_and_distinct_across_sessions() {
        let run = RunId::new(1);
        let actor = ParticipantId::new(7);
        let first = derive_session_id(b"authenticated-session-a");
        let second = derive_session_id(b"authenticated-session-b");

        assert_eq!(
            namespace_command_id(run, actor, first, 1),
            namespace_command_id(run, actor, first, 1)
        );
        assert_ne!(
            namespace_command_id(run, actor, first, 1),
            namespace_command_id(run, actor, second, 1)
        );
        assert_ne!(
            namespace_order_id(run, actor, first, 1),
            namespace_order_id(run, actor, second, 1)
        );
        assert_ne!(
            namespace_command_id(run, actor, first, 1).get(),
            namespace_order_id(run, actor, first, 1).get()
        );
    }

    #[test]
    fn fix_session_ids_are_disjoint_and_map_back_only_to_their_session() {
        let run = RunId::new(1);
        let (one, two) = (ParticipantId::new(1), ParticipantId::new(2));
        let first = session_namespace(run, one, 7);
        let other_participant = session_namespace(run, two, 7);
        let new_epoch = session_namespace(run, one, 8);
        assert_ne!(first, other_participant);
        assert_ne!(first, new_epoch);
        assert_eq!(first, session_namespace(run, one, 7));

        let mine = canonical_id(first, 1).unwrap();
        let theirs = canonical_id(other_participant, 1).unwrap();
        assert_ne!(mine, theirs);
        assert!(mine > u128::from(u64::MAX));
        assert_eq!(local_id(first, mine), 1);
        // Foreign IDs, namespaced or not, never equal a local ID.
        assert!(local_id(first, theirs) > u128::from(u64::MAX));
        assert!(local_id(first, 1) > u128::from(u64::MAX));
        assert_eq!(
            canonical_id(first, 0),
            Err(ApplicationError::InvalidIdentity)
        );
        assert_eq!(
            canonical_id(first, u128::from(u64::MAX) + 1),
            Err(ApplicationError::InvalidIdentity)
        );
    }
}
