//! Versioned competition archive: a run's complete committed history.
//!
//! Version 2 holds the run's genesis snapshot and every journaled
//! [`CommandRecord`] in commit order: order flow, cancels, built-in agent
//! commands, simulation administration and the admission records that
//! ordered them. Replay goes through the origin store's own recovery path,
//! so it re-executes every input and requires identical results, events and
//! hash-chain values. An optional checkpoint lets a verifier skip
//! re-executing the records it covers; those are still checked by
//! fingerprint, sequence and chain.
use bunting_engine::EngineSnapshotEnvelope;
use bunting_engine::simulation::ScoreEntry;
use bunting_origin_store::{CommandRecord, LiveRun, RunLimits, RunRecovery};
use serde::{Deserialize, Serialize};
use std::fmt;

pub const COMPETITION_ARCHIVE_VERSION: u16 = 2;

/// A verified state the archive's records may be replayed from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveCheckpoint {
    pub snapshot: EngineSnapshotEnvelope,
    /// Journal chain value after the last record the snapshot includes.
    pub chain: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CompetitionArchive {
    pub schema_version: u16,
    pub engine_version: String,
    pub genesis: EngineSnapshotEnvelope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checkpoint: Option<ArchiveCheckpoint>,
    pub records: Vec<CommandRecord>,
    /// Journal chain value after the last record.
    pub final_chain: String,
    pub final_state_hash: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReplayResult {
    pub final_state_hash: String,
    pub final_chain: String,
    pub command_count: usize,
    pub event_count: usize,
    pub scores: Vec<ScoreEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ArchiveError {
    Serialization,
    UnsupportedVersion,
    InvalidInitialSnapshot,
    InvalidCheckpoint,
    /// The record at this index does not re-execute to the journaled record.
    RecordMismatch(usize),
    FinalChainMismatch,
    FinalHashMismatch,
}

impl fmt::Display for ArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ArchiveError {}

#[derive(Deserialize)]
struct VersionProbe {
    schema_version: u16,
}

impl CompetitionArchive {
    /// Builds an archive from a run's genesis and its journal records, and
    /// verifies it by replaying it.
    ///
    /// # Errors
    /// Returns an archive error when the records do not replay from genesis.
    pub fn from_journal(
        genesis: EngineSnapshotEnvelope,
        records: Vec<CommandRecord>,
    ) -> Result<Self, ArchiveError> {
        let mut archive = Self {
            schema_version: COMPETITION_ARCHIVE_VERSION,
            engine_version: env!("CARGO_PKG_VERSION").to_owned(),
            genesis,
            checkpoint: None,
            records,
            final_chain: String::new(),
            final_state_hash: String::new(),
        };
        let replay = archive.replay_unchecked()?;
        archive.final_chain = replay.final_chain;
        archive.final_state_hash = replay.final_state_hash;
        Ok(archive)
    }

    /// Parses and validates one versioned archive.
    ///
    /// # Errors
    /// Returns an archive error for malformed JSON, unsupported versions
    /// (including version 1 archives), or an invalid snapshot.
    pub fn from_json(json: &str) -> Result<Self, ArchiveError> {
        let probe: VersionProbe =
            serde_json::from_str(json).map_err(|_| ArchiveError::Serialization)?;
        if probe.schema_version != COMPETITION_ARCHIVE_VERSION {
            return Err(ArchiveError::UnsupportedVersion);
        }
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

    /// Validates the archive envelope and its snapshots.
    ///
    /// # Errors
    /// Returns an archive error when the version or a snapshot violates the
    /// versioned contract.
    pub fn validate(&self) -> Result<(), ArchiveError> {
        if self.schema_version != COMPETITION_ARCHIVE_VERSION {
            return Err(ArchiveError::UnsupportedVersion);
        }
        self.genesis
            .verify()
            .map_err(|_| ArchiveError::InvalidInitialSnapshot)?;
        if let Some(checkpoint) = &self.checkpoint {
            checkpoint
                .snapshot
                .verify()
                .map_err(|_| ArchiveError::InvalidCheckpoint)?;
        }
        Ok(())
    }

    /// Replays every record from genesis (or the checkpoint) through the
    /// origin store's recovery and compares the final chain and state.
    ///
    /// # Errors
    /// Returns an archive error on validation failure, a record that does
    /// not reproduce, or a final chain or state hash mismatch.
    pub fn replay(&self) -> Result<ReplayResult, ArchiveError> {
        self.validate()?;
        let replay = self.replay_unchecked()?;
        if replay.final_chain != self.final_chain {
            return Err(ArchiveError::FinalChainMismatch);
        }
        if replay.final_state_hash != self.final_state_hash {
            return Err(ArchiveError::FinalHashMismatch);
        }
        Ok(replay)
    }

    fn replay_unchecked(&self) -> Result<ReplayResult, ArchiveError> {
        let live = self.recovered()?;
        let state = live.state().map_err(|_| ArchiveError::FinalHashMismatch)?;
        let final_state_hash = state
            .state_hash()
            .map_err(|_| ArchiveError::FinalHashMismatch)?;
        let scores = state
            .simulation()
            .reports
            .last()
            .map_or_else(Vec::new, |report| report.entries.clone());
        Ok(ReplayResult {
            final_state_hash,
            final_chain: live.chain(),
            command_count: self.records.len(),
            event_count: self.records.iter().map(|record| record.events.len()).sum(),
            scores,
        })
    }

    fn recovered(&self) -> Result<LiveRun, ArchiveError> {
        let limits = RunLimits {
            max_commands: self.records.len().max(1),
            max_events: u64::MAX,
            ..RunLimits::default()
        };
        let mut recovery = RunRecovery::new(
            self.genesis.state.clone(),
            self.checkpoint
                .as_ref()
                .map(|checkpoint| (checkpoint.snapshot.state.clone(), checkpoint.chain.as_str())),
            limits,
        )
        .map_err(|_| ArchiveError::InvalidCheckpoint)?;
        for (index, record) in self.records.iter().enumerate() {
            recovery
                .replay(record)
                .map_err(|_| ArchiveError::RecordMismatch(index))?;
        }
        recovery
            .finish()
            .map_err(|_| ArchiveError::InvalidCheckpoint)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_engine::{RunState, ScenarioDefinition};
    use bunting_market_events::{
        Command, CommandPayload, OrderKind, Side, SimulationCommand, SimulationCommandRequest,
        SubmitOrder,
    };
    use bunting_market_types::{
        CommandId, CorrelationId, EventSequence, InstrumentId, IterationId, LogicalTimeNs, OrderId,
        ParticipantId, PriceTicks, QuantityLots, RunId,
    };
    use bunting_origin_store::{Execution, JournalInput};

    fn genesis() -> Result<RunState, ArchiveError> {
        let scenario: ScenarioDefinition = serde_json::from_str(include_str!(
            "../../apps/bunting-server/config/scenario.json"
        ))
        .map_err(|_| ArchiveError::Serialization)?;
        RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|_| ArchiveError::InvalidInitialSnapshot)
    }

    fn order(sequence: u64, id: u128, participant: u128, side: Side) -> JournalInput {
        JournalInput::Command(Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(id),
            correlation_id: CorrelationId::new(id),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: EventSequence::new(sequence),
            actor: ParticipantId::new(participant),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(id),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(participant),
                side,
                quantity: QuantityLots::new(5),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(100),
                },
            }),
        })
    }

    /// Order flow that trades, then a simulation command: the history a
    /// version 1 archive could not replay.
    fn inputs() -> Vec<JournalInput> {
        vec![
            order(0, 1, 10, Side::Sell),
            order(1, 2, 1, Side::Buy),
            order(2, 3, 2, Side::Sell),
            JournalInput::Simulation(SimulationCommandRequest {
                run_id: RunId::new(1),
                command_id: CommandId::new(4),
                correlation_id: CorrelationId::new(4),
                logical_time: LogicalTimeNs::new(0),
                expected_sequence: EventSequence::new(3),
                actor: ParticipantId::new(1),
                payload: SimulationCommand::PauseRun,
            }),
        ]
    }

    /// Commits the inputs as the origin does, returning genesis, the
    /// records, and the live run after `checkpoint_after` records.
    fn journal(
        checkpoint_after: usize,
    ) -> Result<
        (
            EngineSnapshotEnvelope,
            Vec<CommandRecord>,
            ArchiveCheckpoint,
        ),
        ArchiveError,
    > {
        let state = genesis()?;
        let envelope = state
            .snapshot_envelope()
            .map_err(|_| ArchiveError::InvalidInitialSnapshot)?;
        let mut live = LiveRun::genesis(state, RunLimits::default())
            .map_err(|_| ArchiveError::InvalidInitialSnapshot)?;
        let mut records = Vec::new();
        let mut checkpoint = None;
        for (index, input) in inputs().iter().enumerate() {
            match live.execute(input) {
                Ok(Execution::Committed(record)) => records.push(*record),
                _ => return Err(ArchiveError::RecordMismatch(index)),
            }
            if index + 1 == checkpoint_after {
                let state = live
                    .state()
                    .map_err(|_| ArchiveError::InvalidCheckpoint)?
                    .clone();
                checkpoint = Some(ArchiveCheckpoint {
                    snapshot: EngineSnapshotEnvelope::new(state)
                        .map_err(|_| ArchiveError::InvalidCheckpoint)?,
                    chain: live.chain(),
                });
            }
        }
        Ok((
            envelope,
            records,
            checkpoint.ok_or(ArchiveError::InvalidCheckpoint)?,
        ))
    }

    #[test]
    fn full_history_round_trips_and_replays_from_genesis() -> Result<(), ArchiveError> {
        let (genesis, records, _) = journal(2)?;
        assert!(records.iter().any(|record| record.events.len() > 2));
        let archive = CompetitionArchive::from_journal(genesis, records)?;
        let decoded = CompetitionArchive::from_json(&archive.to_json()?)?;
        assert_eq!(decoded, archive);
        let replay = decoded.replay()?;
        assert_eq!(replay.command_count, 4);
        assert_eq!(replay.final_chain, archive.final_chain);
        assert_eq!(
            replay.event_count,
            archive
                .records
                .iter()
                .map(|record| record.events.len())
                .sum::<usize>()
        );
        Ok(())
    }

    #[test]
    fn replay_from_a_checkpoint_matches_replay_from_genesis() -> Result<(), ArchiveError> {
        let (genesis, records, checkpoint) = journal(2)?;
        let mut archive = CompetitionArchive::from_journal(genesis, records)?;
        let full = archive.replay()?;
        archive.checkpoint = Some(checkpoint);
        assert_eq!(archive.replay()?, full);
        Ok(())
    }

    #[test]
    fn a_forged_checkpoint_is_rejected() -> Result<(), ArchiveError> {
        let (genesis, records, mut checkpoint) = journal(2)?;
        let mut archive = CompetitionArchive::from_journal(genesis, records)?;
        checkpoint.chain = archive.final_chain.clone();
        archive.checkpoint = Some(checkpoint);
        assert!(matches!(
            archive.replay(),
            Err(ArchiveError::InvalidCheckpoint | ArchiveError::RecordMismatch(_))
        ));
        Ok(())
    }

    #[test]
    fn tampered_records_are_rejected() -> Result<(), ArchiveError> {
        let (genesis, records, _) = journal(2)?;
        let archive = CompetitionArchive::from_journal(genesis, records)?;

        let mut dropped_event = archive.clone();
        dropped_event.records[1].events.pop();
        assert_eq!(dropped_event.replay(), Err(ArchiveError::RecordMismatch(1)));

        let mut missing_record = archive.clone();
        missing_record.records.remove(2);
        assert_eq!(
            missing_record.replay(),
            Err(ArchiveError::RecordMismatch(2))
        );

        let mut wrong_result = archive.clone();
        wrong_result.records[0].result.accepted = !wrong_result.records[0].result.accepted;
        assert_eq!(wrong_result.replay(), Err(ArchiveError::RecordMismatch(0)));

        let mut wrong_hash = archive;
        wrong_hash.final_state_hash = "0".repeat(64);
        assert_eq!(wrong_hash.replay(), Err(ArchiveError::FinalHashMismatch));
        Ok(())
    }

    #[test]
    fn version_one_archives_are_unsupported() {
        assert_eq!(
            CompetitionArchive::from_json(r#"{"schema_version":1,"accepted_commands":[]}"#),
            Err(ArchiveError::UnsupportedVersion)
        );
    }
}
