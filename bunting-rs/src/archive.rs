use bunting_engine::simulation::ScoreEntry;
use bunting_engine::{EngineSnapshotEnvelope, RunState};
use bunting_market_events::{EventEnvelope, SimulationCommandRequest};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const COMPETITION_ARCHIVE_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePolicy {
    pub matching_interval_ms: u64,
    pub max_messages_per_interval: usize,
    pub max_open_orders: usize,
    pub reconnect_resting_orders: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionArchive {
    pub schema_version: u16,
    pub scenario_id: String,
    pub scenario_version: String,
    pub engine_version: String,
    pub seeds: Vec<u64>,
    pub policy: ArchivePolicy,
    pub initial: EngineSnapshotEnvelope,
    pub accepted_commands: Vec<SimulationCommandRequest>,
    pub canonical_events: Vec<EventEnvelope>,
    pub final_state_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReplayResult {
    pub final_state_hash: String,
    pub command_count: usize,
    pub event_count: usize,
    pub scores: Vec<ScoreEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArchiveError {
    Serialization,
    UnsupportedVersion,
    InvalidPolicy,
    InvalidInitialSnapshot,
    CommandRejected(usize),
    EventMismatch,
    FinalHashMismatch,
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ArchiveError {}

impl CompetitionArchive {
    pub fn from_json(json: &str) -> Result<Self, ArchiveError> {
        let archive: Self = serde_json::from_str(json).map_err(|_| ArchiveError::Serialization)?;
        archive.validate()?;
        Ok(archive)
    }

    pub fn to_json(&self) -> Result<String, ArchiveError> {
        serde_json::to_string_pretty(self).map_err(|_| ArchiveError::Serialization)
    }

    pub fn validate(&self) -> Result<(), ArchiveError> {
        if self.schema_version != COMPETITION_ARCHIVE_VERSION {
            return Err(ArchiveError::UnsupportedVersion);
        }
        if self.policy.matching_interval_ms == 0
            || self.policy.max_messages_per_interval == 0
            || self.policy.max_open_orders == 0
            || !self.policy.reconnect_resting_orders
        {
            return Err(ArchiveError::InvalidPolicy);
        }
        EngineSnapshotEnvelope::from_json(
            &self
                .initial
                .to_json()
                .map_err(|_| ArchiveError::InvalidInitialSnapshot)?,
        )
        .map_err(|_| ArchiveError::InvalidInitialSnapshot)?;
        Ok(())
    }

    pub fn replay(&self) -> Result<ReplayResult, ArchiveError> {
        self.validate()?;
        let mut state: RunState = self.initial.state.clone();
        let mut events = Vec::new();
        for (index, command) in self.accepted_commands.iter().enumerate() {
            let outcome = state
                .transition_simulation(command)
                .map_err(|_| ArchiveError::CommandRejected(index))?;
            if !outcome.accepted {
                return Err(ArchiveError::CommandRejected(index));
            }
            events.extend(outcome.events);
            state = outcome.candidate;
        }
        if events != self.canonical_events {
            return Err(ArchiveError::EventMismatch);
        }
        let final_state_hash = state
            .state_hash()
            .map_err(|_| ArchiveError::FinalHashMismatch)?;
        if final_state_hash != self.final_state_hash {
            return Err(ArchiveError::FinalHashMismatch);
        }
        let scores = state
            .simulation()
            .reports
            .last()
            .map_or_else(Vec::new, |report| report.entries.clone());
        Ok(ReplayResult {
            final_state_hash,
            command_count: self.accepted_commands.len(),
            event_count: events.len(),
            scores,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_engine::ScenarioDefinition;
    use bunting_market_events::SimulationCommand;
    use bunting_market_types::{
        CommandId, CorrelationId, IterationId, LogicalTimeNs, ParticipantId, RunId,
    };

    fn archive() -> Result<CompetitionArchive, ArchiveError> {
        let scenario: ScenarioDefinition = serde_json::from_str(include_str!(
            "../../apps/bunting-server/config/scenario.json"
        ))
        .map_err(|_| ArchiveError::Serialization)?;
        let initial_state = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|_| ArchiveError::InvalidInitialSnapshot)?;
        let command = SimulationCommandRequest {
            run_id: initial_state.run_id(),
            command_id: CommandId::new(1),
            correlation_id: CorrelationId::new(1),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: initial_state.sequence(),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::StartRun,
        };
        let outcome = initial_state
            .transition_simulation(&command)
            .map_err(|_| ArchiveError::CommandRejected(0))?;
        Ok(CompetitionArchive {
            schema_version: COMPETITION_ARCHIVE_VERSION,
            scenario_id: "1".to_owned(),
            scenario_version: "1".to_owned(),
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            seeds: vec![42],
            policy: ArchivePolicy {
                matching_interval_ms: 100,
                max_messages_per_interval: 64,
                max_open_orders: 256,
                reconnect_resting_orders: true,
            },
            initial: initial_state
                .snapshot_envelope()
                .map_err(|_| ArchiveError::InvalidInitialSnapshot)?,
            accepted_commands: vec![command],
            canonical_events: outcome.events,
            final_state_hash: outcome
                .candidate
                .state_hash()
                .map_err(|_| ArchiveError::FinalHashMismatch)?,
        })
    }

    #[test]
    fn archive_round_trip_replays_canonical_bytes() -> Result<(), ArchiveError> {
        let archive = archive()?;
        let decoded = CompetitionArchive::from_json(&archive.to_json()?)?;
        let replay = decoded.replay()?;
        assert_eq!(replay.command_count, 1);
        assert_eq!(replay.event_count, 1);
        Ok(())
    }

    #[test]
    fn replay_rejects_event_drift() -> Result<(), ArchiveError> {
        let mut archive = archive()?;
        archive.canonical_events.clear();
        assert_eq!(archive.replay(), Err(ArchiveError::EventMismatch));
        Ok(())
    }
}
