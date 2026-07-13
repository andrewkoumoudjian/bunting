#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Worker-independent authoritative persistence contract for engine-owned state.

pub use bunting_engine::RunState;
use bunting_market_events::EventEnvelope;
use bunting_market_types::{CommandId, EventSequence, OrderId, RunId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Stable persisted command response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    pub accepted: bool,
    pub reject_code: Option<String>,
    pub committed_sequence: EventSequence,
    pub order_id: Option<OrderId>,
    pub snapshot_checksum: Option<String>,
}

/// One atomic expected-version commit request.
#[derive(Clone, Debug)]
pub struct CommitRequest {
    pub run_id: RunId,
    pub command_id: CommandId,
    pub fingerprint: String,
    pub expected_version: EventSequence,
    pub events: Vec<EventEnvelope>,
    pub result: CommandResult,
    /// Complete candidate state produced only by `bunting-engine`.
    pub candidate: RunState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    Committed(CommandResult),
    Duplicate(CommandResult),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OriginError {
    UnknownRun,
    VersionConflict { current: EventSequence },
    IdempotencyConflict,
    InvalidCommit,
    Unavailable,
}

impl fmt::Display for OriginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for OriginError {}

/// Atomic origin persistence boundary.
pub trait OriginStore {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError>;

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError>;

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError>;
}

#[derive(Debug, Default)]
struct MemoryState {
    runs: BTreeMap<RunId, RunState>,
    commands: BTreeMap<(RunId, CommandId), (String, CommandResult)>,
    events: BTreeMap<RunId, Vec<EventEnvelope>>,
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryOrigin {
    inner: Arc<Mutex<MemoryState>>,
}

impl InMemoryOrigin {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        state.runs.insert(run.run_id(), run);
        Ok(())
    }

    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        let state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(state.events.get(&run_id).cloned().unwrap_or_default())
    }
}

impl OriginStore for InMemoryOrigin {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        let state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        state
            .runs
            .get(&run_id)
            .cloned()
            .ok_or(OriginError::UnknownRun)
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        let state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(state.commands.get(&(run_id, command_id)).cloned())
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        let mut state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if let Some((fingerprint, result)) =
            state.commands.get(&(request.run_id, request.command_id))
        {
            return if fingerprint == &request.fingerprint {
                Ok(CommitOutcome::Duplicate(result.clone()))
            } else {
                Err(OriginError::IdempotencyConflict)
            };
        }
        let current = state
            .runs
            .get(&request.run_id)
            .ok_or(OriginError::UnknownRun)?
            .sequence();
        if current != request.expected_version {
            return Err(OriginError::VersionConflict { current });
        }
        let next = request
            .expected_version
            .checked_add(EventSequence::new(1))
            .ok_or(OriginError::InvalidCommit)?;
        if request.candidate.run_id() != request.run_id
            || request.candidate.sequence() != next
            || request.result.committed_sequence != next
            || request
                .events
                .last()
                .is_some_and(|event| event.sequence != request.candidate.event_sequence())
        {
            return Err(OriginError::InvalidCommit);
        }
        state
            .events
            .entry(request.run_id)
            .or_default()
            .extend(request.events);
        state.commands.insert(
            (request.run_id, request.command_id),
            (request.fingerprint, request.result.clone()),
        );
        state.runs.insert(request.run_id, request.candidate);
        Ok(CommitOutcome::Committed(request.result))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bunting_engine::{
        ListingDefinition, ParticipantDefinition, ScenarioDefinition, TransitionOutcome,
    };
    use bunting_market_events::{Command, CommandPayload};
    use bunting_market_types::{
        CorrelationId, InstrumentId, IterationId, ListingKey, LogicalTimeNs, MoneyMinor,
        ParticipantId, PriceBounds, PriceTicks, QuantityLots, ScenarioId, ScenarioVersion, VenueId,
    };
    use bunting_risk_engine::RiskLimits;
    use std::collections::BTreeMap;
    use std::sync::Barrier;
    use std::thread;

    fn run() -> RunState {
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [ListingDefinition::new(
                ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                "ONE".to_string(),
                PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
            )
            .unwrap()],
            [ParticipantDefinition::new(
                ParticipantId::new(1),
                true,
                RiskLimits {
                    max_order_quantity: QuantityLots::new(100),
                    max_open_order_quantity: QuantityLots::new(1_000),
                    max_absolute_position: QuantityLots::new(1_000),
                },
                MoneyMinor::new(1_000),
                BTreeMap::new(),
            )],
        )
        .unwrap();
        RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap()
    }

    fn request(command_id: u128) -> CommitRequest {
        let initial = run();
        let command = Command {
            run_id: initial.run_id(),
            command_id: CommandId::new(command_id),
            correlation_id: CorrelationId::new(command_id),
            logical_time: LogicalTimeNs::new(1),
            expected_sequence: initial.sequence(),
            actor: ParticipantId::new(1),
            payload: CommandPayload::ActivateKillSwitch,
        };
        let TransitionOutcome {
            candidate, events, ..
        } = initial.transition(&command, None).unwrap();
        CommitRequest {
            run_id: command.run_id,
            command_id: command.command_id,
            fingerprint: command_id.to_string(),
            expected_version: command.expected_sequence,
            events,
            result: CommandResult {
                accepted: true,
                reject_code: None,
                committed_sequence: candidate.sequence(),
                order_id: None,
                snapshot_checksum: None,
            },
            candidate,
        }
    }

    #[test]
    fn same_expected_version_cannot_commit_twice() {
        let origin = InMemoryOrigin::new();
        origin.insert_run(run()).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let first_origin = origin.clone();
        let first_barrier = barrier.clone();
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_origin.commit(request(1))
        });
        let second_origin = origin.clone();
        let second_barrier = barrier.clone();
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_origin.commit(request(2))
        });
        barrier.wait();
        let outcomes = [first.join(), second.join()];
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(Ok(CommitOutcome::Committed(_)))))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, Ok(Err(OriginError::VersionConflict { .. }))))
                .count(),
            1
        );
    }

    #[test]
    fn state_envelope_and_result_round_trip_exactly() {
        let request = request(9);
        let envelope = request.candidate.snapshot_envelope().unwrap();
        let restored =
            bunting_engine::EngineSnapshotEnvelope::from_json(&envelope.to_json().unwrap())
                .unwrap();
        assert_eq!(restored.state, request.candidate);
        let result_json = serde_json::to_string(&request.result).unwrap();
        assert_eq!(
            serde_json::from_str::<CommandResult>(&result_json).unwrap(),
            request.result
        );
    }
}
