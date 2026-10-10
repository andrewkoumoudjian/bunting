#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Host-independent authoritative run ownership and journal contract.
//!
//! One writer owns each run's live [`RunState`]. An input is applied in place
//! by `bunting-engine`, recorded as a command-sourced [`CommandRecord`]
//! (input, result, canonical events and an event-hash chain) and only then
//! acknowledged. Full state appears only at genesis and in checkpoints; a
//! restart re-executes the journaled inputs after the last checkpoint and
//! requires identical results, events and chain values.

pub use bunting_admission_sequencer::AdmissionRecord;
pub use bunting_engine::RunState;
use bunting_engine::{ApplyError, EngineError};
use bunting_market_events::{Command, EventEnvelope, SimulationCommandRequest};
use bunting_market_types::{CommandId, EventSequence, OrderId, RunId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
const GENESIS_DOMAIN: &[u8] = b"bunting.journal.v3.genesis\0";

/// Stable persisted command response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandResult {
    pub accepted: bool,
    pub reject_code: Option<String>,
    pub committed_sequence: EventSequence,
    pub order_id: Option<OrderId>,
}

/// One authoritative input. Order flow and simulation administration share
/// one ordered journal per run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalInput {
    Command(Command),
    Simulation(SimulationCommandRequest),
}

impl JournalInput {
    #[must_use]
    pub const fn run_id(&self) -> RunId {
        match self {
            Self::Command(command) => command.run_id,
            Self::Simulation(request) => request.run_id,
        }
    }

    #[must_use]
    pub const fn command_id(&self) -> CommandId {
        match self {
            Self::Command(command) => command.command_id,
            Self::Simulation(request) => request.command_id,
        }
    }

    #[must_use]
    pub const fn expected_sequence(&self) -> EventSequence {
        match self {
            Self::Command(command) => command.expected_sequence,
            Self::Simulation(request) => request.expected_sequence,
        }
    }

    /// Stable idempotency fingerprint: SHA-256 of the inner value's JSON, so
    /// it equals [`command_fingerprint`] or [`simulation_command_fingerprint`].
    pub fn fingerprint(&self) -> Result<String, OriginError> {
        self.fingerprint_bytes().map(|bytes| hex(&bytes))
    }

    fn fingerprint_bytes(&self) -> Result<[u8; 32], OriginError> {
        match self {
            Self::Command(command) => digest_json(command),
            Self::Simulation(request) => digest_json(request),
        }
    }
}

pub fn command_fingerprint(command: &Command) -> Result<String, OriginError> {
    digest_json(command).map(|bytes| hex(&bytes))
}

pub fn simulation_command_fingerprint(
    request: &SimulationCommandRequest,
) -> Result<String, OriginError> {
    digest_json(request).map(|bytes| hex(&bytes))
}

/// Durable record of one committed input.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandRecord {
    pub input: JournalInput,
    pub fingerprint: String,
    pub result: CommandResult,
    pub events: Vec<EventEnvelope>,
    /// Hex SHA-256 of the previous chain value followed by the canonical JSON
    /// of every other field of this record, so the chain authenticates
    /// inputs, results, events and admission metadata alike. The first value
    /// derives from the genesis state hash.
    pub chain: String,
    /// How the input was ordered (ADR 0030), when it went through the
    /// admission sequencer. Recorded so replay never re-measures latency;
    /// it is not part of the idempotency fingerprint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission: Option<AdmissionRecord>,
}

/// Committed facts returned to the caller of [`OriginStore::execute`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Executed {
    pub result: CommandResult,
    /// Empty for a duplicate: its events were published when it committed.
    pub events: Vec<EventEnvelope>,
    pub duplicate: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OriginError {
    UnknownRun,
    VersionConflict {
        current: EventSequence,
    },
    IdempotencyConflict,
    InvalidCommit,
    Unavailable,
    /// A configured per-run bound would be exceeded; nothing was committed.
    CapacityExceeded,
    /// The engine refused the input; nothing was committed.
    Engine(EngineError),
}

