#![allow(clippy::unwrap_used)]

use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_application::{
    ApplicationService, FixApplicationRequest, FixApplicationState, FixCommandContext,
    VerifiedActor, prepare_authenticated,
};
use bunting_command_transaction::InMemorySnapshotCache;
use bunting_engine::{ListingDefinition, ParticipantDefinition, RunState, ScenarioDefinition};
use bunting_market_events::{Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, IterationId, ListingKey, LogicalTimeNs,
    MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId,
    ScenarioVersion, VenueId,
};
use bunting_origin_store::{CommitOutcome, InMemoryOrigin, OriginStore};
use bunting_risk_engine::RiskLimits;
use bunting_server::config::{StorageConfig, StorageKind};
use bunting_server::storage::FileOriginStore;
use quarcc_execution_engine::ExecutionConfig;
use simfix_wire::FixMessage;
use std::collections::BTreeMap;

fn initial_run() -> RunState {
    let scenario = ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [ListingDefinition::new(
            ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
            "ONE".to_owned(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
        )
        .unwrap()],
        [ParticipantDefinition::new(
            ParticipantId::new(7),
            true,
            RiskLimits {
                max_order_quantity: QuantityLots::new(100),
                max_open_order_quantity: QuantityLots::new(1_000),
                max_absolute_position: QuantityLots::new(1_000),
            },
            MoneyMinor::new(100_000),
            BTreeMap::new(),
        )],
    )
    .unwrap();
    RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap()
}

fn actor() -> VerifiedActor {
    VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(7),
        role: ActorRole::Participant,
        participant_id: Some(UnsignedDecimalString::new(7)),
        team_id: None,
    })
    .unwrap()
}

fn expected_command() -> Command {
    Command {
        run_id: RunId::new(1),
        command_id: CommandId::new(2),
        correlation_id: CorrelationId::new(44),
        logical_time: LogicalTimeNs::new(55),
        expected_sequence: EventSequence::new(0),
        actor: ParticipantId::new(7),
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(1),
            instrument_id: InstrumentId::new(1),
            participant_id: ParticipantId::new(7),
            side: Side::Buy,
            quantity: QuantityLots::new(3),
            kind: OrderKind::Limit {
                price: PriceTicks::new(101),
            },
        }),
    }
}

#[test]
fn native_fix_and_worker_prepare_commit_identical_authoritative_state()
-> Result<(), Box<dyn std::error::Error>> {
    let worker_origin = InMemoryOrigin::new();
    worker_origin.insert_run(initial_run()).unwrap();
    let initial = worker_origin.load_run(RunId::new(1)).unwrap();
    let command = expected_command();
    let prepared = prepare_authenticated(&actor(), &command, &initial, None).unwrap();
    assert!(matches!(
        worker_origin.commit(prepared.commit).unwrap(),
        CommitOutcome::Committed(_)
    ));

    let native_origin = InMemoryOrigin::new();
    native_origin.insert_run(initial_run()).unwrap();
    let cache = InMemorySnapshotCache::new();
    let mut fix = FixApplicationState::new(ExecutionConfig::default());
    let mut message = FixMessage::new("D");
    for (tag, value) in [
        (11, "1"),
        (48, "1"),
        (54, "1"),
        (38, "3"),
        (40, "2"),
        (44, "101"),
    ] {
        message.push(tag, value);
    }
    let mapped = fix
        .map_message(
            &message,
            &FixCommandContext {
                actor: ParticipantId::new(7),
                run_id: RunId::new(1),
                expected_sequence: EventSequence::new(0),
                logical_time: LogicalTimeNs::new(55),
                correlation_id: CorrelationId::new(44),
            },
        )
        .unwrap();
    let FixApplicationRequest::Command(fix_command) = mapped else {
        return Err("FIX command expected".into());
    };
    assert_eq!(fix_command, command);
    let native_execution = ApplicationService::new(&native_origin, &cache)
        .execute(&actor(), &fix_command)
        .unwrap();
    let reports = fix
        .committed_messages(ParticipantId::new(7), &native_execution.events)
        .unwrap();
    assert!(reports.iter().all(|report| report.value(11) == Some("1")));

    assert_eq!(
        native_origin.load_run(RunId::new(1)).unwrap(),
        worker_origin.load_run(RunId::new(1)).unwrap()
    );
    Ok(())
}

#[test]
fn durable_local_origin_restores_committed_state_after_restart()
-> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join(format!(
        "bunting-origin-restart-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    ));
    let config = StorageConfig {
        kind: StorageKind::File,
        path: Some(path.display().to_string()),
        max_runs: 4,
        max_commands: 64,
        max_events_per_run: 256,
    };
    let store = FileOriginStore::open(&path, &config)?;
    store.insert_run(initial_run())?;
    let cache = InMemorySnapshotCache::new();
    let executed =
        ApplicationService::new(&store, &cache).execute(&actor(), &expected_command())?;
    drop(store);

    let restored = FileOriginStore::open(&path, &config)?;
    assert_eq!(restored.load_run(RunId::new(1))?, executed.state);
    assert_eq!(restored.events(RunId::new(1))?, executed.events);
    std::fs::remove_file(path)?;
    Ok(())
}
