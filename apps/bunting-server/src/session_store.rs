//! Durable FIX session state: an append-only log per participant (ADR 0045).
//!
//! `<origin>.fix-session-<participant>.log` holds a checkpoint of the whole
//! session and application state followed by one delta per write: the
//! session's sequence numbers and timers, the resend-journal frames added and
//! removed, and the application inputs (inbound messages and committed
//! batches) applied since the previous delta. Recovery restores the
//! checkpoint and replays the deltas; the application state is rebuilt by
//! re-running its inputs, which is deterministic. [`SessionStore`] owns the
//! application, so no input can change it without being recorded.
//!
//! A delta is durable before any frame it covers is written to the socket
//! (write-ahead): a restarted venue never reuses an outgoing sequence number
//! and can resend every frame the participant may have seen. State that no
//! frame depends on yet (an inbound message not answered so far) rides with
//! the next write instead of costing a disk sync of its own. After a crash:
//!
//! - an inbound message the log never recorded is still the participant's
//!   to resend: the restored incoming sequence number asks for it;
//! - a recorded command that never committed (it was waiting for release)
//!   is admitted again ([`Recovery::readmit`]);
//! - a committed batch the log never recorded is delivered again from the
//!   distributor's retained batches or the origin journal, after
//!   [`Recovery::batches_after`];
//! - a command resent or readmitted after it committed is not committed
//!   again: its command ID is already journaled (see `job_for`).
//!
//! Records are framed like the origin journal (magic, length, SHA-256); an
//! incomplete final record is a crash tail and is dropped, any other damage
//! refuses the session.
use crate::delivery::{Delivery, Unwritten};
use bunting_application::{
    ApplicationError, FixApplicationRequest, FixApplicationSnapshot, FixApplicationState,
    FixCommandContext,
};
use bunting_market_events::{Command, EventEnvelope, EventPayload};
use bunting_market_types::{CorrelationId, EventSequence, LogicalTimeNs, ParticipantId, RunId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use simfix_session::{ConnectionState, FixSession, JournalEntry, SessionConfig, SessionSnapshot};
use simfix_wire::FixMessage;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::ops::Bound;
use std::path::{Path, PathBuf};

const MAGIC: [u8; 8] = *b"BUNTFIX1";
const HEADER_SIZE: usize = 48;
const MAX_RECORD_BYTES: usize = 256 * 1024 * 1024;
/// Deltas may grow to this, or to the checkpoint's size if larger, before
/// the log is compacted into a new checkpoint.
const MIN_COMPACT_BYTES: u64 = 8 * 1024 * 1024;

/// The command context of an inbound application message. Sequence and
/// logical time are stamped at release; the mapping never reads them, so
/// live admission and replay use the same placeholders.
pub(crate) const fn command_context(
    actor: ParticipantId,
    run_id: RunId,
    correlation_id: u128,
) -> FixCommandContext {
    FixCommandContext {
        actor,
        run_id,
        expected_sequence: EventSequence::new(0),
        logical_time: LogicalTimeNs::new(0),
        correlation_id: CorrelationId::new(correlation_id),
    }
}

/// One input that changed the application state, in the order applied.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Input {
    /// `FixApplicationState::map_message`, whatever its outcome.
    Inbound {
        message: FixMessage,
        actor: ParticipantId,
        run_id: RunId,
        correlation_id: u128,
    },
    /// `FixApplicationState::committed_messages` for a batch that produced
    /// reports or registered one of the participant's orders; no other
    /// batch changes the application.
    Committed {
        actor: ParticipantId,
        events: Vec<EventEnvelope>,
    },
}

/// A command mapped from an inbound message, with its correlation ID.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Mapped {
    pub(crate) command: Command,
    pub(crate) correlation_id: u128,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
