//! Durability of the writer-owned file origin (journal format 3): restart
//! re-executes the journal, checkpoints only accelerate it, and every crash
//! point recovers a committed prefix with identical state hashes.
#![allow(clippy::unwrap_used)]

use bunting_engine::{ListingDefinition, ParticipantDefinition, RunState, ScenarioDefinition};
use bunting_market_events::{CancelOrder, Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, EventSequence, InstrumentId, IterationId, ListingKey,
    LogicalTimeNs, MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots,
    RunId, ScenarioId, ScenarioVersion, VenueId,
};
use bunting_origin_store::{InMemoryOrigin, JournalInput, OriginError, OriginStore};
use bunting_risk_engine::RiskLimits;
use bunting_server::config::{StorageConfig, StorageKind};
use bunting_server::storage::FileOriginStore;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        Self(std::env::temp_dir().join(format!("bunting-{name}-{}-{nanos}", std::process::id())))
    }

    fn origin(&self) -> PathBuf {
        self.0.join("origin.json")
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn config(path: &Path, checkpoint_interval: usize) -> StorageConfig {
    StorageConfig {
        kind: StorageKind::File,
        path: Some(path.display().to_string()),
        max_runs: 4,
        max_commands_per_run: 2_000_000,
        max_events_per_run: 16_000_000,
        checkpoint_interval,
        max_commands: None,
    }
}

fn initial_run(run_id: u128) -> RunState {
    let scenario = ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [bunting_engine::InstrumentDefinition::new(
            InstrumentId::new(1),
            "BNT",
            CurrencyId::new(1),
            bunting_engine::InstrumentKind::Equity,
        )
        .with_opening_mark(PriceTicks::new(100))],
        [ListingDefinition::new(
            ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
            "ONE".to_owned(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
        )
        .unwrap()],
        [ParticipantDefinition::new(
            ParticipantId::new(7),
            true,
            RiskLimits::new(
                QuantityLots::new(100),
                QuantityLots::new(1_000),
                QuantityLots::new(1_000),
            ),
            BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(100_000))]),
            BTreeMap::new(),
        )],
    )
    .unwrap();
    RunState::from_scenario(RunId::new(run_id), IterationId::new(1), &scenario).unwrap()
}

/// Deterministic input `index`: even indices rest a buy, odd ones cancel it.
fn input(run_id: u128, index: u64, expected_sequence: EventSequence) -> JournalInput {
    let id = u128::from(index) + 1;
    let payload = if index % 2 == 0 {
        CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(id),
            instrument_id: InstrumentId::new(1),
            participant_id: ParticipantId::new(7),
            side: Side::Buy,
            quantity: QuantityLots::new(1),
            kind: OrderKind::Limit {
                price: PriceTicks::new(90 + i64::try_from(index % 10).unwrap()),
            },
        })
    } else {
        CommandPayload::CancelOrder(CancelOrder {
            order_id: OrderId::new(id - 1),
            participant_id: ParticipantId::new(7),
        })
    };
    JournalInput::Command(Command {
        run_id: RunId::new(run_id),
        command_id: CommandId::new(id),
        correlation_id: CorrelationId::new(id),
        logical_time: LogicalTimeNs::new(index),
        expected_sequence,
        actor: ParticipantId::new(7),
        payload,
    })
}

/// Executes inputs `range` against `origin`, returning their events.
fn drive<O: OriginStore>(
    origin: &O,
    run_id: u128,
    range: std::ops::Range<u64>,
) -> Result<Vec<bunting_market_events::EventEnvelope>, OriginError> {
    let mut events = Vec::new();
    for index in range {
        let sequence = origin.read_run(RunId::new(run_id), RunState::sequence)?;
        events.extend(origin.execute(&input(run_id, index, sequence))?.events);
    }
    Ok(events)
}

/// State hash after the first `count` inputs, computed without any file I/O.
fn reference_hash(count: u64) -> String {
    let origin = InMemoryOrigin::new();
    origin.insert_run(initial_run(1)).unwrap();
    drive(&origin, 1, 0..count).unwrap();
    origin
        .read_run(RunId::new(1), |state| state.state_hash().unwrap())
        .unwrap()
}

fn recovered_hash(path: &Path, interval: usize) -> Result<String, OriginError> {
    FileOriginStore::open(path, &config(path, interval))?
        .read_run(RunId::new(1), |state| state.state_hash().unwrap())
}