impl fmt::Display for OriginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for OriginError {}

/// Per-run bounds. Memory per committed command is one index entry; the
/// journal itself lives in the host's durable store.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RunLimits {
    pub max_commands: usize,
    pub max_events: u64,
    /// Committed inputs between in-memory (and durable) checkpoints. Bounds
    /// rebuild work after a poisoned transition and restart re-execution.
    pub checkpoint_interval: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            max_commands: 2_097_152,
            max_events: 16_777_216,
            checkpoint_interval: 8_192,
        }
    }
}

/// Atomic origin boundary: one writer per run, journal before acknowledgement.
pub trait OriginStore {
    /// Applies one input to the live run, records it durably and returns the
    /// committed facts. A replay of a committed command ID with the same
    /// fingerprint returns the original result as a duplicate.
    fn execute(&self, input: &JournalInput) -> Result<Executed, OriginError> {
        self.execute_admitted(input, None)
    }

    /// [`Self::execute`] for an input ordered by the admission sequencer;
    /// `admission` is journaled with the record.
    fn execute_admitted(
        &self,
        input: &JournalInput,
        admission: Option<&AdmissionRecord>,
    ) -> Result<Executed, OriginError>;

    /// Reads the committed live run without copying it. `read` must not call
    /// back into the store.
    fn read_run<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, OriginError>;

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError>;

    /// Copies the committed run. This costs O(state); keep it off
    /// per-command paths.
    fn clone_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        self.read_run(run_id, RunState::clone)
    }
}

#[derive(Clone, Debug)]
struct IndexedCommand {
    fingerprint: [u8; 32],
    result: CommandResult,
}

/// Outcome of [`LiveRun::execute`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Execution {
    Duplicate(CommandResult),
    /// Applied in memory; the host must persist the record before
    /// acknowledging it, and must stop serving if that write fails.
    Committed(Box<CommandRecord>),
}

/// The writer-owned state of one run.
///
/// `base` is the last checkpointed state and `tail` the inputs committed
/// since, so a transition the engine reports as poisoned is rolled back by
/// re-executing at most `checkpoint_interval` inputs, without any I/O.
#[derive(Clone, Debug)]
pub struct LiveRun {
    state: RunState,
    chain: [u8; 32],
    base: RunState,
    base_chain: [u8; 32],
    tail: Vec<JournalInput>,
    index: BTreeMap<CommandId, IndexedCommand>,
    limits: RunLimits,
    poisoned: bool,
}

impl LiveRun {
    /// Starts a run from its genesis state.
    pub fn genesis(state: RunState, limits: RunLimits) -> Result<Self, OriginError> {
        let chain = genesis_chain(&state)?;
        Ok(Self {
            base: state.clone(),
            base_chain: chain,
            state,
            chain,
            tail: Vec::new(),
            index: BTreeMap::new(),
            limits,
            poisoned: false,
        })
    }

    /// The committed live state.
    pub fn state(&self) -> Result<&RunState, OriginError> {
        if self.poisoned {
            return Err(OriginError::Unavailable);
        }
        Ok(&self.state)
    }

    #[must_use]
    pub fn chain(&self) -> String {
        hex(&self.chain)
    }

    /// The last checkpointed state and its chain value.
    #[must_use]
    pub fn checkpoint_state(&self) -> (&RunState, String) {
        (&self.base, hex(&self.base_chain))
    }

    #[must_use]
    pub fn command_count(&self) -> usize {
        self.index.len()
    }

    #[must_use]
    pub fn find(&self, command_id: CommandId) -> Option<(String, CommandResult)> {
        self.index
            .get(&command_id)
            .map(|entry| (hex(&entry.fingerprint), entry.result.clone()))
    }