enum Record {
    Checkpoint(Box<Checkpoint>),
    Delta(Box<Delta>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    session: SessionSnapshot,
    application: FixApplicationSnapshot,
    seen_through: u64,
    unwritten: Vec<Unwritten>,
    /// Commands mapped before the checkpoint and not yet journaled then.
    in_flight: Vec<Mapped>,
}

/// Borrowed form of [`Record`] with the identical encoding.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
enum RecordRef<'a> {
    Checkpoint(CheckpointRef<'a>),
    Delta(DeltaRef<'a>),
}

#[derive(Serialize)]
struct CheckpointRef<'a> {
    session: &'a SessionSnapshot,
    application: &'a FixApplicationSnapshot,
    seen_through: u64,
    unwritten: Vec<&'a Unwritten>,
    in_flight: &'a [Mapped],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delta {
    incoming_sequence: u64,
    outgoing_sequence: u64,
    reconnect_generation: u64,
    state: ConnectionState,
    pending_resend_begin: Option<u64>,
    pending_inbound: BTreeMap<u64, FixMessage>,
    last_received_millis: u64,
    last_sent_millis: u64,
    outstanding_test_request: Option<String>,
    added: Vec<JournalEntry>,
    removed: Vec<u64>,
    inputs: Vec<Input>,
    /// See [`Delivery`]: the batches applied, and those whose reports were
    /// still unwritten when the delta became durable.
    seen_through: u64,
    unwritten: Vec<u64>,
}

#[derive(Serialize)]
struct DeltaRef<'a> {
    incoming_sequence: u64,
    outgoing_sequence: u64,
    reconnect_generation: u64,
    state: ConnectionState,
    pending_resend_begin: Option<u64>,
    pending_inbound: &'a BTreeMap<u64, FixMessage>,
    last_received_millis: u64,
    last_sent_millis: u64,
    outstanding_test_request: Option<&'a str>,
    added: Vec<&'a JournalEntry>,
    removed: &'a [u64],
    inputs: &'a [Input],
    seen_through: u64,
    unwritten: &'a [u64],
}

/// What a restored session must redo once it is connected.
#[derive(Debug, Default)]
pub(crate) struct Recovery {
    /// Committed batches after this event sequence never reached the
    /// application; `None` for a new session, which has nothing to miss.
    pub(crate) batches_after: Option<u64>,
    /// Reports applied but never written, oldest first, to send now.
    pub(crate) owed: Vec<FixMessage>,
    /// Recorded commands to admit again, in order, unless already journaled.
    pub(crate) readmit: Vec<Mapped>,
}

/// The participant's FIX application state and its log; without an origin
/// file (a memory origin) nothing is written.
pub(crate) struct SessionStore<'a> {
    application: FixApplicationState,
    delivery: Delivery,
    writer: Option<Writer>,
    /// Whether a command ID is in the origin journal.
    journaled: Box<dyn Fn(&Command) -> bool + 'a>,
}

struct Writer {
    path: PathBuf,
    file: File,
    /// Resend journal keys as of the last durable record.
    journal: BTreeSet<u64>,
    /// Application inputs applied since the last durable record.
    inputs: Vec<Input>,
    /// Commands mapped since the last checkpoint, for the next one.
    mapped: Vec<Mapped>,
    since_checkpoint: u64,
    checkpoint_bytes: u64,
}

impl<'a> SessionStore<'a> {
    /// Restores the session and application from the log at `path`, or
    /// starts them fresh, and compacts the log into one checkpoint. Without
    /// a path the store keeps state in memory only.
    pub(crate) fn open(
        path: Option<&Path>,
        config: SessionConfig,
        fresh: impl FnOnce() -> FixApplicationState,
        current_sequence: u64,
        journaled: impl Fn(&Command) -> bool + 'a,
    ) -> Result<(Self, FixSession, Recovery), String> {
        let restored = path.map(read).transpose()?.flatten();
        let (session, application, delivery, recovery) = match restored {
            Some(replayed) => {
                let readmit: Vec<Mapped> = replayed
                    .mapped
                    .into_iter()
                    .filter(|mapped| !journaled(&mapped.command))
                    .collect();
                let recovery = Recovery {
                    batches_after: Some(replayed.delivery.seen_through()),
                    owed: replayed
                        .delivery
                        .unwritten()
                        .flat_map(|batch| batch.messages.iter().cloned())
                        .collect(),
                    readmit,
                };
                (
                    FixSession::restore(config, replayed.session)
                        .map_err(|error| format!("cannot restore FIX session: {error:?}"))?,
                    replayed.application,
                    replayed.delivery,
                    recovery,
                )
            }
            // A new session has no orders, so no earlier batch concerns it.
            None => (
                FixSession::try_new(config)
                    .map_err(|error| format!("invalid FIX session config: {error:?}"))?,
                fresh(),
                Delivery::at(current_sequence),
                Recovery::default(),
            ),
        };
        let writer = path
            .map(|path| {
                Writer::checkpoint(
                    path,
                    session.state(),
                    &application,
                    &delivery,
                    &recovery.readmit,
                )
            })
            .transpose()?;
        Ok((
            Self {
                application,
                delivery,
                writer,
                journaled: Box::new(journaled),
            },
            session,
            recovery,
        ))
    }

