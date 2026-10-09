#![allow(clippy::unwrap_used)]

use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_application::{
    ApplicationService, FixApplicationRequest, FixApplicationState, FixCommandContext,
    VerifiedActor,
};
use bunting_engine::{ListingDefinition, ParticipantDefinition, RunState, ScenarioDefinition};
use bunting_market_events::{Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, EventSequence, InstrumentId, IterationId, ListingKey,
    LogicalTimeNs, MoneyMinor, OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots,
    RunId, ScenarioId, ScenarioVersion, VenueId,
};
use bunting_origin_store::{InMemoryOrigin, OriginStore};
use bunting_risk_engine::RiskLimits;
use quarcc_execution_engine::ExecutionConfig;
use simfix_wire::FixMessage;
use std::collections::BTreeMap;

fn initial_run() -> RunState {
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
        payload: CommandPayload::SubmitOrderAtListing {
            listing_key: ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
            order: SubmitOrder {
                order_id: OrderId::new(1),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(7),
                side: Side::Buy,
                quantity: QuantityLots::new(3),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(101),
                },
            },
        },
    }
}

#[test]
fn fix_mapped_and_canonical_commands_commit_identical_authoritative_state()
-> Result<(), Box<dyn std::error::Error>> {
    let native_origin = InMemoryOrigin::new();
    native_origin.insert_run(initial_run()).unwrap();
    let mut fix = FixApplicationState::new(ExecutionConfig::default());
    let mut message = FixMessage::new("D");
    for (tag, value) in [
        (11, "1"),
        (48, "1"),
        (207, "1"),
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
    // FIX-local IDs (command 2, order 1) are namespaced per participant
    // session so two sessions can never collide; everything else is identical.
    let mut command = expected_command();
    let CommandPayload::SubmitOrderAtListing { order, .. } = &mut command.payload else {
        return Err("listing order expected".into());
    };
    let CommandPayload::SubmitOrderAtListing {
        order: fix_order, ..
    } = &fix_command.payload
    else {
        return Err("FIX listing order expected".into());
    };
    assert!(fix_command.command_id.get() > u128::from(u64::MAX));
    assert_eq!(fix_command.command_id.get() & u128::from(u64::MAX), 2);
    assert!(fix_order.order_id.get() > u128::from(u64::MAX));
    assert_eq!(fix_order.order_id.get() & u128::from(u64::MAX), 1);
    order.order_id = fix_order.order_id;
    command.command_id = fix_command.command_id;
    assert_eq!(fix_command, command);
    let canonical_origin = InMemoryOrigin::new();
    canonical_origin.insert_run(initial_run()).unwrap();
    ApplicationService::new(&canonical_origin)
        .execute(&actor(), &command)
        .unwrap();
    let native_execution = ApplicationService::new(&native_origin)
        .execute(&actor(), &fix_command)
        .unwrap();
    let reports = fix
        .committed_messages(ParticipantId::new(7), &native_execution.events)
        .unwrap();
    assert!(reports.iter().all(|report| report.value(11) == Some("1")));

    assert_eq!(
        native_origin.clone_run(RunId::new(1)).unwrap(),
        canonical_origin.clone_run(RunId::new(1)).unwrap()
    );
    Ok(())
}
