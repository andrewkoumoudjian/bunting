use bunting_engine::simulation::ScoreEntry;
use bunting_engine::{EngineSnapshotEnvelope, RunState};
use bunting_market_events::{Command, EventEnvelope, SimulationCommandRequest};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const COMPETITION_ARCHIVE_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivePolicy {
    pub matching_interval_ms: u64,
    pub max_messages_per_interval: usize,
    pub max_open_orders: usize,
    pub reconnect_resting_orders: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ArchivedCommand {
    Participant { command: Command },
    Simulation { request: SimulationCommandRequest },
}

impl<'de> Deserialize<'de> for ArchivedCommand {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct ArchivedCommandVisitor;

        impl<'de> Visitor<'de> for ArchivedCommandVisitor {
            type Value = ArchivedCommand;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a tagged participant or simulation archived command")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut kind = None;
                let mut command = None;
                let mut request = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "kind" => {
                            if kind.is_some() {
                                return Err(de::Error::duplicate_field("kind"));
                            }
                            kind = Some(map.next_value::<String>()?);
                        }
                        "command" => {
                            if command.is_some() {
                                return Err(de::Error::duplicate_field("command"));
                            }
                            command = Some(map.next_value::<Command>()?);
                        }
                        "request" => {
                            if request.is_some() {
                                return Err(de::Error::duplicate_field("request"));
                            }
                            request = Some(map.next_value::<SimulationCommandRequest>()?);
                        }
                        _ => {
                            return Err(de::Error::unknown_field(
                                &key,
                                &["kind", "command", "request"],
                            ));
                        }
                    }
                }
                match (kind.as_deref(), command, request) {
                    (Some("participant"), Some(command), None) => {
                        Ok(ArchivedCommand::Participant { command })
                    }
                    (Some("simulation"), None, Some(request)) => {
                        Ok(ArchivedCommand::Simulation { request })
                    }
                    (None, _, _) => Err(de::Error::missing_field("kind")),
                    (Some("participant"), None, None) => Err(de::Error::missing_field("command")),
                    (Some("simulation"), None, None) => Err(de::Error::missing_field("request")),
                    (Some("participant" | "simulation"), _, _) => Err(de::Error::custom(
                        "archived command payload does not match its kind",
                    )),
                    (Some(other), _, _) => Err(de::Error::unknown_variant(
                        other,
                        &["participant", "simulation"],
                    )),
                }
            }
        }

        deserializer.deserialize_map(ArchivedCommandVisitor)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AcceptedCommandRecord {
    pub arrival_sequence: u64,
    pub command: ArchivedCommand,
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
    pub accepted_commands: Vec<AcceptedCommandRecord>,
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
    InvalidArrivalSequence(usize),
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
    /// Parses and validates one versioned archive.
    ///
    /// # Errors
    /// Returns an archive error for malformed JSON, unsupported versions,
    /// invalid policy values, or an invalid initial snapshot.
    pub fn from_json(json: &str) -> Result<Self, ArchiveError> {
        let archive: Self = serde_json::from_str(json).map_err(|_| ArchiveError::Serialization)?;
        archive.validate()?;
        Ok(archive)
    }

    /// Serializes the archive as stable, human-reviewable JSON.
    ///
    /// # Errors
    /// Returns `Serialization` if a contained value cannot be encoded.
    pub fn to_json(&self) -> Result<String, ArchiveError> {
        serde_json::to_string_pretty(self).map_err(|_| ArchiveError::Serialization)
    }

    /// Validates the archive envelope and competition policy.
    ///
    /// # Errors
    /// Returns an archive error when the version, policy, or initial snapshot
    /// violates the versioned contract.
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
        let mut previous = None;
        for (index, record) in self.accepted_commands.iter().enumerate() {
            if previous.is_some_and(|value| record.arrival_sequence <= value) {
                return Err(ArchiveError::InvalidArrivalSequence(index));
            }
            previous = Some(record.arrival_sequence);
        }
        Ok(())
    }

    /// Replays accepted commands and compares canonical events and final state.
    ///
    /// # Errors
    /// Returns an archive error on validation failure, command rejection,
    /// canonical event drift, or a final checksum mismatch.
    pub fn replay(&self) -> Result<ReplayResult, ArchiveError> {
        self.validate()?;
        let mut state: RunState = self.initial.state.clone();
        let mut events = Vec::new();
        for (index, record) in self.accepted_commands.iter().enumerate() {
            let outcome = match &record.command {
                ArchivedCommand::Participant { command } => state.transition(command, None),
                ArchivedCommand::Simulation { request } => state.transition_simulation(request),
            }
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
    use bunting_market_events::{
        CancelOrder, Command, CommandPayload, OrderKind, Side, SimulationCommand, SubmitOrder,
    };
    use bunting_market_types::{
        CommandId, CorrelationId, EventSequence, InstrumentId, IterationId, LogicalTimeNs, OrderId,
        ParticipantId, PriceTicks, QuantityLots, RunId,
    };

    #[expect(
        clippy::too_many_lines,
        reason = "the replay fixture keeps the complete mixed command stream visible"
    )]
    fn archive() -> Result<CompetitionArchive, ArchiveError> {
        let scenario: ScenarioDefinition = serde_json::from_str(include_str!(
            "../../apps/bunting-server/config/scenario.json"
        ))
        .map_err(|_| ArchiveError::Serialization)?;
        let initial = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|_| ArchiveError::InvalidInitialSnapshot)?;

        let start = SimulationCommandRequest {
            run_id: initial.run_id(),
            command_id: CommandId::new(1),
            correlation_id: CorrelationId::new(1),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: initial.sequence(),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::StartRun,
        };
        let started = initial
            .transition_simulation(&start)
            .map_err(|_| ArchiveError::CommandRejected(0))?;
        let submit = Command {
            run_id: initial.run_id(),
            command_id: CommandId::new(2),
            correlation_id: CorrelationId::new(2),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: started.candidate.sequence(),
            actor: ParticipantId::new(1),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(1),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(1),
                side: Side::Buy,
                quantity: QuantityLots::new(10),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(100),
                },
            }),
        };
        let submitted = started
            .candidate
            .transition(&submit, None)
            .map_err(|_| ArchiveError::CommandRejected(1))?;
        let cancel = Command {
            run_id: initial.run_id(),
            command_id: CommandId::new(3),
            correlation_id: CorrelationId::new(3),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: submitted.candidate.sequence(),
            actor: ParticipantId::new(1),
            payload: CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(1),
                participant_id: ParticipantId::new(1),
            }),
        };
        let canceled = submitted
            .candidate
            .transition(&cancel, None)
            .map_err(|_| ArchiveError::CommandRejected(2))?;
        let pause = SimulationCommandRequest {
            run_id: initial.run_id(),
            command_id: CommandId::new(4),
            correlation_id: CorrelationId::new(4),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: canceled.candidate.sequence(),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::PauseRun,
        };
        let paused = canceled
            .candidate
            .transition_simulation(&pause)
            .map_err(|_| ArchiveError::CommandRejected(3))?;
        let canonical_events = started
            .events
            .iter()
            .chain(&submitted.events)
            .chain(&canceled.events)
            .chain(&paused.events)
            .cloned()
            .collect();
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
            initial: initial
                .snapshot_envelope()
                .map_err(|_| ArchiveError::InvalidInitialSnapshot)?,
            accepted_commands: vec![
                AcceptedCommandRecord {
                    arrival_sequence: 1,
                    command: ArchivedCommand::Simulation { request: start },
                },
                AcceptedCommandRecord {
                    arrival_sequence: 2,
                    command: ArchivedCommand::Participant { command: submit },
                },
                AcceptedCommandRecord {
                    arrival_sequence: 3,
                    command: ArchivedCommand::Participant { command: cancel },
                },
                AcceptedCommandRecord {
                    arrival_sequence: 4,
                    command: ArchivedCommand::Simulation { request: pause },
                },
            ],
            canonical_events,
            final_state_hash: paused
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
        assert_eq!(replay.command_count, 4);
        assert!(replay.event_count > 4);
        Ok(())
    }

    #[test]
    fn replay_rejects_duplicate_arrival_sequence() -> Result<(), ArchiveError> {
        let mut archive = archive()?;
        archive.accepted_commands[2].arrival_sequence = 2;
        assert_eq!(
            archive.replay(),
            Err(ArchiveError::InvalidArrivalSequence(2))
        );
        Ok(())
    }

    #[test]
    fn replay_rejects_non_monotonic_arrival_sequence() -> Result<(), ArchiveError> {
        let mut archive = archive()?;
        archive.accepted_commands[2].arrival_sequence = 1;
        assert_eq!(
            archive.replay(),
            Err(ArchiveError::InvalidArrivalSequence(2))
        );
        Ok(())
    }

    #[test]
    fn replay_rejects_participant_sequence_conflict() -> Result<(), ArchiveError> {
        let mut archive = archive()?;
        let ArchivedCommand::Participant { command } = &mut archive.accepted_commands[1].command
        else {
            return Err(ArchiveError::Serialization);
        };
        command.expected_sequence = EventSequence::new(999);
        assert_eq!(archive.replay(), Err(ArchiveError::CommandRejected(1)));
        Ok(())
    }

    #[test]
    fn replay_rejects_event_drift() -> Result<(), ArchiveError> {
        let mut archive = archive()?;
        archive.canonical_events.clear();
        assert_eq!(archive.replay(), Err(ArchiveError::EventMismatch));
        Ok(())
    }

    #[test]
    fn replay_rejects_final_hash_drift() -> Result<(), ArchiveError> {
        let mut archive = archive()?;
        archive.final_state_hash = "not-the-final-state".to_owned();
        assert_eq!(archive.replay(), Err(ArchiveError::FinalHashMismatch));
        Ok(())
    }
}
