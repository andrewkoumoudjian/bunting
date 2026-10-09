//! Measurement baseline for the command path (exploration-note Step 2).
//!
//! Replays the same seeded command streams through three layers and reports
//! per-command latency percentiles, throughput and bytes written:
//!
//! - `engine`: `RunState::apply` in place, nothing else;
//! - `memory`: `CommandTransaction` over `InMemoryOrigin` (writer-owned live
//!   run: idempotency index, event-hash chain, in-memory checkpoints);
//! - `file`: `CommandTransaction` over the durable `FileOriginStore` (adds
//!   one command-sourced journal frame + fdatasync per command and a
//!   state-only checkpoint every `CHECKPOINT_INTERVAL` commands).
//!
//! The byte column is the mean journal bytes per command (file layer only).
//!
//! Workloads follow the roadmap's measurement table: `passive` (90% resting
//! submits / 10% cancels), `mixed` (40% passive / 30% cancel / 30% crossing)
//! and `sweep` (one market order through 100 single-lot levels).
//!
//! ```text
//! cargo run --release --locked -p bunting-server --example command_path_baseline [commands]
//! ```
//!
//! Wall-clock single process, no warmup control, one run per cell: compare
//! layers and commits on the same machine, never against external figures.

use bunting_command_transaction::CommandTransaction;
use bunting_engine::simulation::{LogicalClock, SIMULATION_POLICY_VERSION, SimulationScenario};
use bunting_engine::{
    InstrumentDefinition, InstrumentKind, ListingDefinition, ParticipantDefinition, RunState,
    ScenarioDefinition,
};
use bunting_market_events::{
    CancelOrder, ClockMode, Command, CommandPayload, OrderKind, Side, SubmitOrder,
};
use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, EventSequence, InstrumentId, IterationId, ListingKey,
    LogicalTimeNs, MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots,
    RunId, ScenarioId, ScenarioVersion, VenueId,
};
use bunting_origin_store::{InMemoryOrigin, OriginStore};
use bunting_risk_engine::RiskLimits;
use bunting_server::config::{StorageConfig, StorageKind};
use bunting_server::storage::FileOriginStore;

/// Matches the shipped configuration profiles.
const CHECKPOINT_INTERVAL: usize = 8_192;
use std::collections::BTreeMap;
use std::time::Instant;

type Error = Box<dyn std::error::Error>;

const RUN: RunId = RunId::new(1);
const BUYER: ParticipantId = ParticipantId::new(1);
const SELLER: ParticipantId = ParticipantId::new(2);
const INSTRUMENT: InstrumentId = InstrumentId::new(1);
const CURRENCY: CurrencyId = CurrencyId::new(1);

fn initial_run() -> Result<RunState, Error> {
    let huge = QuantityLots::new(1_000_000_000);
    let participant = |id| {
        ParticipantDefinition::new(
            id,
            true,
            RiskLimits::new(huge, huge, huge),
            BTreeMap::from([(CURRENCY, MoneyMinor::new(1_000_000_000_000_000))]),
            BTreeMap::from([(INSTRUMENT, QuantityLots::new(1_000_000))]),
        )
    };
    let scenario = ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [
            InstrumentDefinition::new(INSTRUMENT, "BNT", CURRENCY, InstrumentKind::Equity)
                .with_opening_mark(PriceTicks::new(10_000)),
        ],
        [ListingDefinition::new(
            ListingKey::new(VenueId::new(1), INSTRUMENT),
            "BNT".into(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000_000))
                .map_err(|error| format!("{error:?}"))?,
        )
        .map_err(|error| format!("{error:?}"))?],
        [participant(BUYER), participant(SELLER)],
    )
    .map_err(|error| format!("{error:?}"))?
    .with_simulation(SimulationScenario {
        policy_version: SIMULATION_POLICY_VERSION,
        clock: LogicalClock {
            now: LogicalTimeNs::new(0),
            step_ns: 1_000_000,
            mode: ClockMode::Lockstep,
        },
        facilities: BTreeMap::new(),
        scheduled_actions: Vec::new(),
        initial_news: Vec::new(),
        starts_active: true,
    })
    .map_err(|error| format!("{error:?}"))?;
    Ok(RunState::from_scenario(RUN, IterationId::new(1), &scenario)
        .map_err(|error| format!("{error:?}"))?)
}

/// Deterministic 64-bit LCG (Knuth MMIX constants); no external RNG crate.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound.max(1)
    }
}

/// One command without its expected sequence, which each layer fills in.
#[derive(Clone)]
struct Step {
    actor: ParticipantId,
    payload: CommandPayload,
}

