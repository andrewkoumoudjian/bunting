//! Exploration probe: per-command state cost as the resting book grows.
//!
//! The native writer path currently pays, per committed command, roughly:
//! one `RunState` clone (`OriginStore::load_run`), one engine transition, and
//! one JSON serialization of the complete candidate `RunState` inside the
//! `BUNTWAL1` journal record (`CommitRequest::candidate`). This probe measures
//! those three costs in isolation at several resting-order counts so roadmap
//! Slice 2 decisions can start from a number instead of a source reading.
//!
//! It is not a benchmark: single run, no warmup control, wall-clock timing,
//! and it ignores FIX, fsync and the server's second `recover()` clone.
//!
//! ```text
//! cargo run --release -p bunting-engine --example state_cost_probe
//! ```

use bunting_engine::simulation::{LogicalClock, SIMULATION_POLICY_VERSION, SimulationScenario};
use bunting_engine::{
    EngineError, InstrumentDefinition, InstrumentKind, ListingDefinition, ParticipantDefinition,
    RunState, ScenarioDefinition,
};
use bunting_market_events::{ClockMode, Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, InstrumentId, IterationId, ListingKey, LogicalTimeNs,
    MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId,
    ScenarioVersion, VenueId,
};
use bunting_risk_engine::RiskLimits;
use std::collections::BTreeMap;
use std::time::Instant;

const RUN: RunId = RunId::new(1);
const BUYER: ParticipantId = ParticipantId::new(1);
const INSTRUMENT: InstrumentId = InstrumentId::new(7);
const CURRENCY: CurrencyId = CurrencyId::new(1);
const CHECKPOINTS: [u64; 4] = [1_000, 10_000, 50_000, 100_000];

fn scenario() -> Result<ScenarioDefinition, Box<dyn std::error::Error>> {
    let huge = QuantityLots::new(1_000_000_000);
    let participant = ParticipantDefinition::new(
        BUYER,
        true,
        RiskLimits::new(huge, huge, huge),
        BTreeMap::from([(CURRENCY, MoneyMinor::new(1_000_000_000_000_000))]),
        BTreeMap::new(),
    );
    let listing = ListingDefinition::new(
        ListingKey::new(VenueId::new(1), INSTRUMENT),
        "BNT".into(),
        PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000_000))
            .map_err(|error| format!("{error:?}"))?,
    )
    .map_err(|error| format!("{error:?}"))?;
    let simulation = SimulationScenario {
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
    };
    Ok(ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [InstrumentDefinition::new(
            INSTRUMENT,
            "BNT",
            CURRENCY,
            InstrumentKind::Equity,
        )],
        [listing],
        [participant],
    )
    .map_err(|error| format!("{error:?}"))?
    .with_simulation(simulation)
    .map_err(|error| format!("{error:?}"))?)
}

/// One non-crossing resting bid; prices cycle so levels stay bounded.
fn resting_bid(state: &RunState, n: u64) -> Command {
    Command {
        run_id: RUN,
        command_id: CommandId::new(u128::from(n) + 1),
        correlation_id: CorrelationId::new(1),
        logical_time: LogicalTimeNs::new(0),
        expected_sequence: state.sequence(),
        actor: BUYER,
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(u128::from(n) + 1),
            instrument_id: INSTRUMENT,
            participant_id: BUYER,
            side: Side::Buy,
            quantity: QuantityLots::new(1),
            kind: OrderKind::Limit {
                price: PriceTicks::new(i64::try_from(n % 1_000).unwrap_or(0) + 1),
            },
        }),
    }
}

fn submit(state: RunState, n: u64) -> Result<RunState, String> {
    let command = resting_bid(&state, n);
    let outcome = state
        .transition_owned(&command)
        .map_err(|error: EngineError| format!("order {n}: {error}"))?;
    if !outcome.accepted {
        return Err(format!("order {n} rejected: {:?}", outcome.reject_code));
    }
    Ok(outcome.candidate)
}

fn micros(started: Instant) -> u128 {
    started.elapsed().as_micros()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()?)
        .map_err(|error| format!("{error:?}"))?;
    let mut submitted = 0_u64;
    println!("resting_orders,clone_us,transition_us,state_json_bytes,to_json_us,state_hash_us");
    for checkpoint in CHECKPOINTS {
        while submitted < checkpoint {
            state = submit(state, submitted)?;
            submitted += 1;
        }
        let started = Instant::now();
        let copy = state.clone();
        let clone_us = micros(started);

        let started = Instant::now();
        let advanced = submit(copy, submitted)?;
        let transition_us = micros(started);
        drop(advanced);

        let started = Instant::now();
        let json = serde_json::to_vec(&state)?;
        let to_json_us = micros(started);

        let started = Instant::now();
        let _hash = state.state_hash().map_err(|error| format!("{error:?}"))?;
        let state_hash_us = micros(started);

        println!(
            "{checkpoint},{clone_us},{transition_us},{},{to_json_us},{state_hash_us}",
            json.len()
        );
    }
    Ok(())
}
