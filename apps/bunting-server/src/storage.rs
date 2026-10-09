use crate::commit_journal::{self, EntryRef, JournalEntry, JournalWriter};
use crate::config::{StorageConfig, StorageKind};
use bunting_engine::{EngineSnapshotEnvelope, RunState};
use bunting_market_events::EventEnvelope;
use bunting_market_types::{CommandId, RunId};
use bunting_origin_store::{
    CommandResult, Executed, Execution, InMemoryOrigin, JournalInput, LiveRun, OriginError,
    OriginStore, RunLimits, RunRecovery,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

const CHECKPOINT_VERSION: u16 = 2;

/// State-only restart accelerator. The journal stays authoritative: every
/// checkpointed run must reappear, with the same chain value, in a replay of
/// its journal records.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u16,
    runs: Vec<CheckpointRun>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CheckpointRun {
    snapshot: EngineSnapshotEnvelope,
    chain: String,
}

#[derive(Debug)]
struct FileInner {
    runs: BTreeMap<RunId, LiveRun>,
    journal: JournalWriter,
}

/// Durable single-writer origin: the live runs are in memory, every
/// committed input is appended to the journal before it is acknowledged.
#[derive(Clone, Debug)]
pub struct FileOriginStore {
    path: PathBuf,
    inner: Arc<Mutex<FileInner>>,
    /// An ambiguous journal write prevents further acknowledgments until a
    /// process restart re-verifies the journal.
    poisoned: Arc<AtomicBool>,
    /// OS-backed single-writer lease, held until the last store clone drops.
    #[cfg(unix)]
    _writer_lock: Arc<File>,
    max_runs: usize,
    limits: RunLimits,
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
                .truncate(false)
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

        let limits = config.limits();
        let journal_path = commit_journal::path_for(&path);
        if path.exists() && !journal_path.exists() {
            // A checkpoint is only an accelerator for a journal it came from.
            return Err(OriginError::InvalidCommit);
        }
        let runs = recover(&path, &journal_path, limits, config.max_runs)?;
        let store = Self {
            path,
            inner: Arc::new(Mutex::new(FileInner {
                runs,
                journal: JournalWriter::open(&journal_path)?,
            })),
            poisoned: Arc::new(AtomicBool::new(false)),
            #[cfg(unix)]
            _writer_lock: writer_lock,
            max_runs: config.max_runs,
            limits,
        };
        // Bound the next restart's re-execution if the recovered tail is long.
        let mut inner = store.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if inner.runs.values().any(LiveRun::needs_checkpoint) {
            for live in inner.runs.values_mut() {
                live.checkpoint()?;
            }
            let _ = write_checkpoint(&store.path, &inner.runs);
        }
        drop(inner);
        Ok(store)
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut inner = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        if let Some(existing) = inner.runs.get(&run.run_id()) {
            return if existing.state()? == &run {
                Ok(())
            } else {
                Err(OriginError::InvalidCommit)
            };
        }
        if inner.runs.len() >= self.max_runs {
            return Err(OriginError::CapacityExceeded);
        }
        let snapshot =
            EngineSnapshotEnvelope::new(run.clone()).map_err(|_| OriginError::InvalidCommit)?;
        let live = LiveRun::genesis(run, self.limits)?;
        if let Err(error) = inner.journal.append(&EntryRef::Genesis {
            snapshot: &snapshot,
        }) {
            self.poisoned.store(true, Ordering::Release);
            return Err(error);
        }
        inner.runs.insert(live.state()?.run_id(), live);
        Ok(())
    }

    /// Force a durable, atomic checkpoint of every run's live state.
    pub fn checkpoint(&self) -> Result<(), OriginError> {
        let mut inner = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        for live in inner.runs.values_mut() {
            live.checkpoint()?;
        }
        write_checkpoint(&self.path, &inner.runs)
    }

    /// Every committed event of one run, read back from the journal.
    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        let inner = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        if !inner.runs.contains_key(&run_id) {
            return Err(OriginError::UnknownRun);
        }
        let mut events = Vec::new();
        commit_journal::scan(&commit_journal::path_for(&self.path), false, |entry| {
            if let JournalEntry::Command(record) = entry
                && record.input.run_id() == run_id
            {
                events.extend(record.events);
            }
            Ok(())
        })?;
        Ok(events)
    }
}

impl OriginStore for FileOriginStore {
    fn execute(&self, input: &JournalInput) -> Result<Executed, OriginError> {
        let mut guard = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        let FileInner { runs, journal } = &mut *guard;
        let live = runs
            .get_mut(&input.run_id())
            .ok_or(OriginError::UnknownRun)?;
        let record = match live.execute(input)? {
            Execution::Duplicate(result) => {
                return Ok(Executed {
                    result,
                    events: Vec::new(),
                    duplicate: true,
                });
            }
            Execution::Committed(record) => record,
        };
        // The live run already holds the transition; it becomes committed
        // only when the journal frame is durable. A failed append might
        // still have reached disk, so only a restart can resolve it.
        if let Err(error) = journal.append(&EntryRef::Command(&record)) {
            self.poisoned.store(true, Ordering::Release);
            return Err(error);
        }
        if live.needs_checkpoint() {
            live.checkpoint()?;
            // Best effort: the journal is authoritative, so a failed
            // checkpoint only lengthens the next restart's re-execution.
            let _ = write_checkpoint(&self.path, runs);
        }
        let record = *record;
        Ok(Executed {
            result: record.result,
            events: record.events,
            duplicate: false,
        })
    }

