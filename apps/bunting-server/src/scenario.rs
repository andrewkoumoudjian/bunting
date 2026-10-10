use crate::admission::{AdmissionService, Inbound, Task};
use crate::config::{
    DeploymentProfile, ScenarioRuntimeConfig, ServerConfig, StorageConfig, StorageKind,
};
use crate::distributor::{PublishingOrigin, Subscription};
use crate::storage::persist_json;
use bunting_admission_sequencer::{DelayEstimator, Endpoint};
use bunting_application::{ApplicationService, VerifiedActor, listing_for_command};
use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_events::{Command, EventEnvelope};
use bunting_market_types::{CommandId, ParticipantId, RunId};
use bunting_origin_store::OriginStore;
use bunting_runtime::{
    DeterministicRuntime, RuntimeConfig, RuntimeError, RuntimeHost, RuntimeSnapshot,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::sync_channel;
use std::thread;
use std::time::{Duration, Instant};

pub(crate) fn bootstrap(
    config: &ServerConfig,
) -> Result<Option<(u128, u128, ScenarioDefinition)>, String> {
    let (run_id, iteration_id, bytes) = if let Some(scenario) = &config.scenario {
        let bytes = fs::read(&scenario.path).map_err(|error| {
            format!("cannot read immutable scenario {}: {error}", scenario.path)
        })?;
        (scenario.run_id, scenario.iteration_id, bytes)
    } else if config.profile == DeploymentProfile::Local {
        (1, 1, include_bytes!("../config/scenario.json").to_vec())
    } else {
        return Ok(None);
    };
    if bytes.len() > 4 * 1_024 * 1_024 {
        return Err("scenario exceeds 4194304 bytes".to_owned());
    }
    let definition = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid scenario JSON: {error}"))?;
    Ok(Some((run_id, iteration_id, definition)))
}

/// Longest a built-in agent waits for its released command's result.
const AGENT_REPLY_TIMEOUT: Duration = Duration::from_secs(30);

const AGENT_CHECKPOINT_VERSION: u16 = 1;

/// The agent runtime as of its latest decisions, written next to the file
/// origin before any of them is submitted (see [`RuntimeHost::checkpoint`]).
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AgentCheckpoint {
    version: u16,
    /// Every event up to this sequence had been handed to the runtime.
    handed_through: u64,
    runtime: RuntimeSnapshot,
}

/// Where a file origin keeps its agent checkpoint; memory origins keep none.
pub(crate) fn checkpoint_path(storage: &StorageConfig) -> Option<PathBuf> {
    match (storage.kind, storage.path.as_deref()) {
        (StorageKind::File, Some(path)) => Some(Path::new(path).with_extension("agents.json")),
        _ => None,
    }
}

fn load_checkpoint(path: &Path) -> Result<Option<AgentCheckpoint>, String> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(format!(
                "cannot read agent checkpoint {}: {error}",
                path.display()
            ));
        }
    };
    let checkpoint: AgentCheckpoint = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid agent checkpoint {}: {error}", path.display()))?;
    if checkpoint.version != AGENT_CHECKPOINT_VERSION {
        return Err(format!(
            "agent checkpoint {} has unsupported version {}",
            path.display(),
            checkpoint.version
        ));
    }
    Ok(Some(checkpoint))
}

/// A runtime ready to continue the run, the committed events it has not
/// seen yet, and the sequence through which events are accounted for.
struct Resumed {
    runtime: DeterministicRuntime,
    backlog: Vec<EventEnvelope>,
    handed_through: u64,
}

