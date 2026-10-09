#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Command orchestration over the writer-owned origin.
//!
//! The origin applies each input to its live run in place, journals it and
//! only then returns; this crate gives order flow and simulation commands
//! one call shape and one error type.

use bunting_engine::EngineError;
use bunting_market_events::{Command, EventEnvelope, SimulationCommandRequest};
use bunting_origin_store::{CommandResult, Executed, JournalInput, OriginError, OriginStore};
pub use bunting_origin_store::{command_fingerprint, simulation_command_fingerprint};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    Origin(OriginError),
    IdempotencyConflict,
    Engine(EngineError),
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TransactionError {}

impl From<OriginError> for TransactionError {
    fn from(error: OriginError) -> Self {
        match error {
            OriginError::IdempotencyConflict => Self::IdempotencyConflict,
            OriginError::Engine(error) => Self::Engine(error),
            other => Self::Origin(other),
        }
    }
}

#[derive(Debug)]
pub struct CommandTransaction<'a, O> {
    origin: &'a O,
}

/// Committed facts of one command. The run itself stays with the origin;
/// read it through [`OriginStore::read_run`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutedTransaction {
    pub result: CommandResult,
    /// Empty for a duplicate: its events were published when it committed.
    pub events: Vec<EventEnvelope>,
    pub duplicate: bool,
}

impl From<Executed> for ExecutedTransaction {
    fn from(executed: Executed) -> Self {
        Self {
            result: executed.result,
            events: executed.events,
            duplicate: executed.duplicate,
        }
    }
}

impl<'a, O> CommandTransaction<'a, O>
where
    O: OriginStore,
{
    #[must_use]
    pub const fn new(origin: &'a O) -> Self {
        Self { origin }
    }

    pub fn execute(&self, command: &Command) -> Result<CommandResult, TransactionError> {
        self.execute_detailed(command)
            .map(|executed| executed.result)
    }

    /// Executes one order-flow command and returns its committed events.
    pub fn execute_detailed(
        &self,
        command: &Command,
    ) -> Result<ExecutedTransaction, TransactionError> {
        self.execute_input(&JournalInput::Command(command.clone()))
    }

    /// Executes one simulation-domain command through the same origin path.
    pub fn execute_simulation_detailed(
        &self,
        request: &SimulationCommandRequest,
    ) -> Result<ExecutedTransaction, TransactionError> {
        self.execute_input(&JournalInput::Simulation(request.clone()))
    }

    fn execute_input(&self, input: &JournalInput) -> Result<ExecutedTransaction, TransactionError> {
        self.origin
            .execute(input)
            .map(ExecutedTransaction::from)
            .map_err(TransactionError::from)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bunting_engine::{ListingDefinition, ParticipantDefinition, RunState, ScenarioDefinition};
    use bunting_market_events::{
        CancelOrder, CommandPayload, OrderKind, Side, SimulationCommand, SimulationCommandRequest,
        SubmitOrder,
    };
    use bunting_market_types::CurrencyId;
    use bunting_market_types::{
        CommandId, CorrelationId, EventSequence, InstrumentId, IterationId, ListingKey,
        LogicalTimeNs, MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots,
        RunId, ScenarioId, ScenarioVersion, VenueId,
    };
    use bunting_origin_store::{InMemoryOrigin, OriginStore};
    use bunting_risk_engine::RiskLimits;
    use std::collections::BTreeMap;

    fn setup() -> InMemoryOrigin {
        let participant = |id| {
            ParticipantDefinition::new(
                ParticipantId::new(id),
                true,
                RiskLimits::new(
                    QuantityLots::new(100),
                    QuantityLots::new(1_000),
                    QuantityLots::new(1_000),
                ),
                BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(100_000))]),
                BTreeMap::from([(InstrumentId::new(1), QuantityLots::new(100))]),
            )
        };
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
                "ONE".to_string(),
                PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
            )
            .unwrap()],
            [participant(1), participant(2)],
        )
        .unwrap();
        let run = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap();
        let origin = InMemoryOrigin::new();
        origin.insert_run(run).unwrap();
        origin
    }

    fn submit(
        sequence: EventSequence,
        command_id: u128,
        participant: u128,
        order_id: u128,
        side: Side,
        price: i64,
        quantity: i64,
    ) -> Command {
        Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(command_id),
            correlation_id: CorrelationId::new(command_id),
            logical_time: LogicalTimeNs::new(u64::try_from(command_id).unwrap() * 1_000_000),
            expected_sequence: sequence,
            actor: ParticipantId::new(participant),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(order_id),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(participant),
                side,
                quantity: QuantityLots::new(quantity),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(price),
                },
            }),
        }
    }

    #[test]
    fn duplicate_cross_cancel_and_restart_recovery_remain_transactional() {
        let origin = setup();
        let transaction = CommandTransaction::new(&origin);
        let sell = submit(EventSequence::new(0), 10, 1, 1, Side::Sell, 100, 10);
        let rested = transaction.execute(&sell).unwrap();
        assert_eq!(transaction.execute(&sell).unwrap(), rested);
        let buy = submit(rested.committed_sequence, 20, 2, 2, Side::Buy, 110, 4);
        let crossed = transaction.execute(&buy).unwrap();
        let cancel = Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(30),
            correlation_id: CorrelationId::new(30),
            logical_time: LogicalTimeNs::new(30_000_000),
            expected_sequence: crossed.committed_sequence,
            actor: ParticipantId::new(1),
            payload: CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(1),
                participant_id: ParticipantId::new(1),
            }),
        };
        assert!(transaction.execute(&cancel).unwrap().accepted);
        let restored = origin.clone_run(RunId::new(1)).unwrap();
        let envelope = restored.snapshot_envelope().unwrap();
        assert_eq!(
            bunting_engine::EngineSnapshotEnvelope::from_json(&envelope.to_json().unwrap())
                .unwrap()
                .state,
            restored
        );
    }

    #[test]
    fn stale_versions_are_rejected_by_origin_authority() {
        let origin = setup();
        let transaction = CommandTransaction::new(&origin);
        let command = submit(EventSequence::new(0), 10, 1, 1, Side::Buy, 100, 1);
        let result = transaction.execute(&command).unwrap();
        assert!(result.accepted);
        let stale = submit(EventSequence::new(0), 11, 1, 2, Side::Buy, 100, 1);
        assert!(matches!(
            transaction.execute(&stale),
            Err(TransactionError::Origin(
                OriginError::VersionConflict { .. }
            ))
        ));
        assert_eq!(
            origin.clone_run(RunId::new(1)).unwrap().sequence(),
            EventSequence::new(1)
        );
    }

    #[test]
    fn simulation_commands_use_the_same_idempotent_origin_commit() {
        let origin = setup();
        let transaction = CommandTransaction::new(&origin);
        let request = SimulationCommandRequest {
            run_id: RunId::new(1),
            command_id: CommandId::new(50),
            correlation_id: CorrelationId::new(50),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: EventSequence::new(0),
            actor: ParticipantId::new(99),
            payload: SimulationCommand::PauseRun,
        };
        let committed = transaction.execute_simulation_detailed(&request).unwrap();
        assert!(!committed.duplicate);
        let duplicate = transaction.execute_simulation_detailed(&request).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.result, committed.result);
        assert_eq!(
            origin.clone_run(RunId::new(1)).unwrap().sequence(),
            EventSequence::new(1)
        );
    }
}