    /// Applies one input in place and returns the record to persist.
    pub fn execute(&mut self, input: &JournalInput) -> Result<Execution, OriginError> {
        self.execute_admitted(input, None)
    }

    /// [`Self::execute`] carrying the admission decision into the record.
    pub fn execute_admitted(
        &mut self,
        input: &JournalInput,
        admission: Option<&AdmissionRecord>,
    ) -> Result<Execution, OriginError> {
        if self.poisoned {
            return Err(OriginError::Unavailable);
        }
        if input.run_id() != self.state.run_id() {
            return Err(OriginError::UnknownRun);
        }
        let fingerprint = input.fingerprint_bytes()?;
        if let Some(entry) = self.index.get(&input.command_id()) {
            return if entry.fingerprint == fingerprint {
                Ok(Execution::Duplicate(entry.result.clone()))
            } else {
                Err(OriginError::IdempotencyConflict)
            };
        }
        if self.state.sequence() != input.expected_sequence() {
            return Err(OriginError::VersionConflict {
                current: self.state.sequence(),
            });
        }
        if self.index.len() >= self.limits.max_commands {
            return Err(OriginError::CapacityExceeded);
        }
        let applied = match input {
            JournalInput::Command(command) => self.state.apply(command),
            JournalInput::Simulation(request) => self.state.apply_simulation(request),
        };
        let applied = match applied {
            Ok(applied) => applied,
            Err(ApplyError::Unchanged(error)) => return Err(OriginError::Engine(error)),
            Err(ApplyError::Poisoned(error)) => {
                self.rebuild()?;
                return Err(OriginError::Engine(error));
            }
        };
        if self.state.event_sequence().get() > self.limits.max_events {
            self.rebuild()?;
            return Err(OriginError::CapacityExceeded);
        }
        let result = CommandResult {
            accepted: applied.accepted,
            reject_code: applied.reject_code,
            committed_sequence: self.state.sequence(),
            order_id: applied.order_id,
        };
        let fingerprint_hex = hex(&fingerprint);
        let chain = next_chain(
            &self.chain,
            &Chained {
                input,
                fingerprint: &fingerprint_hex,
                result: &result,
                events: &applied.events,
                admission,
            },
        )?;
        self.index.insert(
            input.command_id(),
            IndexedCommand {
                fingerprint,
                result: result.clone(),
            },
        );
        self.tail.push(input.clone());
        self.chain = chain;
        Ok(Execution::Committed(Box::new(CommandRecord {
            input: input.clone(),
            fingerprint: fingerprint_hex,
            result,
            events: applied.events,
            chain: hex(&chain),
            admission: admission.copied(),
        })))
    }

    #[must_use]
    pub fn needs_checkpoint(&self) -> bool {
        self.tail.len() >= self.limits.checkpoint_interval.max(1)
    }

    /// Makes the live state the rollback base. Costs one state copy, once
    /// per `checkpoint_interval` commands.
    pub fn checkpoint(&mut self) -> Result<(), OriginError> {
        if self.poisoned {
            return Err(OriginError::Unavailable);
        }
        self.base = self.state.clone();
        self.base_chain = self.chain;
        self.tail.clear();
        Ok(())
    }

    /// Restores the last committed state from `base` and `tail`. A failure
    /// here means the committed history no longer re-executes, so the run
    /// stops serving until a restart re-verifies it.
    fn rebuild(&mut self) -> Result<(), OriginError> {
        let mut state = self.base.clone();
        for input in &self.tail {
            let replayed = match input {
                JournalInput::Command(command) => state.apply(command).map(|_| ()),
                JournalInput::Simulation(request) => state.apply_simulation(request).map(|_| ()),
            };
            if replayed.is_err() {
                self.poisoned = true;
                return Err(OriginError::Unavailable);
            }
        }
        self.state = state;
        Ok(())
    }
}

