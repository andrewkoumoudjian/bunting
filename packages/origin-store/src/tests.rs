use super::*;
use bunting_engine::{ListingDefinition, ParticipantDefinition, ScenarioDefinition};
use bunting_market_events::{CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CorrelationId, CurrencyId, InstrumentId, IterationId, ListingKey, LogicalTimeNs, MoneyMinor,
    ParticipantId, PriceBounds, PriceTicks, QuantityLots, ScenarioId, ScenarioVersion, VenueId,
};
use bunting_risk_engine::RiskLimits;
use std::sync::Barrier;
use std::thread;

fn run() -> RunState {
    let participant = |id| {
        ParticipantDefinition::new(
            ParticipantId::new(id),
            true,
            RiskLimits::new(
                QuantityLots::new(100),
                QuantityLots::new(1_000),
                QuantityLots::new(1_000),
            ),
            BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(1_000_000))]),
            BTreeMap::from([(InstrumentId::new(1), QuantityLots::new(1_000))]),
        )
    };
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
            "ONE".to_string(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
        )
        .unwrap()],
        [participant(1), participant(2)],
    )
    .unwrap();
    RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap()
}

fn order(sequence: u64, id: u128, participant: u128, side: Side, price: i64) -> JournalInput {
    JournalInput::Command(Command {
        run_id: RunId::new(1),
        command_id: CommandId::new(id),
        correlation_id: CorrelationId::new(id),
        logical_time: LogicalTimeNs::new(u64::try_from(id).unwrap()),
        expected_sequence: EventSequence::new(sequence),
        actor: ParticipantId::new(participant),
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(id),
            instrument_id: InstrumentId::new(1),
            participant_id: ParticipantId::new(participant),
            side,
            quantity: QuantityLots::new(2),
            kind: OrderKind::Limit {
                price: PriceTicks::new(price),
            },
        }),
    })
}

/// Alternating resting and crossing orders, so records carry trades.
fn workload(count: u64) -> Vec<JournalInput> {
    (0..count)
        .map(|index| {
            let id = u128::from(index) + 1;
            if index % 2 == 0 {
                order(index, id, 1, Side::Sell, 100)
            } else {
                order(index, id, 2, Side::Buy, 100)
            }
        })
        .collect()
}

fn committed(live: &mut LiveRun, input: &JournalInput) -> CommandRecord {
    match live.execute(input).unwrap() {
        Execution::Committed(record) => Some(*record),
        Execution::Duplicate(_) => None,
    }
    .unwrap()
}

#[test]
fn same_expected_version_cannot_commit_twice() {
    let origin = InMemoryOrigin::new();
    origin.insert_run(run()).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let handles = [1, 2].map(|id| {
        let origin = origin.clone();
        let barrier = barrier.clone();
        thread::spawn(move || {
            barrier.wait();
            origin.execute(&order(0, id, 1, Side::Buy, 90))
        })
    });
    barrier.wait();
    let outcomes = handles.map(|handle| handle.join().unwrap());
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(executed) if !executed.duplicate))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Err(OriginError::VersionConflict { .. })))
            .count(),
        1
    );
}

#[test]
fn duplicates_replay_results_and_conflicts_change_nothing() {
    let origin = InMemoryOrigin::new();
    origin.insert_run(run()).unwrap();
    let first = origin.execute(&order(0, 7, 1, Side::Buy, 90)).unwrap();
    assert!(!first.duplicate && !first.events.is_empty());
    let replay = origin.execute(&order(0, 7, 1, Side::Buy, 90)).unwrap();
    assert!(replay.duplicate && replay.events.is_empty());
    assert_eq!(replay.result, first.result);
    let before = origin.clone_run(RunId::new(1)).unwrap();
    assert_eq!(
        origin.execute(&order(1, 7, 1, Side::Buy, 91)),
        Err(OriginError::IdempotencyConflict)
    );
    let mut unknown = order(1, 8, 1, Side::Buy, 90);
    if let JournalInput::Command(command) = &mut unknown {
        command.run_id = RunId::new(2);
    }
    assert_eq!(origin.execute(&unknown), Err(OriginError::UnknownRun));
    assert_eq!(origin.clone_run(RunId::new(1)).unwrap(), before);
    assert_eq!(
        origin
            .find_command(RunId::new(1), CommandId::new(7))
            .unwrap(),
        Some((
            order(0, 7, 1, Side::Buy, 90).fingerprint().unwrap(),
            first.result
        ))
    );
}