fn limit(order_id: u128, actor: ParticipantId, side: Side, price: i64, quantity: i64) -> Step {
    Step {
        actor,
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(order_id),
            instrument_id: INSTRUMENT,
            participant_id: actor,
            side,
            quantity: QuantityLots::new(quantity),
            kind: OrderKind::Limit {
                price: PriceTicks::new(price),
            },
        }),
    }
}

/// `passive_pct` resting submits, `cancel_pct` cancels, the rest crossing.
fn stream(commands: usize, passive_pct: u64, cancel_pct: u64, seed: u64) -> Vec<Step> {
    let mut rng = Lcg(seed);
    let mut live: Vec<(u128, ParticipantId)> = Vec::new();
    let mut steps = Vec::with_capacity(commands);
    for index in 0..commands {
        let order_id = u128::try_from(index).unwrap_or(u128::MAX) + 1;
        let roll = rng.next(100);
        let buy = rng.next(2) == 0;
        let (actor, side) = if buy {
            (BUYER, Side::Buy)
        } else {
            (SELLER, Side::Sell)
        };
        let offset = i64::try_from(rng.next(500)).unwrap_or(0) + 1;
        if roll < passive_pct || (roll < passive_pct + cancel_pct && live.is_empty()) {
            let price = if buy {
                10_000 - offset
            } else {
                10_000 + offset
            };
            live.push((order_id, actor));
            steps.push(limit(order_id, actor, side, price, 10));
        } else if roll < passive_pct + cancel_pct {
            let slot = usize::try_from(rng.next(live.len() as u64)).unwrap_or(0);
            let (target, owner) = live.swap_remove(slot);
            steps.push(Step {
                actor: owner,
                payload: CommandPayload::CancelOrder(CancelOrder {
                    order_id: OrderId::new(target),
                    participant_id: owner,
                }),
            });
        } else {
            // Crossing order at a price that reaches deep into the far side.
            let price = if buy { 10_000 + 600 } else { 10_000 - 600 };
            steps.push(limit(order_id, actor, side, price, 25));
        }
    }
    steps
}

/// Sweep: `levels` single-lot asks at distinct prices, then a market buy for
/// all of them; repeated `rounds` times.
fn sweep(levels: usize, rounds: usize) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut order_id = 0_u128;
    for _ in 0..rounds {
        for level in 0..levels {
            order_id += 1;
            let price = 10_000 + i64::try_from(level).unwrap_or(0);
            steps.push(limit(order_id, SELLER, Side::Sell, price, 1));
        }
        order_id += 1;
        steps.push(Step {
            actor: BUYER,
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(order_id),
                instrument_id: INSTRUMENT,
                participant_id: BUYER,
                side: Side::Buy,
                quantity: QuantityLots::new(i64::try_from(levels).unwrap_or(0)),
                kind: OrderKind::Market,
            }),
        });
    }
    steps
}

fn command(step: &Step, index: usize, expected: EventSequence) -> Command {
    let id = u128::try_from(index).unwrap_or(u128::MAX) + 1;
    Command {
        run_id: RUN,
        command_id: CommandId::new(id),
        correlation_id: CorrelationId::new(id),
        logical_time: LogicalTimeNs::new(u64::try_from(index).unwrap_or(0)),
        expected_sequence: expected,
        actor: step.actor,
        payload: step.payload.clone(),
    }
}

struct Sample {
    nanos: Vec<u128>,
    total_nanos: u128,
    /// Mean journal bytes appended per command (file layer only).
    journal_bytes_per_command: usize,
    resting_at_end: usize,
    rejects: BTreeMap<String, usize>,
}

fn percentile(sorted: &[u128], numerator: usize, denominator: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() - 1) * numerator / denominator;
    sorted[rank]
}

fn report(workload: &str, layer: &str, mut sample: Sample, only_last_of: Option<usize>) {
    if let Some(every) = only_last_of {
        // For sweeps, report only the market orders (every `every`-th step).
        sample.nanos = sample
            .nanos
            .chunks(every)
            .filter_map(|chunk| chunk.last().copied())
            .collect();
    }
    let count = sample.nanos.len();
    sample.nanos.sort_unstable();
    let micros = |nanos: u128| nanos / 1_000;
    let throughput = if sample.total_nanos == 0 {
        0
    } else {
        u128::try_from(count).unwrap_or(0) * 1_000_000_000 / sample.total_nanos
    };
    println!(
        "{workload:7} {layer:6} n={count:6} p50={:>8}us p99={:>8}us max={:>8}us  {throughput:>7} cmd/s  journal={:>6} B/cmd  resting_end={}",
        micros(percentile(&sample.nanos, 50, 100)),
        micros(percentile(&sample.nanos, 99, 100)),
        micros(sample.nanos.last().copied().unwrap_or(0)),
        sample.journal_bytes_per_command,
        sample.resting_at_end,
    );
    if !sample.rejects.is_empty() {
        println!("{:15} rejects: {:?}", "", sample.rejects);
    }
}