/// Restores the agents from their checkpoint, or starts them fresh on a
/// run they have never traded in. Other participants' commits after the
/// checkpoint become the backlog; the agents' own commands after it are the
/// checkpoint's pending actions and resolve to their recorded events.
fn resume(
    scheduler: &RuntimeConfig,
    saved: Option<AgentCheckpoint>,
    durable: &[EventEnvelope],
) -> Result<Resumed, String> {
    let latest = durable.last().map_or(0, |event| event.sequence.get());
    let Some(saved) = saved else {
        let runtime = DeterministicRuntime::new(scheduler.clone())
            .map_err(|error| format!("invalid deterministic runtime: {error}"))?;
        let agents = runtime.participants().collect::<BTreeSet<_>>();
        if durable.iter().any(|event| agents.contains(&event.actor)) {
            return Err(
                "the run has built-in agent commands but no agent checkpoint: agents cannot resume"
                    .to_owned(),
            );
        }
        return Ok(Resumed {
            runtime,
            backlog: Vec::new(),
            handed_through: latest,
        });
    };
    if saved.runtime.config != *scheduler {
        return Err(
            "the agent checkpoint was written for a different runtime configuration".to_owned(),
        );
    }
    let runtime = DeterministicRuntime::restore(saved.runtime)
        .map_err(|error| format!("cannot restore agents: {error}"))?;
    let agents = runtime.participants().collect::<BTreeSet<_>>();
    let backlog = durable
        .iter()
        .filter(|event| {
            event.sequence.get() > saved.handed_through && !agents.contains(&event.actor)
        })
        .cloned()
        .collect();
    Ok(Resumed {
        runtime,
        backlog,
        handed_through: latest.max(saved.handed_through),
    })
}

/// Built-in agents' connection to the venue. Their commands go through the
/// same admission sequencer as FIX (ADR 0035): each agent is a participant
/// in the latency map, so its orders reach a venue `L(agent, venue)` after
/// it decides, and every admission is journaled.
struct Host<'a> {
    origin: &'a PublishingOrigin,
    admission: &'a AdmissionService,
    committed: Subscription<'a>,
    /// Each agent's previous release per destination: one path is FIFO.
    release_floors: BTreeMap<(ParticipantId, Endpoint), u64>,
    /// Commands this process submitted whose published batch is not yet
    /// drained: their events came back from `commit`.
    own: BTreeSet<CommandId>,
    /// Committed before this process subscribed, not yet handed over.
    backlog: Vec<EventEnvelope>,
    handed_through: u64,
    checkpoint: Option<PathBuf>,
}

impl Host<'_> {
    /// Events of a command committed before a restart, from the journal.
    fn recorded(&self, command: &Command) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let events = self
            .origin
            .inner()
            .durable_events(command.run_id)
            .map_err(|error| RuntimeError::Host(format!("origin store error: {error}")))?
            .into_iter()
            .filter(|event| event.command_id == command.command_id)
            .collect::<Vec<_>>();
        if events.is_empty() {
            return Err(RuntimeError::Host(format!(
                "agent command {} is already committed but has no durable events",
                command.command_id
            )));
        }
        Ok(events)
    }
}