/// Rebuilds one run from its genesis state, an optional checkpoint and its
/// journal records in order. Records the checkpoint covers are verified by
/// fingerprint, sequence and chain and indexed; later records are
/// re-executed and must reproduce the journaled record exactly.
#[derive(Debug)]
pub struct RunRecovery {
    live: LiveRun,
    genesis_sequence: EventSequence,
    replayed_through: EventSequence,
    checkpoint_sequence: EventSequence,
    checkpoint_chain: [u8; 32],
}

impl RunRecovery {
    pub fn new(
        genesis: RunState,
        checkpoint: Option<(RunState, &str)>,
        limits: RunLimits,
    ) -> Result<Self, OriginError> {
        let genesis_sequence = genesis.sequence();
        let mut live = LiveRun::genesis(genesis, limits)?;
        let (checkpoint_sequence, checkpoint_chain) = match checkpoint {
            None => (genesis_sequence, live.chain),
            Some((state, chain)) => {
                if state.run_id() != live.state.run_id()
                    || state.scenario_hash() != live.state.scenario_hash()
                    || state.sequence() < genesis_sequence
                {
                    return Err(OriginError::InvalidCommit);
                }
                let chain = parse_hex(chain)?;
                let sequence = state.sequence();
                live.base = state.clone();
                live.base_chain = chain;
                live.state = state;
                (sequence, chain)
            }
        };
        let recovery = Self {
            live,
            genesis_sequence,
            replayed_through: genesis_sequence,
            checkpoint_sequence,
            checkpoint_chain,
        };
        recovery.check_checkpoint_reached()?;
        Ok(recovery)
    }

    /// Applies the next journal record of this run.
    pub fn replay(&mut self, record: &CommandRecord) -> Result<(), OriginError> {
        let sequence = record.result.committed_sequence;
        let next = self
            .replayed_through
            .checked_add(EventSequence::new(1))
            .ok_or(OriginError::InvalidCommit)?;
        if sequence != next
            || record.input.run_id() != self.live.state.run_id()
            || record.input.expected_sequence() != self.replayed_through
        {
            return Err(OriginError::InvalidCommit);
        }
        if sequence <= self.checkpoint_sequence {
            let fingerprint = record.input.fingerprint_bytes()?;
            let chain = next_chain(&self.live.chain, &Chained::of(record))?;
            if hex(&fingerprint) != record.fingerprint
                || hex(&chain) != record.chain
                || self.live.index.contains_key(&record.input.command_id())
            {
                return Err(OriginError::InvalidCommit);
            }
            self.live.index.insert(
                record.input.command_id(),
                IndexedCommand {
                    fingerprint,
                    result: record.result.clone(),
                },
            );
            self.live.chain = chain;
            self.replayed_through = sequence;
            return self.check_checkpoint_reached();
        }
        match self
            .live
            .execute_admitted(&record.input, record.admission.as_ref())
        {
            Ok(Execution::Committed(produced)) if produced.as_ref() == record => {
                self.replayed_through = sequence;
                Ok(())
            }
            _ => Err(OriginError::InvalidCommit),
        }
    }

    /// Returns the recovered run once every journal record has been replayed.
    pub fn finish(self) -> Result<LiveRun, OriginError> {
        if self.replayed_through < self.checkpoint_sequence {
            // The checkpoint claims commands the journal does not hold.
            return Err(OriginError::InvalidCommit);
        }
        Ok(self.live)
    }

    #[must_use]
    pub const fn genesis_sequence(&self) -> EventSequence {
        self.genesis_sequence
    }

    fn check_checkpoint_reached(&self) -> Result<(), OriginError> {
        if self.replayed_through == self.checkpoint_sequence
            && self.live.chain != self.checkpoint_chain
        {
            return Err(OriginError::InvalidCommit);
        }
        Ok(())
    }
}

/// Non-durable origin for tests, embeddings and local runs.
#[derive(Clone, Debug, Default)]
pub struct InMemoryOrigin {
    runs: Arc<Mutex<BTreeMap<RunId, LiveRun>>>,
    limits: RunLimits,
}

