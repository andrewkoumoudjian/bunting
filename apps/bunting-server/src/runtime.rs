use crate::admission::{AdmissionService, VenueClock};
use crate::config::ServerConfig;
use crate::distributor::{MAX_PENDING_BATCHES, PublishingOrigin};
use crate::storage::NativeOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_engine::RunState;
use bunting_market_types::{IterationId, RunId};
use bunting_origin_store::{OriginError, OriginStore};
use std::sync::{Arc, mpsc};

pub fn run(config: &ServerConfig) -> Result<(), String> {
    config.validate().map_err(|error| error.to_string())?;
    let origin =
        NativeOrigin::from_config(&config.storage).map_err(|error| origin_error(&error))?;
    bootstrap_run(config, &origin)?;
    let clock = VenueClock::start();
    let origin = Arc::new(PublishingOrigin::new(origin, MAX_PENDING_BATCHES, clock));
    let writer = Arc::new(AuthoritativeWriter::new());
    let admission = config
        .fix
        .as_ref()
        .map(|fix| AdmissionService::new(clock, fix.admission.clone()).map(Arc::new))
        .transpose()?;
    let (completed, listener) = mpsc::channel();
    let mut task_count = 0_usize;
    if let Some(admin) = config.admin.clone() {
        let origin = origin.clone();
        let admission = admission.clone();
        let completed = completed.clone();
        spawn_host("bunting-admin", completed, move || {
            crate::admin::run(&admin, origin.inner(), admission.as_deref())
        })?;
        task_count = task_count.saturating_add(1);
    }
    if let Some(admission) = admission.clone() {
        let origin = origin.clone();
        let writer = writer.clone();
        let completed = completed.clone();
        spawn_host("bunting-sequencer", completed, move || {
            admission.run_sequencer(&origin, &writer)
        })?;
    }
    if let Some(runtime) = config.runtime.clone() {
        let origin = origin.clone();
        let writer = writer.clone();
        let completed = completed.clone();
        spawn_host("bunting-scenario", completed, move || {
            crate::scenario::run(&runtime, &origin, &writer)
        })?;
        task_count = task_count.saturating_add(1);
    }
    if let (Some(fix), Some(admission)) = (config.fix.clone(), admission) {
        let origin = origin.clone();
        let storage_kind = config.storage.kind;
        let storage_path = config.storage.path.clone();
        let completed = completed.clone();
        spawn_host("bunting-fix-acceptor", completed, move || {
            crate::acceptor::run(
                &fix,
                storage_kind,
                storage_path.as_deref(),
                &origin,
                &admission,
            )
        })?;
        task_count = task_count.saturating_add(1);
    }
    drop(completed);
    if task_count == 0 {
        return Err("native profile requires at least one FIX or admin listener".to_owned());
    }
    listener
        .recv()
        .map_err(|_| "server listener task panicked".to_owned())?
}

fn spawn_host(
    name: &str,
    completed: mpsc::Sender<Result<(), String>>,
    host: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _ = completed.send(host());
        })
        .map(|_| ())
        .map_err(|error| format!("cannot spawn {name}: {error}"))
}

fn origin_error(error: &OriginError) -> String {
    format!("origin store error: {error}")
}

/// Installs the configured immutable scenario's run, or checks that the
/// restored run was built from the same scenario.
fn bootstrap_run(config: &ServerConfig, origin: &NativeOrigin) -> Result<(), String> {
    if let Some((run_id, iteration_id, definition)) = crate::scenario::bootstrap(config)? {
        definition
            .validate()
            .map_err(|error| format!("scenario validation failed: {error:?}"))?;
        let run = RunState::from_scenario(
            RunId::new(run_id),
            IterationId::new(iteration_id),
            &definition,
        )
        .map_err(|error| format!("cannot create run from scenario: {error}"))?;
        if let Some(runtime) = &config.runtime {
            if runtime.scheduler.run_id != run.run_id()
                || run
                    .listing_key_for_instrument(runtime.scheduler.instrument_id)
                    .is_err()
                || runtime.scheduler.agents.iter().any(|agent| {
                    !definition
                        .participants()
                        .contains_key(&agent.participant_id)
                })
            {
                return Err(
                    "runtime run, instrument and agent participants must exist in the immutable scenario"
                        .to_owned(),
                );
            }
        }
        match origin.read_run(run.run_id(), |existing| {
            existing.scenario_hash() == run.scenario_hash()
        }) {
            Ok(true) => {}
            Ok(false) => {
                return Err(
                    "configured immutable scenario does not match the restored run hash".to_owned(),
                );
            }
            Err(OriginError::UnknownRun) => {
                origin
                    .insert_run(run)
                    .map_err(|error| origin_error(&error))?;
            }
            Err(error) => return Err(origin_error(&error)),
        }
    }
    Ok(())
}
