use crate::config::{StorageConfig, StorageKind};
use bunting_engine::RunState;
use bunting_market_events::EventEnvelope;
use bunting_market_types::{CommandId, RunId};
use bunting_origin_store::{
    CommandResult, CommitOutcome, CommitRequest, InMemoryOrigin, OriginError, OriginStore,
};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct StoredCommand {
    run_id: RunId,
    command_id: CommandId,
    fingerprint: String,
    result: CommandResult,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct RunEvents {
    run_id: RunId,
    events: Vec<EventEnvelope>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileState {
    runs: Vec<RunState>,
    commands: Vec<StoredCommand>,
    events: Vec<RunEvents>,
}

#[derive(Clone, Debug)]
pub struct FileOriginStore {
    path: PathBuf,
    state: Arc<Mutex<FileState>>,
    max_runs: usize,
    max_commands: usize,
    max_events_per_run: usize,
}

impl FileOriginStore {
    pub fn open(path: impl Into<PathBuf>, config: &StorageConfig) -> Result<Self, OriginError> {
        let path = path.into();
        let state = if path.exists() {
            let bytes = fs::read(&path).map_err(|_| OriginError::Unavailable)?;
            serde_json::from_slice(&bytes).map_err(|_| OriginError::Unavailable)?
        } else {
            FileState::default()
        };
        let store = Self {
            path,
            state: Arc::new(Mutex::new(state)),
            max_runs: config.max_runs,
            max_commands: config.max_commands,
            max_events_per_run: config.max_events_per_run,
        };
        store.validate_loaded()?;
        Ok(store)
    }

    fn validate_loaded(&self) -> Result<(), OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if state.runs.len() > self.max_runs || state.commands.len() > self.max_commands {
            return Err(OriginError::Unavailable);
        }
        if state
            .events
            .iter()
            .any(|events| events.events.len() > self.max_events_per_run)
        {
            return Err(OriginError::Unavailable);
        }
        Ok(())
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if let Some(existing) = state.runs.iter().find(|item| item.run_id() == run.run_id()) {
            return if existing == &run {
                Ok(())
            } else {
                Err(OriginError::InvalidCommit)
            };
        }
        if state.runs.len() >= self.max_runs {
            return Err(OriginError::Unavailable);
        }
        let mut candidate = state.clone();
        candidate.runs.push(run);
        persist(&self.path, &candidate)?;
        *state = candidate;
        Ok(())
    }

    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(state
            .events
            .iter()
            .find(|item| item.run_id == run_id)
            .map(|item| item.events.clone())
            .unwrap_or_default())
    }
}

impl OriginStore for FileOriginStore {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        state
            .runs
            .iter()
            .find(|run| run.run_id() == run_id)
            .cloned()
            .ok_or(OriginError::UnknownRun)
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(state
            .commands
            .iter()
            .find(|item| item.run_id == run_id && item.command_id == command_id)
            .map(|item| (item.fingerprint.clone(), item.result.clone())))
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        let mut state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if let Some(existing) = state
            .commands
            .iter()
            .find(|item| item.run_id == request.run_id && item.command_id == request.command_id)
        {
            return if existing.fingerprint == request.fingerprint {
                Ok(CommitOutcome::Duplicate(existing.result.clone()))
            } else {
                Err(OriginError::IdempotencyConflict)
            };
        }
        let run_index = state
            .runs
            .iter()
            .position(|run| run.run_id() == request.run_id)
            .ok_or(OriginError::UnknownRun)?;
        let current = state.runs[run_index].sequence();
        if current != request.expected_version {
            return Err(OriginError::VersionConflict { current });
        }
        let next = request
            .expected_version
            .checked_add(bunting_market_types::EventSequence::new(1))
            .ok_or(OriginError::InvalidCommit)?;
        if request.candidate.run_id() != request.run_id
            || request.candidate.sequence() != next
            || request.result.committed_sequence != next
            || request
                .events
                .last()
                .is_some_and(|event| event.sequence != request.candidate.event_sequence())
        {
            return Err(OriginError::InvalidCommit);
        }
        if state.commands.len() >= self.max_commands {
            return Err(OriginError::Unavailable);
        }
        let event_index = state
            .events
            .iter()
            .position(|item| item.run_id == request.run_id);
        let existing_events = event_index.map_or(0, |index| state.events[index].events.len());
        if existing_events.saturating_add(request.events.len()) > self.max_events_per_run {
            return Err(OriginError::Unavailable);
        }
        let mut candidate = state.clone();
        candidate.runs[run_index] = request.candidate;
        candidate.commands.push(StoredCommand {
            run_id: request.run_id,
            command_id: request.command_id,
            fingerprint: request.fingerprint,
            result: request.result.clone(),
        });
        if let Some(index) = event_index {
            candidate.events[index].events.extend(request.events);
        } else {
            candidate.events.push(RunEvents {
                run_id: request.run_id,
                events: request.events,
            });
        }
        persist(&self.path, &candidate)?;
        *state = candidate;
        Ok(CommitOutcome::Committed(request.result))
    }
}

fn persist(path: &Path, state: &FileState) -> Result<(), OriginError> {
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|_| OriginError::Unavailable)?;
    }
    let bytes = serde_json::to_vec(state).map_err(|_| OriginError::InvalidCommit)?;
    let temporary = path.with_extension("tmp");
    let mut file = File::create(&temporary).map_err(|_| OriginError::Unavailable)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| OriginError::Unavailable)?;
    fs::rename(&temporary, path).map_err(|_| OriginError::Unavailable)
}

#[derive(Clone, Debug)]
pub enum NativeOrigin {
    Memory(InMemoryOrigin),
    File(FileOriginStore),
}

impl NativeOrigin {
    pub fn from_config(config: &StorageConfig) -> Result<Self, OriginError> {
        match config.kind {
            StorageKind::Memory => Ok(Self::Memory(InMemoryOrigin::new())),
            StorageKind::File => Ok(Self::File(FileOriginStore::open(
                config.path.as_deref().ok_or(OriginError::Unavailable)?,
                config,
            )?)),
        }
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        match self {
            Self::Memory(store) => store.insert_run(run),
            Self::File(store) => store.insert_run(run),
        }
    }

    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        match self {
            Self::Memory(store) => store.events(run_id),
            Self::File(store) => store.events(run_id),
        }
    }
}

impl OriginStore for NativeOrigin {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        match self {
            Self::Memory(store) => store.load_run(run_id),
            Self::File(store) => store.load_run(run_id),
        }
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        match self {
            Self::Memory(store) => store.find_command(run_id, command_id),
            Self::File(store) => store.find_command(run_id, command_id),
        }
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        match self {
            Self::Memory(store) => store.commit(request),
            Self::File(store) => store.commit(request),
        }
    }
}
