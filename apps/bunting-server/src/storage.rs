use crate::config::{StorageConfig, StorageKind};
use bunting_engine::{EngineSnapshotEnvelope, RunState};
use bunting_market_events::EventEnvelope;
use bunting_market_types::{CommandId, EventSequence, RunId};
use bunting_origin_store::{
    CommandResult, CommitOutcome, CommitRequest, InMemoryOrigin, OriginError, OriginStore,
};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TURSO_SCHEMA_VERSION: &str = "1";

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
        validate_commit(&request, current)?;
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

#[derive(Clone)]
pub struct TursoOriginStore {
    inner: Arc<TursoInner>,
}

struct TursoInner {
    runtime: tokio::runtime::Runtime,
    connection: Mutex<turso::Connection>,
    max_runs: usize,
    max_commands: usize,
    max_events_per_run: usize,
}

impl fmt::Debug for TursoOriginStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TursoOriginStore")
            .field("max_runs", &self.inner.max_runs)
            .field("max_commands", &self.inner.max_commands)
            .field("max_events_per_run", &self.inner.max_events_per_run)
            .finish_non_exhaustive()
    }
}

impl TursoOriginStore {
    pub fn open(path: impl AsRef<Path>, config: &StorageConfig) -> Result<Self, OriginError> {
        let path = path.as_ref();
        let existed = path.exists();
        if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|_| OriginError::Unavailable)?;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| OriginError::Unavailable)?;
        let database_path = path.to_string_lossy().into_owned();
        let connection = runtime.block_on(async {
            let database = turso::Builder::new_local(database_path)
                .build()
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let connection = database.connect().map_err(|_| OriginError::Unavailable)?;
            if existed {
                validate_existing_turso(&connection).await?;
            } else {
                initialize_turso(&connection).await?;
            }
            Ok::<_, OriginError>(connection)
        })?;
        Ok(Self {
            inner: Arc::new(TursoInner {
                runtime,
                connection: Mutex::new(connection),
                max_runs: config.max_runs,
                max_commands: config.max_commands,
                max_events_per_run: config.max_events_per_run,
            }),
        })
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut connection = self
            .inner
            .connection
            .lock()
            .map_err(|_| OriginError::Unavailable)?;
        let max_runs = self.inner.max_runs;
        self.inner.runtime.block_on(async {
            let transaction = connection
                .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let run_id = run.run_id().get().to_string();
            let mut rows = transaction
                .query("SELECT snapshot_json FROM bunting_runs WHERE run_id = ?1", [run_id.clone()])
                .await
                .map_err(|_| OriginError::Unavailable)?;
            if let Some(row) = rows.next().await.map_err(|_| OriginError::Unavailable)? {
                let encoded: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
                let existing = decode_run(&encoded)?;
                return if existing == run {
                    transaction.rollback().await.map_err(|_| OriginError::Unavailable)?;
                    Ok(())
                } else {
                    Err(OriginError::InvalidCommit)
                };
            }
            let count = count_rows(&transaction, "SELECT COUNT(*) FROM bunting_runs").await?;
            if count >= max_runs {
                return Err(OriginError::Unavailable);
            }
            let snapshot = encode_run(&run)?;
            transaction
                .execute(
                    "INSERT INTO bunting_runs (run_id, sequence, snapshot_json) VALUES (?1, ?2, ?3)",
                    [run_id, run.sequence().get().to_string(), snapshot],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            transaction.commit().await.map_err(|_| OriginError::Unavailable)
        })
    }

    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        let connection = self
            .inner
            .connection
            .lock()
            .map_err(|_| OriginError::Unavailable)?;
        self.inner.runtime.block_on(async {
            let mut rows = connection
                .query(
                    "SELECT event_json FROM bunting_events WHERE run_id = ?1 ORDER BY ordinal ASC",
                    [run_id.get().to_string()],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let mut events = Vec::new();
            while let Some(row) = rows.next().await.map_err(|_| OriginError::Unavailable)? {
                let encoded: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
                events.push(serde_json::from_str(&encoded).map_err(|_| OriginError::Unavailable)?);
            }
            Ok(events)
        })
    }
}