fn resting(state: &RunState) -> usize {
    state.live_orders(BUYER).count() + state.live_orders(SELLER).count()
}

fn run_engine(steps: &[Step]) -> Result<Sample, Error> {
    let mut state = initial_run()?;
    let mut rejects = BTreeMap::new();
    let mut nanos = Vec::with_capacity(steps.len());
    let started = Instant::now();
    for (index, step) in steps.iter().enumerate() {
        let command = command(step, index, state.sequence());
        let begin = Instant::now();
        let applied = state
            .apply(&command)
            .map_err(|error| format!("{error:?}"))?;
        nanos.push(begin.elapsed().as_nanos());
        if let Some(code) = applied.reject_code {
            *rejects.entry(code).or_insert(0) += 1;
        }
    }
    Ok(Sample {
        nanos,
        total_nanos: started.elapsed().as_nanos(),
        journal_bytes_per_command: 0,
        resting_at_end: resting(&state),
        rejects,
    })
}

fn run_transaction<O: OriginStore>(origin: &O, steps: &[Step]) -> Result<Sample, Error> {
    let transaction = CommandTransaction::new(origin);
    let mut expected = origin
        .read_run(RUN, RunState::sequence)
        .map_err(|error| format!("{error:?}"))?;
    let mut rejects = BTreeMap::new();
    let mut nanos = Vec::with_capacity(steps.len());
    let started = Instant::now();
    for (index, step) in steps.iter().enumerate() {
        let command = command(step, index, expected);
        let begin = Instant::now();
        let result = transaction
            .execute(&command)
            .map_err(|error| format!("{error:?}"))?;
        nanos.push(begin.elapsed().as_nanos());
        if let Some(code) = result.reject_code {
            *rejects.entry(code).or_insert(0) += 1;
        }
        expected = result.committed_sequence;
    }
    let total_nanos = started.elapsed().as_nanos();
    let resting_at_end = origin
        .read_run(RUN, resting)
        .map_err(|error| format!("{error:?}"))?;
    Ok(Sample {
        nanos,
        total_nanos,
        journal_bytes_per_command: 0,
        resting_at_end,
        rejects,
    })
}

fn run_file(steps: &[Step], label: &str) -> Result<Sample, Error> {
    let directory =
        std::env::temp_dir().join(format!("bunting-baseline-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("origin.json");
    let origin = FileOriginStore::open(
        &path,
        &StorageConfig {
            kind: StorageKind::File,
            path: Some(path.display().to_string()),
            max_runs: 1,
            max_commands_per_run: 10_000_000,
            max_events_per_run: 100_000_000,
            checkpoint_interval: CHECKPOINT_INTERVAL,
            max_commands: None,
        },
    )
    .map_err(|error| format!("{error:?}"))?;
    origin
        .insert_run(initial_run()?)
        .map_err(|error| format!("{error:?}"))?;
    let genesis_bytes = std::fs::metadata(path.with_extension("wal"))?.len();
    let mut sample = run_transaction(&origin, steps)?;
    let journal_bytes = std::fs::metadata(path.with_extension("wal"))?.len() - genesis_bytes;
    sample.journal_bytes_per_command = usize::try_from(journal_bytes)? / steps.len().max(1);
    drop(origin);
    let _ = std::fs::remove_dir_all(&directory);
    Ok(sample)
}

fn main() -> Result<(), Error> {
    let commands = std::env::args()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .unwrap_or(2_000_usize);
    println!(
        "command_path_baseline: {commands} commands per stream; release build expected; latency per command"
    );
    let workloads = [
        ("passive", stream(commands, 90, 10, 7)),
        ("mixed", stream(commands, 40, 30, 11)),
    ];
    for (name, steps) in &workloads {
        report(name, "engine", run_engine(steps)?, None);
        let memory = InMemoryOrigin::new();
        memory
            .insert_run(initial_run()?)
            .map_err(|error| format!("{error:?}"))?;
        report(name, "memory", run_transaction(&memory, steps)?, None);
        report(name, "file", run_file(steps, name)?, None);
    }
    let levels = 100;
    let steps = sweep(levels, 20);
    report("sweep", "engine", run_engine(&steps)?, Some(levels + 1));
    let memory = InMemoryOrigin::new();
    memory
        .insert_run(initial_run()?)
        .map_err(|error| format!("{error:?}"))?;
    report(
        "sweep",
        "memory",
        run_transaction(&memory, &steps)?,
        Some(levels + 1),
    );
    report(
        "sweep",
        "file",
        run_file(&steps, "sweep")?,
        Some(levels + 1),
    );
    Ok(())
}
