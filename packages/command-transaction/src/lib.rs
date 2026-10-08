#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Sans-I/O recovery and commit coordination around `bunting-engine`.

use bunting_engine::{EngineError, RunState, TransitionOutcome};
use bunting_market_events::{Command, SimulationCommandRequest};
use bunting_origin_store::{CommandResult, CommitOutcome, CommitRequest, OriginError, OriginStore};
use sha2::{Digest, Sha256};
use std::fmt;

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    Origin(OriginError),
    IdempotencyConflict,
    Engine(EngineError),
    Serialization,
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
            other => Self::Origin(other),
        }
    }
}

impl From<EngineError> for TransactionError {
    fn from(error: EngineError) -> Self {
        Self::Engine(error)
    }
}

#[derive(Debug)]
pub struct CommandTransaction<'a, O> {
    origin: &'a O,
}

#[derive(Clone, Debug)]
pub struct PreparedCommand {
    pub commit: CommitRequest,
}

#[derive(Clone, Debug)]
pub struct ExecutedTransaction {
    pub result: CommandResult,
    pub events: Vec<bunting_market_events::EventEnvelope>,
    pub state: RunState,
    pub duplicate: bool,
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

    /// Executes and returns the committed events and complete recovery state.
    pub fn execute_detailed(
        &self,
        command: &Command,
    ) -> Result<ExecutedTransaction, TransactionError> {
        let fingerprint = command_fingerprint(command)?;
        if let Some((stored_fingerprint, result)) = self
            .origin
            .find_command(command.run_id, command.command_id)?
        {
            return if stored_fingerprint == fingerprint {
                Ok(ExecutedTransaction {
                    result,
                    events: Vec::new(),
                    state: self.origin.load_run(command.run_id)?,
                    duplicate: true,
                })
            } else {
                Err(TransactionError::IdempotencyConflict)
            };
        }
        let candidate = self.origin.load_run(command.run_id)?;
        if candidate.sequence() != command.expected_sequence {
            return Err(TransactionError::Origin(OriginError::VersionConflict {
                current: candidate.sequence(),
            }));
        }
        let prepared = prepare_command_owned(command, candidate)?;
        let events = prepared.commit.events.clone();
        let committed_state = prepared.commit.candidate.clone();
        match self.origin.commit(prepared.commit)? {
            CommitOutcome::Committed(result) => Ok(ExecutedTransaction {
                result,
                events,
                state: committed_state,
                duplicate: false,
            }),
            CommitOutcome::Duplicate(result) => Ok(ExecutedTransaction {
                result,
                events: Vec::new(),
                state: self.origin.load_run(command.run_id)?,
                duplicate: true,
            }),
        }
    }

    /// Executes one simulation-domain command through the same atomic origin boundary.
    pub fn execute_simulation_detailed(
        &self,
        request: &SimulationCommandRequest,
    ) -> Result<ExecutedTransaction, TransactionError> {
        let fingerprint = simulation_command_fingerprint(request)?;
        if let Some((stored_fingerprint, result)) = self
            .origin
            .find_command(request.run_id, request.command_id)?
        {
            return if stored_fingerprint == fingerprint {
                Ok(ExecutedTransaction {
                    result,
                    events: Vec::new(),
                    state: self.origin.load_run(request.run_id)?,
                    duplicate: true,
                })
            } else {
                Err(TransactionError::IdempotencyConflict)
            };
        }
        let state = self.origin.load_run(request.run_id)?;
        if state.sequence() != request.expected_sequence {
            return Err(TransactionError::Origin(OriginError::VersionConflict {
                current: state.sequence(),
            }));
        }
        let prepared = prepare_simulation_command_owned(request, state)?;
        let events = prepared.commit.events.clone();
        let committed_state = prepared.commit.candidate.clone();
        match self.origin.commit(prepared.commit)? {
            CommitOutcome::Committed(result) => Ok(ExecutedTransaction {
                result,
                events,
                state: committed_state,
                duplicate: false,
            }),
            CommitOutcome::Duplicate(result) => Ok(ExecutedTransaction {
                result,
                events: Vec::new(),
                state: self.origin.load_run(request.run_id)?,
                duplicate: true,
            }),
        }
    }
}

/// Prepares the engine-owned candidate without origin I/O.
pub fn prepare_command(
    command: &Command,
    candidate: &RunState,
) -> Result<PreparedCommand, TransactionError> {
    prepared_command_outcome(command, candidate.transition(command)?)
}

/// A loaded run is already owned by the transaction; consume it rather than
/// copying every book snapshot, account and history before staging a candidate.
pub fn prepare_command_owned(
    command: &Command,
    candidate: RunState,
) -> Result<PreparedCommand, TransactionError> {
    prepared_command_outcome(command, candidate.transition_owned(command)?)
}

