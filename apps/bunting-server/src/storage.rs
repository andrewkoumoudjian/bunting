use crate::commit_journal;
use crate::config::{StorageConfig, StorageKind};
use bunting_engine::{EngineSnapshotEnvelope, RunState};
use bunting_market_events::EventEnvelope;
use bunting_market_types::{CommandId, EventSequence, RunId};
use bunting_origin_store::{
    CommandResult, CommitOutcome, CommitRequest, InMemoryOrigin, OriginError, OriginStore,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

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
    /// An ambiguous write failure prevents further acknowledgments until a
    /// process restart and journal-prefix recovery.
    poisoned: Arc<AtomicBool>,
    /// OS-backed single-writer lease, held until the last store clone drops.
    #[cfg(unix)]
    _writer_lock: Arc<File>,
    max_runs: usize,
    max_commands: usize,
    max_events_per_run: usize,
}

impl FileOriginStore {
    pub fn open(path: impl Into<PathBuf>, config: &StorageConfig) -> Result<Self, OriginError> {
        let path = path.into();
        // Native durable mode must never admit two independent processes
        // reading the same expected version and both appending accepted writes.
        // Unix flock follows the lifetime of the open file descriptor, rather
        // than leaving stale PID files behind after an unclean process exit.
        #[cfg(unix)]
        let writer_lock = {
            use rustix::fs::{FlockOperation, flock};
            let lock_path = path.with_extension("lock");
            if let Some(parent) = lock_path
                .parent()
                .filter(|item| !item.as_os_str().is_empty())
            {
                fs::create_dir_all(parent).map_err(|_| OriginError::Unavailable)?;
            }
            let file = fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(lock_path)
                .map_err(|_| OriginError::Unavailable)?;
            flock(&file, FlockOperation::NonBlockingLockExclusive)
                .map_err(|_| OriginError::Unavailable)?;
            Arc::new(file)
        };
        // A known atomic locking primitive is required for durable mode.
        #[cfg(not(unix))]
        return Err(OriginError::Unavailable);

        let journal_path = commit_journal::path_for(&path);
        if !path.exists() && journal_path.exists() {
            // A journal without its genesis checkpoint is not a valid run.
            return Err(OriginError::Unavailable);
        }
        let mut state = if path.exists() {
            let bytes = fs::read(&path).map_err(|_| OriginError::Unavailable)?;
            serde_json::from_slice(&bytes).map_err(|_| OriginError::Unavailable)?
        } else {
            FileState::default()
        };
        commit_journal::replay(&journal_path, |request| {
            if check_record(
                &state,
                &request,
                config.max_commands,
                config.max_events_per_run,
            )?
            .is_none()
            {
                apply_record(&mut state, request)?;
            }
            Ok(())
        })?;
        let store = Self {
            path,
            state: Arc::new(Mutex::new(state)),
            poisoned: Arc::new(AtomicBool::new(false)),
            #[cfg(unix)]
            _writer_lock: writer_lock,
            max_runs: config.max_runs,
            max_commands: config.max_commands,
            max_events_per_run: config.max_events_per_run,
        };
        store.validate_loaded()?;
        Ok(store)
    }