impl OriginStore for TursoOriginStore {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        let connection = self
            .inner
            .connection
            .lock()
            .map_err(|_| OriginError::Unavailable)?;
        self.inner.runtime.block_on(async {
            let mut rows = connection
                .query(
                    "SELECT snapshot_json FROM bunting_runs WHERE run_id = ?1",
                    [run_id.get().to_string()],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let Some(row) = rows.next().await.map_err(|_| OriginError::Unavailable)? else {
                return Err(OriginError::UnknownRun);
            };
            let encoded: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
            let run = decode_run(&encoded)?;
            if run.run_id() != run_id {
                return Err(OriginError::Unavailable);
            }
            Ok(run)
        })
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        let connection = self
            .inner
            .connection
            .lock()
            .map_err(|_| OriginError::Unavailable)?;
        self.inner.runtime.block_on(async {
            let mut rows = connection
                .query(
                    "SELECT fingerprint, result_json FROM bunting_commands WHERE run_id = ?1 AND command_id = ?2",
                    [run_id.get().to_string(), command_id.get().to_string()],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let Some(row) = rows.next().await.map_err(|_| OriginError::Unavailable)? else {
                return Ok(None);
            };
            let fingerprint: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
            let encoded: String = row.get(1).map_err(|_| OriginError::Unavailable)?;
            let result = serde_json::from_str(&encoded).map_err(|_| OriginError::Unavailable)?;
            Ok(Some((fingerprint, result)))
        })
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        let mut connection = self
            .inner
            .connection
            .lock()
            .map_err(|_| OriginError::Unavailable)?;
        let max_commands = self.inner.max_commands;
        let max_events_per_run = self.inner.max_events_per_run;
        self.inner.runtime.block_on(async move {
            let transaction = connection
                .transaction_with_behavior(turso::transaction::TransactionBehavior::Immediate)
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let run_id = request.run_id.get().to_string();
            let command_id = request.command_id.get().to_string();
            let mut command_rows = transaction
                .query(
                    "SELECT fingerprint, result_json FROM bunting_commands WHERE run_id = ?1 AND command_id = ?2",
                    [run_id.clone(), command_id.clone()],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            if let Some(row) = command_rows
                .next()
                .await
                .map_err(|_| OriginError::Unavailable)?
            {
                let fingerprint: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
                let encoded: String = row.get(1).map_err(|_| OriginError::Unavailable)?;
                let result: CommandResult =
                    serde_json::from_str(&encoded).map_err(|_| OriginError::Unavailable)?;
                return if fingerprint == request.fingerprint {
                    transaction.rollback().await.map_err(|_| OriginError::Unavailable)?;
                    Ok(CommitOutcome::Duplicate(result))
                } else {
                    Err(OriginError::IdempotencyConflict)
                };
            }

            let mut run_rows = transaction
                .query(
                    "SELECT sequence FROM bunting_runs WHERE run_id = ?1",
                    [run_id.clone()],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let Some(row) = run_rows.next().await.map_err(|_| OriginError::Unavailable)? else {
                return Err(OriginError::UnknownRun);
            };
            let sequence: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
            let current = sequence
                .parse::<u64>()
                .map(EventSequence::new)
                .map_err(|_| OriginError::Unavailable)?;
            validate_commit(&request, current)?;

            let command_count = count_rows(&transaction, "SELECT COUNT(*) FROM bunting_commands").await?;
            if command_count >= max_commands {
                return Err(OriginError::Unavailable);
            }
            let mut count_rows_for_run = transaction
                .query(
                    "SELECT COUNT(*) FROM bunting_events WHERE run_id = ?1",
                    [run_id.clone()],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let event_count = count_rows_for_run
                .next()
                .await
                .map_err(|_| OriginError::Unavailable)?
                .ok_or(OriginError::Unavailable)?
                .get::<i64>(0)
                .map_err(|_| OriginError::Unavailable)?;
            let event_count = usize::try_from(event_count).map_err(|_| OriginError::Unavailable)?;
            if event_count.saturating_add(request.events.len()) > max_events_per_run {
                return Err(OriginError::Unavailable);
            }

            for (offset, event) in request.events.iter().enumerate() {
                let ordinal = event_count
                    .checked_add(offset)
                    .ok_or(OriginError::InvalidCommit)?;
                let event_json =
                    serde_json::to_string(event).map_err(|_| OriginError::InvalidCommit)?;
                transaction
                    .execute(
                        "INSERT INTO bunting_events (run_id, ordinal, event_sequence, event_json) VALUES (?1, ?2, ?3, ?4)",
                        turso::params![
                            run_id.clone(),
                            i64::try_from(ordinal).map_err(|_| OriginError::InvalidCommit)?,
                            event.sequence.get().to_string(),
                            event_json
                        ],
                    )
                    .await
                    .map_err(|_| OriginError::Unavailable)?;
            }

            let result_json =
                serde_json::to_string(&request.result).map_err(|_| OriginError::InvalidCommit)?;
            transaction
                .execute(
                    "INSERT INTO bunting_commands (run_id, command_id, fingerprint, result_json) VALUES (?1, ?2, ?3, ?4)",
                    [
                        run_id.clone(),
                        command_id,
                        request.fingerprint.clone(),
                        result_json,
                    ],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            let snapshot = encode_run(&request.candidate)?;
            transaction
                .execute(
                    "UPDATE bunting_runs SET sequence = ?1, snapshot_json = ?2 WHERE run_id = ?3",
                    [
                        request.candidate.sequence().get().to_string(),
                        snapshot,
                        run_id,
                    ],
                )
                .await
                .map_err(|_| OriginError::Unavailable)?;
            transaction
                .commit()
                .await
                .map_err(|_| OriginError::Unavailable)?;
            Ok(CommitOutcome::Committed(request.result))
        })
    }
}

async fn initialize_turso(connection: &turso::Connection) -> Result<(), OriginError> {
    connection
        .execute_batch(
            "CREATE TABLE bunting_meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);\n\
             CREATE TABLE bunting_runs (run_id TEXT PRIMARY KEY NOT NULL, sequence TEXT NOT NULL, snapshot_json TEXT NOT NULL);\n\
             CREATE TABLE bunting_commands (run_id TEXT NOT NULL, command_id TEXT NOT NULL, fingerprint TEXT NOT NULL, result_json TEXT NOT NULL, PRIMARY KEY (run_id, command_id));\n\
             CREATE TABLE bunting_events (run_id TEXT NOT NULL, ordinal INTEGER NOT NULL, event_sequence TEXT NOT NULL, event_json TEXT NOT NULL, PRIMARY KEY (run_id, ordinal));",
        )
        .await
        .map_err(|_| OriginError::Unavailable)?;
    connection
        .execute(
            "INSERT INTO bunting_meta (key, value) VALUES ('schema_version', ?1)",
            [TURSO_SCHEMA_VERSION],
        )
        .await
        .map_err(|_| OriginError::Unavailable)?;
    Ok(())
}

async fn validate_existing_turso(connection: &turso::Connection) -> Result<(), OriginError> {
    connection
        .query("PRAGMA schema_version", ())
        .await
        .map_err(|_| OriginError::Unavailable)?;
    let mut rows = connection
        .query(
            "SELECT value FROM bunting_meta WHERE key = 'schema_version'",
            (),
        )
        .await
        .map_err(|_| OriginError::Unavailable)?;
    let Some(row) = rows.next().await.map_err(|_| OriginError::Unavailable)? else {
        return Err(OriginError::Unavailable);
    };
    let version: String = row.get(0).map_err(|_| OriginError::Unavailable)?;
    if version != TURSO_SCHEMA_VERSION {
        return Err(OriginError::Unavailable);
    }
    Ok(())
}

async fn count_rows(connection: &turso::Connection, sql: &str) -> Result<usize, OriginError> {
    let mut rows = connection
        .query(sql, ())
        .await
        .map_err(|_| OriginError::Unavailable)?;
    let count = rows
        .next()
        .await
        .map_err(|_| OriginError::Unavailable)?
        .ok_or(OriginError::Unavailable)?
        .get::<i64>(0)
        .map_err(|_| OriginError::Unavailable)?;
    usize::try_from(count).map_err(|_| OriginError::Unavailable)
}

fn encode_run(run: &RunState) -> Result<String, OriginError> {
    run.snapshot_envelope()
        .map_err(|_| OriginError::InvalidCommit)?
        .to_json()
        .map_err(|_| OriginError::InvalidCommit)
}

fn decode_run(encoded: &str) -> Result<RunState, OriginError> {
    EngineSnapshotEnvelope::from_json(encoded)
        .map(|envelope| envelope.state)
        .map_err(|_| OriginError::Unavailable)
}

fn validate_commit(request: &CommitRequest, current: EventSequence) -> Result<(), OriginError> {
    if current != request.expected_version {
        return Err(OriginError::VersionConflict { current });
    }
    let next = request
        .expected_version
        .checked_add(EventSequence::new(1))
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
    Ok(())
}

#[derive(Clone, Debug)]
pub enum NativeOrigin {
    Memory(InMemoryOrigin),
    File(FileOriginStore),
    Turso(TursoOriginStore),
}

impl NativeOrigin {
    pub fn from_config(config: &StorageConfig) -> Result<Self, OriginError> {
        match config.kind {
            StorageKind::Memory => Ok(Self::Memory(InMemoryOrigin::new())),
            StorageKind::File => Ok(Self::File(FileOriginStore::open(
                config.path.as_deref().ok_or(OriginError::Unavailable)?,
                config,
            )?)),
            StorageKind::Turso => Ok(Self::Turso(TursoOriginStore::open(
                config.path.as_deref().ok_or(OriginError::Unavailable)?,
                config,
            )?)),
        }
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        match self {
            Self::Memory(store) => store.insert_run(run),
            Self::File(store) => store.insert_run(run),
            Self::Turso(store) => store.insert_run(run),
        }
    }

    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        match self {
            Self::Memory(store) => store.events(run_id),
            Self::File(store) => store.events(run_id),
            Self::Turso(store) => store.events(run_id),
        }
    }
}

impl OriginStore for NativeOrigin {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        match self {
            Self::Memory(store) => store.load_run(run_id),
            Self::File(store) => store.load_run(run_id),
            Self::Turso(store) => store.load_run(run_id),
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
            Self::Turso(store) => store.find_command(run_id, command_id),
        }
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        match self {
            Self::Memory(store) => store.commit(request),
            Self::File(store) => store.commit(request),
            Self::Turso(store) => store.commit(request),
        }
    }
}