#[test]
fn restart_re_executes_the_journal_and_keeps_idempotency() -> TestResult {
    let folder = Folder::new("origin-restart");
    let path = folder.origin();
    let store = FileOriginStore::open(&path, &config(&path, 1_000))?;
    store.insert_run(initial_run(1))?;
    let events = drive(&store, 1, 0..9)?;
    drop(store);

    // Below the checkpoint interval only the journal exists.
    assert!(!path.exists());
    let restored = FileOriginStore::open(&path, &config(&path, 1_000))?;
    assert_eq!(
        restored.read_run(RunId::new(1), |state| state.state_hash().unwrap())?,
        reference_hash(9)
    );
    assert_eq!(restored.events(RunId::new(1))?, events);
    // A committed command replayed after restart is still a duplicate.
    let replay = restored.execute(&input(1, 8, EventSequence::new(8)))?;
    assert!(replay.duplicate && replay.events.is_empty());
    assert_eq!(replay.result.committed_sequence, EventSequence::new(9));
    drive(&restored, 1, 9..10)?;
    drop(restored);
    assert_eq!(recovered_hash(&path, 1_000)?, reference_hash(10));
    Ok(())
}

#[test]
fn checkpoints_accelerate_restart_but_the_journal_must_confirm_them() -> TestResult {
    let folder = Folder::new("origin-checkpoint");
    let path = folder.origin();
    let journal = path.with_extension("wal");
    let store = FileOriginStore::open(&path, &config(&path, 4))?;
    store.insert_run(initial_run(1))?;
    drive(&store, 1, 0..10)?;
    drop(store);

    let original = std::fs::read(&path)?;
    let checkpoint: serde_json::Value = serde_json::from_slice(&original)?;
    assert_eq!(checkpoint["version"], serde_json::json!(3));
    assert_eq!(
        checkpoint["runs"][0]["snapshot"]["state"]["sequence"],
        serde_json::json!(8)
    );
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(10));

    let mut forged_chain = checkpoint.clone();
    forged_chain["runs"][0]["chain"] = serde_json::json!("0".repeat(64));
    std::fs::write(&path, serde_json::to_vec(&forged_chain)?)?;
    assert_eq!(recovered_hash(&path, 4), Err(OriginError::InvalidCommit));

    let mut forged_state = checkpoint.clone();
    forged_state["runs"][0]["snapshot"]["state"]["kill_switch"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_vec(&forged_state)?)?;
    assert_eq!(recovered_hash(&path, 4), Err(OriginError::InvalidCommit));

    // Stores written before journal format 3 are rejected, not migrated.
    std::fs::write(&path, br#"{"runs":[],"commands":[],"events":[]}"#)?;
    assert_eq!(recovered_hash(&path, 4), Err(OriginError::InvalidCommit));

    // Without a checkpoint the journal alone rebuilds the run from genesis.
    std::fs::remove_file(&path)?;
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(10));

    // A checkpoint without the journal it came from is refused.
    std::fs::write(&path, &original)?;
    std::fs::rename(&journal, journal.with_extension("moved"))?;
    assert_eq!(recovered_hash(&path, 4), Err(OriginError::InvalidCommit));
    Ok(())
}

#[test]
fn crash_points_recover_a_committed_prefix_with_identical_hashes() -> TestResult {
    let folder = Folder::new("origin-crash");
    let path = folder.origin();
    let journal = path.with_extension("wal");
    let store = FileOriginStore::open(&path, &config(&path, 4))?;
    store.insert_run(initial_run(1))?;
    drive(&store, 1, 0..5)?;
    let journal_5 = std::fs::read(&journal)?;
    let checkpoint_4 = std::fs::read(&path)?;
    drive(&store, 1, 5..6)?;
    let journal_6 = std::fs::read(&journal)?;
    drive(&store, 1, 6..8)?;
    let checkpoint_8 = std::fs::read(&path)?;
    drop(store);
    let restore = |journal_bytes: &[u8], checkpoint: &[u8]| -> std::io::Result<()> {
        std::fs::write(&journal, journal_bytes)?;
        std::fs::write(&path, checkpoint)
    };

    // Crash before the sixth append: the command was never acknowledged, so
    // it is absent after restart and can be executed again.
    restore(&journal_5, &checkpoint_4)?;
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(5));
    let reopened = FileOriginStore::open(&path, &config(&path, 4))?;
    drive(&reopened, 1, 5..6)?;
    drop(reopened);
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(6));

    // Crash after the sixth append, before anything else.
    restore(&journal_6, &checkpoint_4)?;
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(6));

    // Torn sixth frame: the incomplete tail is cut off durably.
    restore(&journal_6[..journal_5.len() + 20], &checkpoint_4)?;
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(5));
    assert_eq!(std::fs::read(&journal)?, journal_5);

    // Crash while writing a checkpoint: the temporary file is ignored.
    restore(&journal_6, &checkpoint_4)?;
    std::fs::write(path.with_extension("tmp"), b"{\"partial")?;
    assert_eq!(recovered_hash(&path, 4)?, reference_hash(6));

    // A checkpoint ahead of the surviving journal fails closed.
    restore(&journal_5, &checkpoint_8)?;
    assert_eq!(recovered_hash(&path, 4), Err(OriginError::InvalidCommit));

    // A complete frame with a bad checksum fails closed.
    let mut corrupt = journal_6.clone();
    let last = corrupt.len() - 1;
    corrupt[last] ^= 1;
    restore(&corrupt, &checkpoint_4)?;
    assert_eq!(recovered_hash(&path, 4), Err(OriginError::InvalidCommit));
    Ok(())
}