    /// Fail closed on any incomplete, duplicated or inconsistent committed
    /// state, rather than starting an exchange from a plausible JSON file.
    fn validate_loaded(&self) -> Result<(), OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if state.runs.len() > self.max_runs || state.commands.len() > self.max_commands {
            return Err(OriginError::Unavailable);
        }
        let mut runs = BTreeMap::new();
        for run in &state.runs {
            if runs.insert(run.run_id(), run).is_some() {
                return Err(OriginError::InvalidCommit);
            }
            // Validate the exact engine schema and canonical snapshot hash.
            // Do not attempt recovery from obsolete or structurally corrupt runs.
            let envelope = run
                .snapshot_envelope()
                .and_then(|envelope| envelope.to_json())
                .map_err(|_| OriginError::InvalidCommit)?;
            EngineSnapshotEnvelope::from_json(&envelope).map_err(|_| OriginError::InvalidCommit)?;
        }
        let mut event_heads = BTreeMap::new();
        let mut event_commands = BTreeSet::new();
        for batch in &state.events {
            if batch.events.len() > self.max_events_per_run
                || !runs.contains_key(&batch.run_id)
                || event_heads.contains_key(&batch.run_id)
            {
                return Err(OriginError::InvalidCommit);
            }
            let mut current = EventSequence::new(0);
            for event in &batch.events {
                let next = current
                    .checked_add(EventSequence::new(1))
                    .ok_or(OriginError::InvalidCommit)?;
                if event.run_id != batch.run_id || event.sequence != next {
                    return Err(OriginError::InvalidCommit);
                }
                event_commands.insert((batch.run_id, event.command_id));
                current = next;
            }
            event_heads.insert(batch.run_id, current);
        }
        if runs.iter().any(|(run_id, run)| {
            event_heads
                .get(run_id)
                .copied()
                .unwrap_or(EventSequence::new(0))
                != run.event_sequence()
        }) {
            return Err(OriginError::InvalidCommit);
        }
        let mut command_ids = BTreeSet::new();
        for command in &state.commands {
            let Some(run) = runs.get(&command.run_id) else {
                return Err(OriginError::InvalidCommit);
            };
            if !command_ids.insert((command.run_id, command.command_id))
                || !event_commands.contains(&(command.run_id, command.command_id))
                || command.fingerprint.is_empty()
                || command.result.committed_sequence.get() == 0
                || command.result.committed_sequence > run.sequence()
            {
                return Err(OriginError::InvalidCommit);
            }
        }
        Ok(())
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
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
        // Checkpoint supersedes each previously committed journal entry.
        // Leaving redundant entries on an I/O failure is safe on recovery.
        let _ = commit_journal::clear(&commit_journal::path_for(&self.path));
        Ok(())
    }

    /// Force a durable, atomic checkpoint and compact the synced journal.
    /// Failure to compact does not undo the checkpoint or committed commands.
    pub fn checkpoint(&self) -> Result<(), OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        persist(&self.path, &state)?;
        let _ = commit_journal::clear(&commit_journal::path_for(&self.path));
        Ok(())
    }

    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        let state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
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
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
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
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        Ok(state
            .commands
            .iter()
            .find(|item| item.run_id == run_id && item.command_id == command_id)
            .map(|item| (item.fingerprint.clone(), item.result.clone())))
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        let mut state = self.state.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        if let Some(previous) = check_record(
            &state,
            &request,
            self.max_commands,
            self.max_events_per_run,
        )? {
            return Ok(CommitOutcome::Duplicate(previous));
        }
        // The journal is durable BEFORE the in-memory run and responses change.
        // A failed append poisons the writer: its last frame might be complete,
        // and only replay can resolve that ambiguity safely.
        if let Err(error) = commit_journal::append(
            &commit_journal::path_for(&self.path),
            &request,
        ) {
            self.poisoned.store(true, Ordering::Release);
            return Err(error);
        }
        let result = request.result.clone();
        if let Err(error) = apply_record(&mut state, request) {
            self.poisoned.store(true, Ordering::Release);
            return Err(error);
        }
        // Periodically compact history to one atomically replaced checkpoint.
        // Compaction is best effort after the journal commit and never causes a
        // durable acknowledged order to be reported as rejected.
        if state.commands.len() % commit_journal::CHECKPOINT_INTERVAL == 0
            && persist(&self.path, &state).is_ok()
        {
            let _ = commit_journal::clear(&commit_journal::path_for(&self.path));
        }
        Ok(CommitOutcome::Committed(result))
    }
}

/// Reject idempotency collisions, version races, and invalid event cursors
/// before any durable write. Reused without change during journal recovery.
fn check_record(
    state: &FileState,
    request: &CommitRequest,
    max_commands: usize,
    max_events_per_run: usize,
) -> Result<Option<CommandResult>, OriginError> {
    if let Some(previous) = state
        .commands
        .iter()
        .find(|item| item.run_id == request.run_id && item.command_id == request.command_id)
    {
        return if previous.fingerprint == request.fingerprint
            && previous.result == request.result
            && state.runs.iter().any(|run| {
                run.run_id() == request.run_id
                    && run.sequence() >= request.result.committed_sequence
            })
        {
            Ok(Some(previous.result.clone()))
        } else {
            Err(OriginError::IdempotencyConflict)
        };
    }
    let run = state
        .runs
        .iter()
        .find(|run| run.run_id() == request.run_id)
        .ok_or(OriginError::UnknownRun)?;
    if run.sequence() != request.expected_version {
        return Err(OriginError::VersionConflict {
            current: run.sequence(),
        });
    }
    request.validate_against(run.event_sequence())?;
    if state.commands.len() >= max_commands {
        return Err(OriginError::Unavailable);
    }
    let existing_events = state
        .events
        .iter()
        .find(|batch| batch.run_id == request.run_id)
        .map_or(0, |batch| batch.events.len());
    if existing_events.saturating_add(request.events.len()) > max_events_per_run {
        return Err(OriginError::Unavailable);
    }
    Ok(None)
}

fn apply_record(state: &mut FileState, request: CommitRequest) -> Result<(), OriginError> {
    let run = state
        .runs
        .iter_mut()
        .find(|run| run.run_id() == request.run_id)
        .ok_or(OriginError::UnknownRun)?;
    *run = request.candidate;
    state.commands.push(StoredCommand {
        run_id: request.run_id,
        command_id: request.command_id,
        fingerprint: request.fingerprint,
        result: request.result,
    });
    if let Some(batch) = state
        .events
        .iter_mut()
        .find(|batch| batch.run_id == request.run_id)
    {
        batch.events.extend(request.events);
    } else {
        state.events.push(RunEvents {
            run_id: request.run_id,
            events: request.events,
        });
    }
    Ok(())
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
    // Closing the file before rename is required on some supported hosts.
    drop(file);
    fs::rename(&temporary, path).map_err(|_| OriginError::Unavailable)?;
    // A synced temporary file alone does not make the rename durable after a
    // sudden power loss. On POSIX, syncing the containing directory persists
    // the directory entry that names the atomically replaced snapshot.
    #[cfg(unix)]
    {
        let directory = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| OriginError::Unavailable)?;
    }
    Ok(())
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