    /// The last event sequence of the committed batches applied so far.
    pub(crate) const fn seen_through(&self) -> u64 {
        self.delivery.seen_through()
    }

    /// Every report due at `now_us` is being written: call before the
    /// write, so the log record made durable ahead of it agrees.
    pub(crate) fn on_written(&mut self, now_us: u64) {
        self.delivery.on_written(now_us);
    }

    /// Maps one inbound application message and records it.
    pub(crate) fn map_message(
        &mut self,
        message: &FixMessage,
        context: &FixCommandContext,
    ) -> Result<FixApplicationRequest, ApplicationError> {
        let request = self.application.map_message(message, context);
        if let Some(writer) = &mut self.writer {
            writer.inputs.push(Input::Inbound {
                message: message.clone(),
                actor: context.actor,
                run_id: context.run_id,
                correlation_id: context.correlation_id.get(),
            });
            if let Ok(FixApplicationRequest::Command(command)) = &request {
                writer.mapped.push(Mapped {
                    command: command.clone(),
                    correlation_id: context.correlation_id.get(),
                });
            }
        }
        request
    }

    /// Maps one committed batch to the participant's reports, which leave
    /// at `send_at_us`, and records it.
    pub(crate) fn committed_messages(
        &mut self,
        actor: ParticipantId,
        events: &[EventEnvelope],
        send_at_us: u64,
    ) -> Result<Vec<FixMessage>, ApplicationError> {
        let messages = self.application.committed_messages(actor, events)?;
        if let Some(last) = events.last() {
            self.delivery
                .on_batch(last.sequence.get(), &messages, send_at_us);
        }
        if let Some(writer) = &mut self.writer
            && (!messages.is_empty() || registers_order(actor, events))
        {
            writer.inputs.push(Input::Committed {
                actor,
                events: events.to_vec(),
            });
        }
        Ok(messages)
    }

    /// Makes the current session and application state durable. Call
    /// before writing any frame the session produced.
    pub(crate) fn persist(&mut self, session: &FixSession) -> Result<(), String> {
        let Some(writer) = &mut self.writer else {
            return Ok(());
        };
        if writer.since_checkpoint <= MIN_COMPACT_BYTES.max(writer.checkpoint_bytes)
            && writer.append_delta(session.state(), &self.delivery)?
        {
            return Ok(());
        }
        let in_flight: Vec<Mapped> = writer
            .mapped
            .iter()
            .filter(|mapped| !(self.journaled)(&mapped.command))
            .cloned()
            .collect();
        let path = writer.path.clone();
        *writer = Writer::checkpoint(
            &path,
            session.state(),
            &self.application,
            &self.delivery,
            &in_flight,
        )?;
        Ok(())
    }
}

impl SessionStore<'_> {
    /// Debug builds check at every clean close that replaying the log
    /// reproduces the live state exactly, so a mutation that escapes the
    /// log fails the server's own tests.
    #[cfg(debug_assertions)]
    pub(crate) fn verify(&self, session: &FixSession) -> Result<(), String> {
        let Some(writer) = &self.writer else {
            return Ok(());
        };
        let replayed = read(&writer.path)?.ok_or("FIX session log missing")?;
        let reports = |delivery: &Delivery| {
            delivery
                .unwritten()
                .map(|batch| (batch.last, batch.messages.clone()))
                .collect::<Vec<_>>()
        };
        if &replayed.session != session.state()
            || replayed.application.snapshot() != self.application.snapshot()
            || replayed.delivery.seen_through() != self.delivery.seen_through()
            || reports(&replayed.delivery) != reports(&self.delivery)
        {
            return Err("FIX session log replay diverged from the live session".to_owned());
        }
        Ok(())
    }
}