impl RuntimeHost for Host<'_> {
    fn read_state<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, RuntimeError> {
        self.origin
            .read_run(run_id, read)
            .map_err(|error| RuntimeError::Host(format!("origin store error: {error}")))
    }

    fn commit(
        &mut self,
        actor: &VerifiedActor,
        command: &Command,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        if self
            .origin
            .find_command(command.run_id, command.command_id)
            .map_err(|error| RuntimeError::Host(format!("origin store error: {error}")))?
            .is_some()
        {
            // A resumed runtime re-submitting an action committed before
            // the restart: exactly once.
            return self.recorded(command);
        }
        let participant = command.actor;
        let destination = self
            .read_state(command.run_id, |state| listing_for_command(state, command))?
            .map_or(Endpoint::Hub, |listing| Endpoint::Venue(listing.venue_id));
        let floor = (participant, destination);
        let (reply, result) = sync_channel(1);
        let (actor, mut command) = (actor.clone(), command.clone());
        let command_id = command.command_id;
        let task: Task = Box::new(move |context| {
            let service = ApplicationService::new(context.origin);
            let outcome = context.run_time(command.run_id).and_then(|logical_time| {
                command.logical_time = logical_time;
                service
                    .read(command.run_id, RunState::sequence)
                    .and_then(|sequence| {
                        command.expected_sequence = sequence;
                        service.execute_admitted(&actor, &command, context.admission)
                    })
                    .map(|executed| executed.events)
                    .map_err(|error| format!("runtime command failed: {error}"))
            });
            let _ = reply.try_send(outcome);
        });
        // In-process agents have no access network: only the virtual path.
        let record = self
            .admission
            .admit(
                &Inbound {
                    received_us: self.admission.clock().now_us(),
                    estimator: &DelayEstimator::new(),
                    participant,
                    destination,
                    floor_us: self.release_floors.get(&floor).copied().unwrap_or(0),
                },
                task,
            )
            .map_err(|error| RuntimeError::Host(format!("agent admission: {error}")))?;
        self.release_floors.insert(floor, record.release_us);
        let events = result
            .recv_timeout(AGENT_REPLY_TIMEOUT)
            .map_err(|error| RuntimeError::Host(format!("agent command not released: {error}")))?
            .map_err(RuntimeError::Host)?;
        self.own.insert(command_id);
        Ok(events)
    }

    fn take_committed(&mut self) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let mut events = std::mem::take(&mut self.backlog);
        for batch in self.committed.drain().map_err(RuntimeError::Host)? {
            let Some(first) = batch.events.first() else {
                continue;
            };
            let own = self.own.remove(&first.command_id);
            for event in &batch.events {
                if !own && event.sequence.get() > self.handed_through {
                    events.push(event.clone());
                }
                self.handed_through = self.handed_through.max(event.sequence.get());
            }
        }
        Ok(events)
    }

    fn checkpoint(&mut self, snapshot: &RuntimeSnapshot) -> Result<(), RuntimeError> {
        let Some(path) = &self.checkpoint else {
            return Ok(());
        };
        persist_json(
            path,
            &AgentCheckpoint {
                version: AGENT_CHECKPOINT_VERSION,
                handed_through: self.handed_through,
                runtime: snapshot.clone(),
            },
        )
        .map_err(|error| RuntimeError::Host(format!("agent checkpoint failed: {error}")))
    }
}