impl InMemoryOrigin {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_limits(limits: RunLimits) -> Self {
        Self {
            runs: Arc::default(),
            limits,
        }
    }

    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut runs = self.runs.lock().map_err(|_| OriginError::Unavailable)?;
        if let Some(existing) = runs.get(&run.run_id()) {
            return if existing.state()? == &run {
                Ok(())
            } else {
                Err(OriginError::InvalidCommit)
            };
        }
        runs.insert(run.run_id(), LiveRun::genesis(run, self.limits)?);
        Ok(())
    }
}

impl OriginStore for InMemoryOrigin {
    fn execute_admitted(
        &self,
        input: &JournalInput,
        admission: Option<&AdmissionRecord>,
    ) -> Result<Executed, OriginError> {
        let mut runs = self.runs.lock().map_err(|_| OriginError::Unavailable)?;
        let live = runs
            .get_mut(&input.run_id())
            .ok_or(OriginError::UnknownRun)?;
        match live.execute_admitted(input, admission)? {
            Execution::Duplicate(result) => Ok(Executed {
                result,
                events: Vec::new(),
                duplicate: true,
            }),
            Execution::Committed(record) => {
                if live.needs_checkpoint() {
                    live.checkpoint()?;
                }
                Ok(Executed {
                    result: record.result,
                    events: record.events,
                    duplicate: false,
                })
            }
        }
    }

    fn read_run<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, OriginError> {
        let runs = self.runs.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(read(
            runs.get(&run_id).ok_or(OriginError::UnknownRun)?.state()?,
        ))
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        let runs = self.runs.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(runs
            .get(&run_id)
            .ok_or(OriginError::UnknownRun)?
            .find(command_id))
    }
}

fn digest_json(value: &impl Serialize) -> Result<[u8; 32], OriginError> {
    let bytes = serde_json::to_vec(value).map_err(|_| OriginError::InvalidCommit)?;
    Ok(Sha256::digest(bytes).into())
}

fn genesis_chain(state: &RunState) -> Result<[u8; 32], OriginError> {
    let state_hash = state.state_hash().map_err(|_| OriginError::InvalidCommit)?;
    let mut hasher = Sha256::new();
    hasher.update(GENESIS_DOMAIN);
    hasher.update(state_hash.as_bytes());
    Ok(hasher.finalize().into())
}

/// The fields of a [`CommandRecord`] that its chain value covers.
#[derive(Serialize)]
struct Chained<'a> {
    input: &'a JournalInput,
    fingerprint: &'a str,
    result: &'a CommandResult,
    events: &'a [EventEnvelope],
    admission: Option<&'a AdmissionRecord>,
}

impl<'a> Chained<'a> {
    fn of(record: &'a CommandRecord) -> Self {
        Self {
            input: &record.input,
            fingerprint: &record.fingerprint,
            result: &record.result,
            events: &record.events,
            admission: record.admission.as_ref(),
        }
    }
}

fn next_chain(previous: &[u8; 32], record: &Chained<'_>) -> Result<[u8; 32], OriginError> {
    let bytes = serde_json::to_vec(record).map_err(|_| OriginError::InvalidCommit)?;
    let mut hasher = Sha256::new();
    hasher.update(previous);
    hasher.update(bytes);
    Ok(hasher.finalize().into())
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn parse_hex(value: &str) -> Result<[u8; 32], OriginError> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(OriginError::InvalidCommit),
    };
    let bytes = value.as_bytes();
    if bytes.len() != 64 {
        return Err(OriginError::InvalidCommit);
    }
    let mut output = [0_u8; 32];
    for (index, pair) in bytes.chunks_exact(2).enumerate() {
        output[index] = (digit(pair[0])? << 4) | digit(pair[1])?;
    }
    Ok(output)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests;