/// Whether the batch registers one of `actor`'s orders, which changes the
/// application even when no report results.
fn registers_order(actor: ParticipantId, events: &[EventEnvelope]) -> bool {
    events.iter().any(|event| {
        matches!(&event.payload, EventPayload::OrderReceived { order, .. }
            if order.participant_id == actor)
    })
}

impl Writer {
    /// Writes a new log holding one checkpoint and installs it atomically.
    fn checkpoint(
        path: &Path,
        session: &SessionSnapshot,
        application: &FixApplicationState,
        delivery: &Delivery,
        in_flight: &[Mapped],
    ) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .map_err(|error| format!("cannot create FIX session directory: {error}"))?;
        }
        let snapshot = application.snapshot();
        let frame = encode(&RecordRef::Checkpoint(CheckpointRef {
            session,
            application: &snapshot,
            seen_through: delivery.seen_through(),
            unwritten: delivery.unwritten().collect(),
            in_flight,
        }))?;
        let temporary = path.with_extension("tmp");
        let mut file = File::create(&temporary)
            .map_err(|error| format!("cannot create FIX session log: {error}"))?;
        file.write_all(&frame)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("cannot write FIX session log: {error}"))?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot install FIX session log: {error}"))?;
        sync_parent(path)?;
        let file = OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|error| format!("cannot open FIX session log: {error}"))?;
        Ok(Self {
            path: path.to_owned(),
            file,
            journal: session.journal.keys().copied().collect(),
            inputs: Vec::new(),
            mapped: in_flight.to_vec(),
            since_checkpoint: 0,
            checkpoint_bytes: u64::try_from(frame.len()).unwrap_or(u64::MAX),
        })
    }

    /// Appends one delta with a single data sync. Returns `false`, writing
    /// nothing, when the journal changed in a way a delta cannot express.
    fn append_delta(
        &mut self,
        session: &SessionSnapshot,
        delivery: &Delivery,
    ) -> Result<bool, String> {
        // Outgoing sequence numbers only grow, so new frames sit above every
        // recorded key; removals (the oldest frames, or a replaced one) are
        // found from the front until all are accounted for.
        let added: Vec<&JournalEntry> = match self.journal.last() {
            Some(last) => session
                .journal
                .range((Bound::Excluded(*last), Bound::Unbounded))
                .map(|(_, entry)| entry)
                .collect(),
            None => session.journal.values().collect(),
        };
        let mut missing = (self.journal.len() + added.len()).checked_sub(session.journal.len());
        let mut removed = Vec::new();
        for key in &self.journal {
            if missing == Some(0) {
                break;
            }
            if !session.journal.contains_key(key) {
                removed.push(*key);
                missing = missing.and_then(|count| count.checked_sub(1));
            }
        }
        if missing != Some(0) {
            return Ok(false);
        }
        let frame = encode(&RecordRef::Delta(DeltaRef {
            incoming_sequence: session.incoming_sequence,
            outgoing_sequence: session.outgoing_sequence,
            reconnect_generation: session.reconnect_generation,
            state: session.state,
            pending_resend_begin: session.pending_resend_begin,
            pending_inbound: &session.pending_inbound,
            last_received_millis: session.last_received_millis,
            last_sent_millis: session.last_sent_millis,
            outstanding_test_request: session.outstanding_test_request.as_deref(),
            added: added.clone(),
            removed: &removed,
            inputs: &self.inputs,
            seen_through: delivery.seen_through(),
            unwritten: &delivery.unwritten_keys(),
        }))?;
        // The record is appended, never rewritten: one data sync.
        self.file
            .write_all(&frame)
            .and_then(|()| self.file.sync_data())
            .map_err(|error| format!("cannot persist FIX session log: {error}"))?;
        for key in removed {
            self.journal.remove(&key);
        }
        self.journal
            .extend(added.iter().map(|entry| entry.sequence));
        self.inputs.clear();
        self.since_checkpoint = self
            .since_checkpoint
            .saturating_add(u64::try_from(frame.len()).unwrap_or(u64::MAX));
        Ok(true)
    }
}

fn encode(record: &RecordRef<'_>) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(record)
        .map_err(|error| format!("cannot encode FIX session log: {error}"))?;
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err("FIX session log record out of bounds".to_owned());
    }
    let mut frame = Vec::with_capacity(bytes.len().saturating_add(HEADER_SIZE));
    frame.extend_from_slice(&MAGIC);
    frame.extend_from_slice(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
    frame.extend_from_slice(&Sha256::digest(&bytes));
    frame.extend_from_slice(&bytes);
    Ok(frame)
}

