#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Transport-neutral application service around the authoritative Bunting engine.

use bunting_api_contract::{ActorIdentity, ActorRole};
use bunting_command_transaction::{
    CachedSnapshot, CommandTransaction, ExecutedTransaction, PreparedCommand, SnapshotCache,
    TransactionError, prepare_command,
};
use bunting_engine::RunState;
use bunting_market_events::{Command, CommandPayload};
use bunting_market_types::{
    CorrelationId, EventSequence, InstrumentId, ListingKey, LogicalTimeNs, ParticipantId, RunId,
};
use bunting_origin_store::OriginStore;
use quarcc_bunting_adapter::{AdapterError, BuntingCommandContext, BuntingExecutionAdapter};
use quarcc_execution_engine::{
    ExecutionAction, ExecutionActionBuffer, ExecutionConfig, ExecutionEngine, ExecutionError,
    ExecutionIntent, ExecutionSnapshot, QuarccExecutionEngine,
    ids::{ClientOrderId, IntentId, LocalOrderId},
};
use serde::{Deserialize, Serialize};
use simfix_mapping::{InboundApplication, MappingContext, MappingError, map_inbound};
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
        CommandPayload::SubmitOrder(order) => Some(order.participant_id),
        CommandPayload::CancelOrder(cancel) => Some(cancel.participant_id),
        CommandPayload::ActivateKillSwitch | CommandPayload::NbcDone(_) => None,
    };
    if payload_participant.is_some_and(|value| value != participant) {
        return Err(ApplicationError::ActorMismatch);
    }
    Ok(())
}

/// Worker-compatible authenticated prepare step. Persistence remains adapter-owned.
pub fn prepare_authenticated(
    actor: &VerifiedActor,
    command: &Command,
    candidate: &RunState,
    cached: Option<&CachedSnapshot>,
) -> Result<PreparedCommand, ApplicationError> {
    authorize_command(actor, command)?;
    prepare_command(command, candidate, cached).map_err(ApplicationError::from)
}

#[derive(Debug)]
pub struct ApplicationService<'a, O, C> {
    origin: &'a O,
    cache: &'a C,
}

impl<'a, O, C> ApplicationService<'a, O, C>
where
    O: OriginStore,
    C: SnapshotCache,
{
    #[must_use]
    pub const fn new(origin: &'a O, cache: &'a C) -> Self {
        Self { origin, cache }
    }

    /// Executes one authenticated command and returns only origin-committed facts.
    pub fn execute(
        &self,
        actor: &VerifiedActor,
        command: &Command,
    ) -> Result<ExecutedTransaction, ApplicationError> {
        authorize_command(actor, command)?;
        CommandTransaction::new(self.origin, self.cache)
            .execute_detailed(command)
            .map_err(ApplicationError::from)
    }

    pub fn recover(&self, run_id: RunId) -> Result<RunState, ApplicationError> {
        self.origin
            .load_run(run_id)
            .map_err(TransactionError::from)
            .map_err(ApplicationError::from)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketProjection {
    pub run_id: RunId,
    pub instrument_id: InstrumentId,
    pub sequence: EventSequence,
    pub bids: Vec<(i64, i64)>,
    pub asks: Vec<(i64, i64)>,
}

pub fn project_market(
    state: &RunState,
    instrument_id: InstrumentId,
) -> Result<MarketProjection, ApplicationError> {
    let listing_key = state
        .listing_key_for_instrument(instrument_id)
        .map_err(|_| ApplicationError::UnknownInstrument)?;
    let (bids, asks) = state
        .visible_levels(listing_key)
        .map_err(|_| ApplicationError::UnknownInstrument)?;
    let convert = |levels: Vec<(u128, u64)>| {
        levels
            .into_iter()
            .map(|(price, quantity)| {
                Ok((
                    i64::try_from(price).map_err(|_| ApplicationError::UnknownInstrument)?,
                    i64::try_from(quantity).map_err(|_| ApplicationError::UnknownInstrument)?,
                ))
            })
            .collect::<Result<Vec<_>, ApplicationError>>()
    };
    Ok(MarketProjection {
        run_id: state.run_id(),
        instrument_id,
        sequence: state.sequence(),
        bids: convert(bids)?,
        asks: convert(asks)?,
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
        instrument_id: InstrumentId,
        subscription: bool,
        market_depth: usize,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FixApplicationState {
    execution: QuarccExecutionEngine,
    adapter: BuntingExecutionAdapter,
    next_intent_id: u128,
    client_order_ids: BTreeMap<LocalOrderId, ClientOrderId>,
}

impl FixApplicationState {
    #[must_use]
    pub fn new(config: ExecutionConfig) -> Self {
        Self {
            execution: QuarccExecutionEngine::new(config),
            adapter: BuntingExecutionAdapter::default(),
            next_intent_id: 1,
            client_order_ids: BTreeMap::new(),
        }
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
            InboundApplication::MarketDataRequest {
                request_id,
                instrument_id,
                subscription,
                market_depth,
                ..
            } => Ok(FixApplicationRequest::MarketData {
                request_id,
                instrument_id,
                subscription,
                market_depth,
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
                let command = self.adapter.command_for_action(
                    action,
                    &BuntingCommandContext {
                        run_id: context.run_id,
                        actor: context.actor,
                        expected_sequence: context.expected_sequence,
                        logical_time: context.logical_time,
                        correlation_id: context.correlation_id,
                    },
                )?;
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
        let mut reports = self.adapter.normalize_committed_events(actor, events)?;
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

#[must_use]
pub fn listing_for_command(state: &RunState, command: &Command) -> Option<ListingKey> {
    match &command.payload {
        CommandPayload::SubmitOrder(order) => {
            state.listing_key_for_instrument(order.instrument_id).ok()
        }
        CommandPayload::CancelOrder(cancel) => state
            .ownership()
            .get(&cancel.order_id)
            .map(|owned| owned.listing_key),
        CommandPayload::ActivateKillSwitch | CommandPayload::NbcDone(_) => None,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bunting_api_contract::UnsignedDecimalString;
    use bunting_command_transaction::InMemorySnapshotCache;
    use bunting_engine::{ListingDefinition, ParticipantDefinition, ScenarioDefinition};
    use bunting_market_events::{OrderKind, Side, SubmitOrder};
    use bunting_market_types::{
        CommandId, InstrumentId, IterationId, MoneyMinor, OrderId, PriceBounds, PriceTicks,
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
            [ListingDefinition::new(
                ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                "ONE".to_owned(),
                PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
            )
            .unwrap()],
            [ParticipantDefinition::new(
                ParticipantId::new(7),
                true,
                RiskLimits {
                    max_order_quantity: QuantityLots::new(100),
                    max_open_order_quantity: QuantityLots::new(1_000),
                    max_absolute_position: QuantityLots::new(1_000),
                },
                MoneyMinor::new(100_000),
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
            }),
        }
    }

    #[test]
    fn commits_before_returning_and_recovers_same_projection() {
        let origin = InMemoryOrigin::new();
        origin.insert_run(run()).unwrap();
        let cache = InMemorySnapshotCache::new();
        let service = ApplicationService::new(&origin, &cache);
        let executed = service.execute(&actor(7), &command()).unwrap();
        assert!(!executed.duplicate);
        assert_eq!(executed.result.committed_sequence, EventSequence::new(1));
        assert_eq!(service.recover(RunId::new(1)).unwrap(), executed.state);
    }

    #[test]
    fn identity_cannot_override_command_participant() {
        assert_eq!(
            authorize_command(&actor(8), &command()),
            Err(ApplicationError::ActorMismatch)
        );
    }
}