#[test]
fn bound_violations_roll_back_through_the_checkpoint_tail() {
    let limits = RunLimits {
        max_commands: 1_000,
        max_events: 12,
        checkpoint_interval: 3,
    };
    let mut live = LiveRun::genesis(run(), limits).unwrap();
    let inputs = workload(8);
    let mut index = 0;
    let rejected = loop {
        let before = live.state().unwrap().clone();
        let chain = live.chain();
        match live.execute(&inputs[index]) {
            Ok(_) => {
                if live.needs_checkpoint() {
                    live.checkpoint().unwrap();
                }
                index += 1;
            }
            Err(error) => {
                assert_eq!(live.state().unwrap(), &before);
                assert_eq!(live.chain(), chain);
                break error;
            }
        }
    };
    assert_eq!(rejected, OriginError::CapacityExceeded);
    assert!(index > 3, "the rollback must re-execute a non-empty tail");
    assert_eq!(live.find(inputs[index].command_id()), None);
}

#[test]
fn recovery_from_genesis_or_checkpoint_reproduces_state_and_chain() {
    let inputs = workload(9);
    let mut live = LiveRun::genesis(run(), RunLimits::default()).unwrap();
    let mut records = Vec::new();
    let mut midpoint = None;
    for (index, input) in inputs.iter().enumerate() {
        records.push(committed(&mut live, input));
        if index == 4 {
            midpoint = Some((live.state().unwrap().clone(), live.chain()));
        }
    }
    let (checkpoint, checkpoint_chain) = midpoint.unwrap();

    for start in [None, Some((checkpoint.clone(), checkpoint_chain.as_str()))] {
        let mut recovery = RunRecovery::new(run(), start, RunLimits::default()).unwrap();
        for record in &records {
            recovery.replay(record).unwrap();
        }
        let recovered = recovery.finish().unwrap();
        assert_eq!(recovered.state().unwrap(), live.state().unwrap());
        assert_eq!(recovered.chain(), live.chain());
        assert_eq!(recovered.command_count(), records.len());
    }

    // A journal that stops before the checkpoint is rejected.
    let mut short = RunRecovery::new(
        run(),
        Some((checkpoint.clone(), checkpoint_chain.as_str())),
        RunLimits::default(),
    )
    .unwrap();
    for record in &records[..3] {
        short.replay(record).unwrap();
    }
    assert_eq!(short.finish().unwrap_err(), OriginError::InvalidCommit);

    // A checkpoint whose chain disagrees with the journal is rejected.
    let wrong_chain = "0".repeat(64);
    let mut forged = RunRecovery::new(
        run(),
        Some((checkpoint, wrong_chain.as_str())),
        RunLimits::default(),
    )
    .unwrap();
    let outcome = records.iter().try_for_each(|record| forged.replay(record));
    assert_eq!(outcome, Err(OriginError::InvalidCommit));
}

#[test]
fn recovery_rejects_altered_records_before_and_after_the_checkpoint() {
    let inputs = workload(6);
    let mut live = LiveRun::genesis(run(), RunLimits::default()).unwrap();
    let records = inputs
        .iter()
        .map(|input| committed(&mut live, input))
        .collect::<Vec<_>>();
    let checkpoint_at = |index: usize| {
        let mut replayed = LiveRun::genesis(run(), RunLimits::default()).unwrap();
        for input in &inputs[..index] {
            committed(&mut replayed, input);
        }
        (replayed.state().unwrap().clone(), replayed.chain())
    };
    let (checkpoint, chain) = checkpoint_at(3);
    for tampered_index in [1, 4] {
        let mut altered = records.clone();
        altered[tampered_index].events[0].logical_time = LogicalTimeNs::new(999);
        let mut recovery = RunRecovery::new(
            run(),
            Some((checkpoint.clone(), chain.as_str())),
            RunLimits::default(),
        )
        .unwrap();
        let outcome = altered
            .iter()
            .try_for_each(|record| recovery.replay(record));
        assert_eq!(outcome, Err(OriginError::InvalidCommit));
    }
}

#[test]
fn reads_borrow_the_live_state_and_checkpoints_move_the_base() {
    let origin = InMemoryOrigin::with_limits(RunLimits {
        checkpoint_interval: 2,
        ..RunLimits::default()
    });
    origin.insert_run(run()).unwrap();
    for input in workload(5) {
        origin.execute(&input).unwrap();
    }
    assert_eq!(
        origin.read_run(RunId::new(1), RunState::sequence).unwrap(),
        EventSequence::new(5)
    );
    assert_eq!(
        origin.read_run(RunId::new(2), RunState::sequence),
        Err(OriginError::UnknownRun)
    );
    // Re-inserting the committed state is idempotent; any other state is not.
    let live = origin.clone_run(RunId::new(1)).unwrap();
    origin.insert_run(live).unwrap();
    assert_eq!(origin.insert_run(run()), Err(OriginError::InvalidCommit));
}
