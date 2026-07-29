use crate::config::{DeploymentProfile, ScenarioRuntimeConfig, ServerConfig};
use crate::storage::NativeOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_application::{ApplicationService, VerifiedActor};
use bunting_command_transaction::InMemorySnapshotCache;
use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_types::RunId;
use bunting_origin_store::OriginStore;
use bunting_runtime::{DeterministicRuntime, RuntimeError, RuntimeHost};
use std::fs;
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

struct Host<'a> {
    origin: &'a NativeOrigin,
    cache: &'a InMemorySnapshotCache,
}

impl RuntimeHost for Host<'_> {
    fn state(&self, run_id: RunId) -> Result<RunState, RuntimeError> {
        self.origin
            .load_run(run_id)
            .map_err(|error| RuntimeError::Host(format!("origin store error: {error}")))
    }

    fn commit(
        &mut self,
        actor: &VerifiedActor,
        command: &bunting_market_events::Command,
    ) -> Result<Vec<bunting_market_events::EventEnvelope>, RuntimeError> {
        ApplicationService::new(self.origin, self.cache)
            .execute(actor, command)
            .map(|executed| executed.events)
            .map_err(|error| RuntimeError::Host(format!("runtime command failed: {error}")))
    }
}

pub(crate) fn run(
    config: &ScenarioRuntimeConfig,
    origin: &NativeOrigin,
    cache: &InMemorySnapshotCache,
    writer: &AuthoritativeWriter,
) -> Result<(), String> {
    let mut runtime = DeterministicRuntime::new(config.scheduler.clone())
        .map_err(|error| format!("invalid deterministic runtime: {error}"))?;
    let mut host = Host { origin, cache };
    let cadence = Duration::from_millis(config.wall_tick_ms);
    loop {
        let started = Instant::now();
        {
            let _writer_guard = writer.lock()?;
            runtime
                .advance(&mut host)
                .map_err(|error| format!("deterministic runtime failed: {error}"))?;
        }
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
