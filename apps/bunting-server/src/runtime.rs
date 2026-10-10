use crate::admission::{AdmissionService, VenueClock};
use crate::config::AdmissionConfig;
use crate::config::ServerConfig;
use crate::consolidated::ConsolidatedTape;
use crate::distributor::{MAX_PENDING_BATCHES, PublishingOrigin};
use crate::storage::NativeOrigin;
use bunting_admission_sequencer::Endpoint;
use bunting_engine::RunState;
use bunting_engine::ScenarioDefinition;
use bunting_market_types::{IterationId, RunId};
use bunting_origin_store::{OriginError, OriginStore};
use std::collections::BTreeSet;
use std::sync::{Arc, mpsc};

/// Public changes on their way to the consolidated tape's processor.
const MAX_TAPE_IN_FLIGHT: usize = 65_536;

pub fn run(config: &ServerConfig) -> Result<(), String> {
    config.validate().map_err(|error| error.to_string())?;
    let origin =
        NativeOrigin::from_config(&config.storage).map_err(|error| origin_error(&error))?;
    bootstrap_run(config, &origin)?;
    let clock = VenueClock::start();
    // Every command, from FIX sessions and built-in agents alike, is
    // admitted by one sequencer: the only thread that commits.
    let admission = Arc::new(AdmissionService::new(
        clock,
        config
            .fix
            .as_ref()
            .map_or_else(AdmissionConfig::colocated, |fix| fix.admission.clone()),
    )?);
    // The consolidated tape's processor sits at the hub (ADR 0036).
    let tape = {
        let admission = admission.clone();
        ConsolidatedTape::new(
            admission.config().consolidated_processing_us,
            MAX_TAPE_IN_FLIGHT,
            Box::new(move |venue| admission.delay_us(Endpoint::Venue(venue), Endpoint::Hub)),
        )
    };
    let origin = Arc::new(PublishingOrigin::new(
        origin,
        MAX_PENDING_BATCHES,
        clock,
        tape,
    ));
    let (completed, listener) = mpsc::channel();
    let mut task_count = 0_usize;
    if let Some(admin) = config.admin.clone() {
        let origin = origin.clone();
        let admission = admission.clone();
        let completed = completed.clone();
        spawn_host("bunting-admin", completed, move || {
            crate::admin::run(&admin, origin.inner(), Some(&admission))
        })?;
        task_count = task_count.saturating_add(1);
    }
    {
        let admission = admission.clone();
        let origin = origin.clone();
        let completed = completed.clone();
        spawn_host("bunting-sequencer", completed, move || {
            admission.run_sequencer(&origin)
        })?;
    }
    {
        let origin = origin.clone();
        let completed = completed.clone();
        spawn_host("bunting-tape", completed, move || {
            origin.tape().run(|| clock.now_us())
        })?;
    }
    if let Some(runtime) = config.runtime.clone() {
        let origin = origin.clone();
        let admission = admission.clone();
        let completed = completed.clone();
        let checkpoint = crate::scenario::checkpoint_path(&config.storage);
        spawn_host("bunting-scenario", completed, move || {
            crate::scenario::run(&runtime, &origin, &admission, checkpoint)
        })?;
        task_count = task_count.saturating_add(1);
    }
    if let Some(fix) = config.fix.clone() {
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
        if let Some(fix) = &config.fix {
            validate_latency_table(&definition, &fix.admission)?;
        }
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

/// The latency map (ADR 0035) may only place participants and venues the
/// scenario lists. An empty map is valid: everyone at one location, so only
/// real network delay separates teams.
fn validate_latency_table(
    definition: &ScenarioDefinition,
    admission: &AdmissionConfig,
) -> Result<(), String> {
    let venues = definition
        .listings()
        .keys()
        .map(|listing| listing.venue_id)
        .collect::<BTreeSet<_>>();
    if let Some(placement) = admission.map.participants.iter().find(|placement| {
        !definition
            .participants()
            .contains_key(&placement.participant_id)
    }) {
        return Err(format!(
            "fix.admission.map places participant {} that the scenario does not list",
            placement.participant_id
        ));
    }
    if let Some(placement) = admission
        .map
        .venues
        .iter()
        .find(|placement| !venues.contains(&placement.venue_id))
    {
        return Err(format!(
            "fix.admission.map places venue {} that the scenario does not list",
            placement.venue_id
        ));
    }
    Ok(())
}
