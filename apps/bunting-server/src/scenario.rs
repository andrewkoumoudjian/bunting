use crate::admission::{AdmissionService, Inbound, Task};
use crate::config::{DeploymentProfile, ScenarioRuntimeConfig, ServerConfig};
use crate::distributor::{PublishingOrigin, Subscription};
use bunting_admission_sequencer::{DelayEstimator, Endpoint};
use bunting_application::{ApplicationService, VerifiedActor, listing_for_command};
use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_events::{Command, EventEnvelope};
use bunting_market_types::{ParticipantId, RunId};
use bunting_origin_store::OriginStore;
use bunting_runtime::{DeterministicRuntime, RuntimeError, RuntimeHost};
use std::collections::BTreeMap;
use std::fs;
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
}

impl Host<'_> {
    fn drain(&self) -> Result<Vec<EventEnvelope>, RuntimeError> {
        Ok(self
            .committed
            .drain()
            .map_err(RuntimeError::Host)?
            .iter()
            .flat_map(|batch| batch.events.iter().cloned())
            .collect())
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
        let participant = command.actor;
        let destination = self
            .read_state(command.run_id, |state| listing_for_command(state, command))?
            .map_or(Endpoint::Hub, |listing| Endpoint::Venue(listing.venue_id));
        let floor = (participant, destination);
        let (reply, result) = sync_channel(1);
        let (actor, mut command) = (actor.clone(), command.clone());
        let task: Task = Box::new(move |context| {
            let service = ApplicationService::new(context.origin);
            let outcome = service
                .read(command.run_id, RunState::sequence)
                .and_then(|sequence| {
                    command.expected_sequence = sequence;
                    command.logical_time = context.logical_time;
                    service.execute_admitted(&actor, &command, context.admission)
                })
                .map(|_| ())
                .map_err(|error| format!("runtime command failed: {error}"));
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
        result
            .recv_timeout(AGENT_REPLY_TIMEOUT)
            .map_err(|error| RuntimeError::Host(format!("agent command not released: {error}")))?
            .map_err(RuntimeError::Host)?;
        // Published before the reply: everything up to this commit.
        self.drain()
    }

    fn take_committed(&mut self) -> Result<Vec<EventEnvelope>, RuntimeError> {
        self.drain()
    }
}

pub(crate) fn run(
    config: &ScenarioRuntimeConfig,
    origin: &PublishingOrigin,
    admission: &AdmissionService,
) -> Result<(), String> {
    let mut runtime = DeterministicRuntime::new(config.scheduler.clone())
        .map_err(|error| format!("invalid deterministic runtime: {error}"))?;
    let mut host = Host {
        origin,
        admission,
        committed: origin.distributor().subscribe(None)?,
        release_floors: BTreeMap::new(),
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
    use bunting_market_types::ParticipantId;

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