    fn read_run<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, OriginError> {
        let inner = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        Ok(read(
            inner
                .runs
                .get(&run_id)
                .ok_or(OriginError::UnknownRun)?
                .state()?,
        ))
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        let inner = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(OriginError::Unavailable);
        }
        Ok(inner
            .runs
            .get(&run_id)
            .ok_or(OriginError::UnknownRun)?
            .find(command_id))
    }
}

/// Rebuilds every run from the journal, starting each from its checkpoint
/// when one exists. Records after a checkpoint are re-executed and must
/// reproduce the journal exactly, so every restart is a determinism check.
fn recover(
    checkpoint_path: &Path,
    journal_path: &Path,
    limits: RunLimits,
    max_runs: usize,
) -> Result<BTreeMap<RunId, LiveRun>, OriginError> {
    let mut checkpointed = BTreeMap::new();
    if checkpoint_path.exists() {
        let bytes = fs::read(checkpoint_path).map_err(|_| OriginError::Unavailable)?;
        // Stores from before journal format 2 are rejected, not migrated.
        let checkpoint: Checkpoint =
            serde_json::from_slice(&bytes).map_err(|_| OriginError::InvalidCommit)?;
        if checkpoint.version != CHECKPOINT_VERSION {
            return Err(OriginError::InvalidCommit);
        }
        for run in checkpoint.runs {
            run.snapshot
                .verify()
                .map_err(|_| OriginError::InvalidCommit)?;
            let run_id = run.snapshot.state.run_id();
            if checkpointed
                .insert(run_id, (run.snapshot.state, run.chain))
                .is_some()
            {
                return Err(OriginError::InvalidCommit);
            }
        }
    }
    let mut recoveries = BTreeMap::<RunId, RunRecovery>::new();
    commit_journal::scan(journal_path, true, |entry| match entry {
        JournalEntry::Genesis { snapshot } => {
            snapshot.verify().map_err(|_| OriginError::InvalidCommit)?;
            let genesis = snapshot.state;
            let run_id = genesis.run_id();
            if recoveries.contains_key(&run_id) || recoveries.len() >= max_runs {
                return Err(OriginError::InvalidCommit);
            }
            let checkpoint = checkpointed.remove(&run_id);
            let recovery = RunRecovery::new(
                genesis,
                checkpoint
                    .as_ref()
                    .map(|(state, chain)| (state.clone(), chain.as_str())),
                limits,
            )?;
            recoveries.insert(run_id, recovery);
            Ok(())
        }
        JournalEntry::Command(record) => recoveries
            .get_mut(&record.input.run_id())
            .ok_or(OriginError::InvalidCommit)?
            .replay(&record),
    })?;
    if !checkpointed.is_empty() {
        // A checkpointed run has no genesis in the journal.
        return Err(OriginError::InvalidCommit);
    }
    recoveries
        .into_iter()
        .map(|(run_id, recovery)| Ok((run_id, recovery.finish()?)))
        .collect()
}

fn write_checkpoint(path: &Path, runs: &BTreeMap<RunId, LiveRun>) -> Result<(), OriginError> {
    let runs = runs
        .values()
        .map(|live| {
            let (state, chain) = live.checkpoint_state();
            Ok(CheckpointRun {
                snapshot: EngineSnapshotEnvelope::new(state.clone())
                    .map_err(|_| OriginError::InvalidCommit)?,
                chain,
            })
        })
        .collect::<Result<Vec<_>, OriginError>>()?;
    persist(
        path,
        &Checkpoint {
            version: CHECKPOINT_VERSION,
            runs,
        },
    )
}

fn persist(path: &Path, checkpoint: &Checkpoint) -> Result<(), OriginError> {
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|_| OriginError::Unavailable)?;
    }
    let bytes = serde_json::to_vec(checkpoint).map_err(|_| OriginError::InvalidCommit)?;
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
    // the directory entry that names the atomically replaced checkpoint.
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
            StorageKind::Memory => Ok(Self::Memory(InMemoryOrigin::with_limits(config.limits()))),
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
}

impl OriginStore for NativeOrigin {
    fn execute(&self, input: &JournalInput) -> Result<Executed, OriginError> {
        match self {
            Self::Memory(store) => store.execute(input),
            Self::File(store) => store.execute(input),
        }
    }

    fn read_run<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, OriginError> {
        match self {
            Self::Memory(store) => store.read_run(run_id, read),
            Self::File(store) => store.read_run(run_id, read),
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
}
