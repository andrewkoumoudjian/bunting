#![allow(clippy::unwrap_used)]

use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_application::{ApplicationService, VerifiedActor};
use bunting_command_transaction::InMemorySnapshotCache;
use bunting_engine::{ListingDefinition, ParticipantDefinition, RunState, ScenarioDefinition};
use bunting_market_events::{Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, IterationId, ListingKey, LogicalTimeNs,
    MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId,
    ScenarioVersion, VenueId,
};
use bunting_origin_store::OriginStore;
use bunting_risk_engine::RiskLimits;
use bunting_server::config::{StorageConfig, StorageKind};
use bunting_server::storage::TursoOriginStore;
use std::collections::BTreeMap;

fn temp_db(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "bunting-{name}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn config(path: &std::path::Path) -> StorageConfig {
    StorageConfig {
        kind: StorageKind::Turso,
        path: Some(path.display().to_string()),
        max_runs: 4,
        max_commands: 64,
        max_events_per_run: 256,
    }
}

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

fn command() -> Command {
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
fn turso_restart_recovers_authoritative_state_and_resting_orders()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("turso-restart");
    let storage = config(&path);
    let store = TursoOriginStore::open(&path, &storage)?;
    store.insert_run(initial_run())?;
    let executed = ApplicationService::new(&store, &InMemorySnapshotCache::new())
        .execute(&actor(), &command())?;
    assert_eq!(
        executed
            .state
            .simulation()
            .private
            .get(&ParticipantId::new(7))
            .map(|projection| projection.live_orders.len()),
        Some(1)
    );
    drop(store);

    let restored = TursoOriginStore::open(&path, &storage)?;
    assert_eq!(restored.load_run(RunId::new(1))?, executed.state);
    assert_eq!(restored.events(RunId::new(1))?, executed.events);
    assert_eq!(
        restored
            .load_run(RunId::new(1))?
            .simulation()
            .private
            .get(&ParticipantId::new(7))
            .map(|projection| projection.live_orders.len()),
        Some(1)
    );
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn turso_restart_preserves_command_idempotency()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("turso-idempotency");
    let storage = config(&path);
    let store = TursoOriginStore::open(&path, &storage)?;
    store.insert_run(initial_run())?;
    let first = ApplicationService::new(&store, &InMemorySnapshotCache::new())
        .execute(&actor(), &command())?;
    drop(store);

    let restored = TursoOriginStore::open(&path, &storage)?;
    let duplicate = ApplicationService::new(&restored, &InMemorySnapshotCache::new())
        .execute(&actor(), &command())?;
    assert!(duplicate.duplicate);
    assert_eq!(duplicate.result, first.result);
    assert_eq!(duplicate.state, first.state);
    std::fs::remove_file(path)?;
    Ok(())
}

#[test]
fn incompatible_turso_file_fails_explicitly_instead_of_reinitializing()
-> Result<(), Box<dyn std::error::Error>> {
    let path = temp_db("turso-invalid");
    std::fs::write(&path, b"not a turso database")?;
    let storage = config(&path);
    assert!(TursoOriginStore::open(&path, &storage).is_err());
    assert_eq!(std::fs::read(&path)?, b"not a turso database");
    std::fs::remove_file(path)?;
    Ok(())
}
