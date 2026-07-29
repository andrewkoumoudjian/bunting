use crate::config::ServerConfig;
use crate::storage::NativeOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_command_transaction::InMemorySnapshotCache;
use bunting_engine::RunState;
use bunting_market_types::{IterationId, RunId};
use bunting_origin_store::{OriginError, OriginStore};
use std::sync::Arc;
use std::time::Duration;

pub async fn run(config: &ServerConfig) -> Result<(), String> {
    config.validate().map_err(|error| error.to_string())?;
    let origin =
        NativeOrigin::from_config(&config.storage).map_err(|error| origin_error(&error))?;
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
        match origin.load_run(run.run_id()) {
            Ok(existing) if existing.scenario_hash() == run.scenario_hash() => {}
            Ok(_) => {
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
    let origin = Arc::new(origin);
    let cache = Arc::new(InMemorySnapshotCache::new());
    let (matching_interval_ms, max_interval_queue) =
        config.fix.as_ref().map_or((1, 1_024), |fix| {
            (fix.matching_interval_ms, fix.max_interval_queue)
        });
    let writer = Arc::new(AuthoritativeWriter::new(
        Duration::from_millis(matching_interval_ms),
        max_interval_queue,
    ));
    let mut tasks = Vec::new();
    if let Some(admin) = config.admin.clone() {
        let origin = origin.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            crate::admin::run(&admin, &origin)
        }));
    }
    if let Some(runtime) = config.runtime.clone() {
        let origin = origin.clone();
        let cache = cache.clone();
        let writer = writer.clone();
        tasks.push(tokio::task::spawn_blocking(move || {
            crate::scenario::run(&runtime, &origin, &cache, &writer)
        }));
    }
    if let Some(fix) = config.fix.clone() {
        let origin = origin.clone();
        let cache = cache.clone();
        let writer = writer.clone();
        let storage_kind = config.storage.kind;
        let storage_path = config.storage.path.clone();
        tasks.push(tokio::spawn(async move {
            crate::acceptor::run(fix, storage_kind, storage_path, origin, cache, writer).await
        }));
    }
    if tasks.is_empty() {
        return Err("native profile requires at least one FIX or admin listener".to_owned());
    }
    for task in tasks {
        task.await
            .map_err(|_| "server listener task panicked".to_owned())??;
    }
    Ok(())
}

fn origin_error(error: &OriginError) -> String {
    format!("origin store error: {error}")
}
