use bunting_application::{ApplicationError, ApplicationService, VerifiedActor};
use bunting_command_transaction::InMemorySnapshotCache;
use bunting_engine::RunState;
use bunting_market_events::{Command, SimulationCommandRequest};
use bunting_market_types::RunId;
use bunting_origin_store::{InMemoryOrigin, OriginError};

/// Concrete host-neutral embedding façade used by every language binding.
#[derive(Debug)]
pub struct BuntingHandle {
    origin: InMemoryOrigin,
    cache: InMemorySnapshotCache,
}

impl BuntingHandle {
    pub fn new(initial: RunState) -> Result<Self, OriginError> {
        let origin = InMemoryOrigin::new();
        origin.insert_run(initial)?;
        Ok(Self {
            origin,
            cache: InMemorySnapshotCache::new(),
        })
    }

    pub fn recover(&self, run_id: RunId) -> Result<RunState, ApplicationError> {
        ApplicationService::new(&self.origin, &self.cache).recover(run_id)
    }

    pub fn execute(
        &self,
        actor: &VerifiedActor,
        command: &Command,
    ) -> Result<RunState, ApplicationError> {
        ApplicationService::new(&self.origin, &self.cache)
            .execute(actor, command)
            .map(|executed| executed.state)
    }

    pub fn execute_simulation(
        &self,
        actor: &VerifiedActor,
        command: &SimulationCommandRequest,
    ) -> Result<RunState, ApplicationError> {
        ApplicationService::new(&self.origin, &self.cache)
            .execute_simulation(actor, command)
            .map(|executed| executed.state)
    }

    pub fn replay_archive_json(json: &str) -> Result<String, String> {
        let archive =
            crate::CompetitionArchive::from_json(json).map_err(|error| error.to_string())?;
        let result = archive.replay().map_err(|error| error.to_string())?;
        serde_json::to_string(&result).map_err(|error| error.to_string())
    }
}