pub(crate) fn run(
    config: &ScenarioRuntimeConfig,
    origin: &PublishingOrigin,
    admission: &AdmissionService,
    checkpoint: Option<PathBuf>,
) -> Result<(), String> {
    // Subscribe before reading the journal so no commit falls between.
    let committed = origin.distributor().subscribe(None)?;
    let durable = origin
        .inner()
        .durable_events(config.scheduler.run_id)
        .map_err(|error| format!("origin store error: {error}"))?;
    let saved = checkpoint
        .as_deref()
        .map(load_checkpoint)
        .transpose()?
        .flatten();
    let Resumed {
        mut runtime,
        backlog,
        handed_through,
    } = resume(&config.scheduler, saved, &durable)?;
    drop(durable);
    let mut host = Host {
        origin,
        admission,
        committed,
        release_floors: BTreeMap::new(),
        own: BTreeSet::new(),
        backlog,
        handed_through,
        checkpoint,
    };
    let cadence = Duration::from_millis(config.wall_tick_ms);
    loop {
        let started = Instant::now();
        runtime
            .advance(&mut host)
            .map_err(|error| format!("deterministic runtime failed: {error}"))?;
        thread::sleep(cadence.saturating_sub(started.elapsed()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_market_events::EventPayload;
    use bunting_market_types::{CorrelationId, EventId, EventSequence, LogicalTimeNs};

    fn scheduler() -> Result<RuntimeConfig, String> {
        ServerConfig::local_default()
            .runtime
            .map(|runtime| runtime.scheduler)
            .ok_or_else(|| "local profile has agents".to_owned())
    }

    fn event(sequence: u64, actor: u128) -> EventEnvelope {
        EventEnvelope {
            schema_version: 1,
            run_id: RunId::new(1),
            event_id: EventId::new(u128::from(sequence)),
            sequence: EventSequence::new(sequence),
            logical_time: LogicalTimeNs::new(0),
            actor: ParticipantId::new(actor),
            command_id: CommandId::new(u128::from(sequence)),
            correlation_id: CorrelationId::new(1),
            causation_sequence: None,
            payload: EventPayload::KillSwitchActivated,
        }
    }

    fn sequences(events: &[EventEnvelope]) -> Vec<u64> {
        events.iter().map(|event| event.sequence.get()).collect()
    }

    #[test]
    fn agents_resume_from_their_checkpoint_and_catch_up_on_others_commits() -> Result<(), String> {
        let scheduler = scheduler()?;
        let agent = 10;
        let snapshot = DeterministicRuntime::new(scheduler.clone())
            .map_err(|error| error.to_string())?
            .snapshot();
        let saved = || AgentCheckpoint {
            version: AGENT_CHECKPOINT_VERSION,
            handed_through: 2,
            runtime: snapshot.clone(),
        };
        // Seen: 1–2. After the checkpoint: a team's commit (3), the agent's
        // own crash-window command (4, resolved from its recorded events),
        // and another team commit (5).
        let durable = [
            event(1, 1),
            event(2, agent),
            event(3, 2),
            event(4, agent),
            event(5, 1),
        ];
        let resumed = resume(&scheduler, Some(saved()), &durable)?;
        assert_eq!(sequences(&resumed.backlog), vec![3, 5]);
        assert_eq!(resumed.handed_through, 5);
        assert_eq!(resumed.runtime.snapshot(), snapshot);

        // A checkpoint for another configuration is refused.
        let mut other = scheduler.clone();
        other.max_actions_per_tick += 1;
        assert!(resume(&other, Some(saved()), &durable).is_err());
        // Without a checkpoint, agents start fresh only on a run they never
        // traded in, and skip its history.
        assert!(resume(&scheduler, None, &durable).is_err());
        let fresh = resume(&scheduler, None, &[event(1, 1), event(2, 2)])?;
        assert!(fresh.backlog.is_empty());
        assert_eq!(fresh.handed_through, 2);
        Ok(())
    }

    #[test]
    fn agent_checkpoints_round_trip_beside_the_file_origin() -> Result<(), String> {
        let mut storage = ServerConfig::local_default().storage;
        assert_eq!(checkpoint_path(&storage), None, "memory origins keep none");
        let directory =
            std::env::temp_dir().join(format!("bunting-agent-checkpoint-{}", std::process::id()));
        storage.kind = StorageKind::File;
        storage.path = Some(directory.join("origin.json").display().to_string());
        let path = checkpoint_path(&storage).ok_or("file origins keep one")?;
        assert_eq!(path, directory.join("origin.agents.json"));
        assert!(load_checkpoint(&path)?.is_none());
        let checkpoint = AgentCheckpoint {
            version: AGENT_CHECKPOINT_VERSION,
            handed_through: 7,
            runtime: DeterministicRuntime::new(scheduler()?)
                .map_err(|error| error.to_string())?
                .snapshot(),
        };
        persist_json(&path, &checkpoint).map_err(|error| error.to_string())?;
        let loaded = load_checkpoint(&path)?.ok_or("checkpoint written")?;
        let _ = fs::remove_dir_all(&directory);
        assert_eq!(loaded.handed_through, 7);
        assert_eq!(loaded.runtime, checkpoint.runtime);
        Ok(())
    }

    #[test]
    fn zero_configuration_profile_bootstraps_the_canonical_scenario() -> Result<(), String> {
        let (run_id, iteration_id, scenario) = bootstrap(&ServerConfig::local_default())?
            .ok_or_else(|| "local scenario missing".to_owned())?;
        assert_eq!((run_id, iteration_id), (1, 1));
        assert_eq!(scenario.listings().len(), 1);
        assert_eq!(scenario.participants().len(), 3);
        assert!(
            scenario
                .participants()
                .contains_key(&ParticipantId::new(10))
        );
        Ok(())
    }
}