fn prepared_command_outcome(
    command: &Command,
    outcome: TransitionOutcome,
) -> Result<PreparedCommand, TransactionError> {
    let TransitionOutcome {
        candidate,
        events,
        accepted,
        reject_code,
        order_id,
        ..
    } = outcome;
    let result = CommandResult {
        accepted,
        reject_code,
        committed_sequence: candidate.sequence(),
        order_id,
    };
    Ok(PreparedCommand {
        commit: CommitRequest {
            run_id: command.run_id,
            command_id: command.command_id,
            fingerprint: command_fingerprint(command)?,
            client_key: None,
            expected_version: command.expected_sequence,
            events,
            result,
            candidate,
        },
    })
}

/// Prepares one simulation command without origin or cache I/O.
pub fn prepare_simulation_command(
    request: &SimulationCommandRequest,
    state: &RunState,
) -> Result<PreparedCommand, TransactionError> {
    prepared_simulation_outcome(request, state.transition_simulation(request)?)
}

/// Prepares an administration transition by consuming an already loaded run.
pub fn prepare_simulation_command_owned(
    request: &SimulationCommandRequest,
    state: RunState,
) -> Result<PreparedCommand, TransactionError> {
    prepared_simulation_outcome(request, state.transition_simulation_owned(request)?)
}

fn prepared_simulation_outcome(
    request: &SimulationCommandRequest,
    outcome: TransitionOutcome,
) -> Result<PreparedCommand, TransactionError> {
    let TransitionOutcome {
        candidate,
        events,
        accepted,
        reject_code,
        order_id,
        ..
    } = outcome;
    let result = CommandResult {
        accepted,
        reject_code,
        committed_sequence: candidate.sequence(),
        order_id,
    };
    Ok(PreparedCommand {
        commit: CommitRequest {
            run_id: request.run_id,
            command_id: request.command_id,
            fingerprint: simulation_command_fingerprint(request)?,
            client_key: None,
            expected_version: request.expected_sequence,
            events,
            result,
            candidate,
        },
    })
}

pub fn command_fingerprint(command: &Command) -> Result<String, TransactionError> {
    fingerprint(command)
}

/// Returns the stable idempotency fingerprint for a simulation-domain command.
pub fn simulation_command_fingerprint(
    command: &SimulationCommandRequest,
) -> Result<String, TransactionError> {
    fingerprint(command)
}

fn fingerprint(value: &impl serde::Serialize) -> Result<String, TransactionError> {
    let bytes = serde_json::to_vec(value).map_err(|_| TransactionError::Serialization)?;
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
    use bunting_engine::{ListingDefinition, ParticipantDefinition, ScenarioDefinition};
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
    use std::sync::atomic::{AtomicBool, Ordering};

    struct CommitRaceOrigin {
        committed: InMemoryOrigin,
        stale: RunState,
        commit_attempted: AtomicBool,
    }

    impl OriginStore for CommitRaceOrigin {
        fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
            if self.commit_attempted.load(Ordering::SeqCst) {
                self.committed.load_run(run_id)
            } else {
                Ok(self.stale.clone())
            }
        }

        fn find_command(
            &self,
            _run_id: RunId,
            _command_id: CommandId,
        ) -> Result<Option<(String, CommandResult)>, OriginError> {
            Ok(None)
        }

        fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
            let outcome = self.committed.commit(request);
            self.commit_attempted.store(true, Ordering::SeqCst);
            outcome
        }
    }

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
        let restored = origin.load_run(RunId::new(1)).unwrap();
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
            origin.load_run(RunId::new(1)).unwrap().sequence(),
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
            origin.load_run(RunId::new(1)).unwrap().sequence(),
            EventSequence::new(1)
        );
    }

    #[test]
    fn duplicate_commit_race_discards_local_events_and_reloads_origin_state() {
        let origin = setup();
        let stale = origin.load_run(RunId::new(1)).unwrap();
        let command = submit(EventSequence::new(0), 10, 1, 1, Side::Buy, 100, 1);
        CommandTransaction::new(&origin)
            .execute_detailed(&command)
            .unwrap();

        let race = CommitRaceOrigin {
            committed: origin,
            stale,
            commit_attempted: AtomicBool::new(false),
        };
        let duplicate = CommandTransaction::new(&race)
            .execute_detailed(&command)
            .unwrap();

        assert!(duplicate.duplicate);
        assert!(duplicate.events.is_empty());
        assert_eq!(duplicate.state.sequence(), EventSequence::new(1));
    }
}