#[test]
fn command_bounds_are_per_run_and_refusals_are_not_journaled() -> TestResult {
    let folder = Folder::new("origin-bounds");
    let path = folder.origin();
    let mut bounded = config(&path, 4);
    bounded.max_commands_per_run = 3;
    let store = FileOriginStore::open(&path, &bounded)?;
    store.insert_run(initial_run(1))?;
    store.insert_run(initial_run(2))?;
    for run_id in [1, 2] {
        drive(&store, run_id, 0..3)?;
        assert_eq!(
            drive(&store, run_id, 3..4),
            Err(OriginError::CapacityExceeded)
        );
    }
    let journal = std::fs::read(path.with_extension("wal"))?;
    drop(store);
    let reopened = FileOriginStore::open(&path, &bounded)?;
    assert_eq!(std::fs::read(path.with_extension("wal"))?, journal);
    assert_eq!(
        reopened.read_run(RunId::new(2), RunState::sequence)?,
        EventSequence::new(3)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn native_file_origin_enforces_single_writer_across_independent_opens() -> TestResult {
    let folder = Folder::new("origin-writer");
    let path = folder.origin();
    let first = FileOriginStore::open(&path, &config(&path, 4))?;
    first.insert_run(initial_run(1))?;
    assert!(FileOriginStore::open(&path, &config(&path, 4)).is_err());
    let clone = first.clone();
    drop(first);
    assert!(FileOriginStore::open(&path, &config(&path, 4)).is_err());
    drop(clone);
    let reopened = FileOriginStore::open(&path, &config(&path, 4))?;
    assert_eq!(
        reopened.read_run(RunId::new(1), RunState::sequence)?,
        EventSequence::new(0)
    );
    Ok(())
}

/// Step 3 acceptance: a one-million-command run neither hits a capacity
/// error nor grows per-command cost with history. Run with
/// `cargo test --release -p bunting-server --test durable_origin -- --ignored --nocapture`.
#[test]
#[ignore = "long-running acceptance check; run in release"]
fn million_command_run_commits_and_restarts() -> TestResult {
    const COMMANDS: u64 = 1_000_000;
    let folder = Folder::new("origin-million");
    let path = folder.origin();
    let configured = config(&path, 8_192);
    let store = FileOriginStore::open(&path, &configured)?;
    store.insert_run(initial_run(1))?;
    let started = std::time::Instant::now();
    let mut window = std::time::Instant::now();
    for chunk in 0..10 {
        let range = chunk * (COMMANDS / 10)..(chunk + 1) * (COMMANDS / 10);
        drive(&store, 1, range)?;
        println!(
            "commands {:>7}: {:>6.1} us/command over the last 100k",
            (chunk + 1) * (COMMANDS / 10),
            window.elapsed().as_secs_f64() * 10.0
        );
        window = std::time::Instant::now();
    }
    let elapsed = started.elapsed();
    let hash = store.read_run(RunId::new(1), |state| state.state_hash().unwrap())?;
    drop(store);
    let journal_bytes = std::fs::metadata(path.with_extension("wal"))?.len();
    let reopened_at = std::time::Instant::now();
    assert_eq!(recovered_hash(&path, 8_192)?, hash);
    println!(
        "{COMMANDS} commands in {:.1}s; journal {} MiB ({} B/command); restart {:.1}s",
        elapsed.as_secs_f64(),
        journal_bytes / (1 << 20),
        journal_bytes / COMMANDS,
        reopened_at.elapsed().as_secs_f64()
    );
    Ok(())
}