/// A log replayed up to its last complete record.
struct Replayed {
    session: SessionSnapshot,
    application: FixApplicationState,
    delivery: Delivery,
    /// The checkpoint's in-flight commands, then every command mapped since.
    mapped: Vec<Mapped>,
}

impl Replayed {
    fn apply(&mut self, delta: Delta) -> Result<(), String> {
        let session = &mut self.session;
        session.incoming_sequence = delta.incoming_sequence;
        session.outgoing_sequence = delta.outgoing_sequence;
        session.reconnect_generation = delta.reconnect_generation;
        session.state = delta.state;
        session.pending_resend_begin = delta.pending_resend_begin;
        session.pending_inbound = delta.pending_inbound;
        session.last_received_millis = delta.last_received_millis;
        session.last_sent_millis = delta.last_sent_millis;
        session.outstanding_test_request = delta.outstanding_test_request;
        for sequence in delta.removed {
            session.journal.remove(&sequence);
        }
        for entry in delta.added {
            session.journal.insert(entry.sequence, entry);
        }
        for input in delta.inputs {
            match input {
                // Live admission turns a mapping error into a rejection and
                // keeps whatever the mapping changed; replay does the same.
                Input::Inbound {
                    message,
                    actor,
                    run_id,
                    correlation_id,
                } => {
                    if let Ok(FixApplicationRequest::Command(command)) = self
                        .application
                        .map_message(&message, &command_context(actor, run_id, correlation_id))
                    {
                        self.mapped.push(Mapped {
                            command,
                            correlation_id,
                        });
                    }
                }
                Input::Committed { actor, events } => {
                    let messages = self
                        .application
                        .committed_messages(actor, &events)
                        .map_err(|error| format!("FIX session log replay failed: {error}"))?;
                    if let Some(last) = events.last() {
                        self.delivery.on_batch(last.sequence.get(), &messages, 0);
                    }
                }
            }
        }
        self.delivery.advance_to(delta.seen_through);
        self.delivery.retain(&delta.unwritten);
        Ok(())
    }
}

/// Replays the log: the checkpoint, then every complete delta.
fn read(path: &Path) -> Result<Option<Replayed>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| format!("cannot read FIX session log: {error}"))?;
    let invalid = || format!("invalid FIX session log {}", path.display());
    let mut replayed: Option<Replayed> = None;
    let mut rest = bytes.as_slice();
    while !rest.is_empty() {
        let Some((header, body)) = rest.split_at_checked(HEADER_SIZE) else {
            break; // crash tail
        };
        if header[..8] != MAGIC {
            return Err(invalid());
        }
        let length = header[8..16]
            .try_into()
            .map(u64::from_le_bytes)
            .ok()
            .and_then(|length| usize::try_from(length).ok())
            .filter(|length| (1..=MAX_RECORD_BYTES).contains(length))
            .ok_or_else(invalid)?;
        let Some((record, tail)) = body.split_at_checked(length) else {
            break; // crash tail
        };
        if Sha256::digest(record)[..] != header[16..] {
            return Err(invalid());
        }
        match serde_json::from_slice(record).map_err(|_| invalid())? {
            Record::Checkpoint(checkpoint) => {
                let checkpoint = *checkpoint;
                replayed = Some(Replayed {
                    session: checkpoint.session,
                    application: FixApplicationState::restore(checkpoint.application)
                        .map_err(|error| format!("cannot restore FIX application: {error}"))?,
                    delivery: Delivery::restore(checkpoint.seen_through, checkpoint.unwritten),
                    mapped: checkpoint.in_flight,
                });
            }
            Record::Delta(delta) => replayed.as_mut().ok_or_else(invalid)?.apply(*delta)?,
        }
        rest = tail;
    }
    replayed.map(Some).ok_or_else(invalid)
}

// Windows has no portable directory sync; NTFS journals the rename itself.
#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn sync_parent(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|item| !item.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(parent)
            .and_then(|parent| parent.sync_all())
            .map_err(|error| format!("cannot sync FIX session directory: {error}"))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}
