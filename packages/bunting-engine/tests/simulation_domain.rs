#![allow(clippy::too_many_lines, clippy::unwrap_used)]

use bunting_engine::simulation::{
    EconomicInstrument, FacilityDefinition, FacilityKind, InstrumentKind, LogicalClock,
    RunLifecycle, SIMULATION_POLICY_VERSION, ScheduledAction, ScheduledActionKind,
    SimulationScenario,
};
use bunting_engine::{
    ListingDefinition, ParticipantDefinition, PublishScenarioOutcome, RunState, ScenarioCatalog,
    ScenarioDefinition,
};
use bunting_ledger::TransactionKind;
use bunting_market_events::{
    AdvancedOrderPolicy, ClockMode, Command, CommandPayload, CompositeLeg, CompositePolicy,
    NewsAudience, OrderKind, OtcDecision, Side, SimulationCommand, SimulationCommandRequest,
    SubmitOrder, TenderDecision, TimeInForcePolicy,
};
use bunting_market_types::{
    CommandId, CorrelationId, CurrencyId, EventSequence, FacilityId, InstrumentId, IterationId,
    ListingKey, LogicalTimeNs, MoneyMinor, NegotiationId, NewsId, OrderId, ParticipantId,
    PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId, ScenarioVersion, TenderId, VenueId,
};
use bunting_risk_engine::RiskLimits;
use std::collections::BTreeMap;

const RUN: RunId = RunId::new(11);
const ADMIN: ParticipantId = ParticipantId::new(99);
const PARTICIPANT: ParticipantId = ParticipantId::new(1);
const COUNTERPARTY: ParticipantId = ParticipantId::new(2);
const INSTRUMENT: InstrumentId = InstrumentId::new(7);
const CURRENCY: CurrencyId = CurrencyId::new(1);

fn participant(id: ParticipantId) -> ParticipantDefinition {
    ParticipantDefinition::new(
        id,
        true,
        RiskLimits {
            max_order_quantity: QuantityLots::new(1_000),
            max_open_order_quantity: QuantityLots::new(10_000),
            max_absolute_position: QuantityLots::new(10_000),
        },
        MoneyMinor::new(1_000_000),
        BTreeMap::from([(INSTRUMENT, QuantityLots::new(1_000))]),
    )
}

fn scenario() -> ScenarioDefinition {
    let simulation = SimulationScenario {
        policy_version: SIMULATION_POLICY_VERSION,
        clock: LogicalClock {
            now: LogicalTimeNs::new(0),
            step_ns: 1_000_000,
            mode: ClockMode::Lockstep,
        },
        instruments: BTreeMap::from([(
            INSTRUMENT,
            EconomicInstrument {
                instrument_id: INSTRUMENT,
                symbol: "BNT".into(),
                settlement_currency: CURRENCY,
                kind: InstrumentKind::Equity,
                contract_multiplier: 1,
            },
        )]),
        facilities: BTreeMap::from([(
            FacilityId::new(1),
            FacilityDefinition {
                facility_id: FacilityId::new(1),
                kind: FacilityKind::Conversion,
                capacity: QuantityLots::new(100),
                input_instrument: Some(INSTRUMENT),
                output_instrument: Some(INSTRUMENT),
            },
        )]),
        scheduled_actions: vec![ScheduledAction {
            action_id: 1,
            effective_at: LogicalTimeNs::new(2_000_000),
            kind: ScheduledActionKind::Cashflow {
                participant_id: PARTICIPANT,
                currency_id: CURRENCY,
                amount: MoneyMinor::new(25),
                kind: TransactionKind::Dividend,
            },
        }],
        initial_news: Vec::new(),
    };
    ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [ListingDefinition::new(
            ListingKey::new(VenueId::new(1), INSTRUMENT),
            "BNT".into(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(10_000)).unwrap(),
        )
        .unwrap()],
        [
            participant(PARTICIPANT),
            participant(COUNTERPARTY),
            participant(ADMIN),
        ],
    )
    .unwrap()
    .with_simulation(simulation)
    .unwrap()
}

fn command(
    sequence: u64,
    logical_time: u64,
    actor: ParticipantId,
    payload: SimulationCommand,
) -> SimulationCommandRequest {
    SimulationCommandRequest {
        run_id: RUN,
        command_id: CommandId::new(u128::from(sequence) + 1),
        correlation_id: CorrelationId::new(1),
        logical_time: LogicalTimeNs::new(logical_time),
        expected_sequence: EventSequence::new(sequence),
        actor,
        payload,
    }
}

fn apply(state: &RunState, command: &SimulationCommandRequest) -> RunState {
    state.transition_simulation(command).unwrap().candidate
}

#[test]
fn lifecycle_scheduled_cashflow_snapshot_and_replay_are_equal() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    assert_eq!(initial.simulation().lifecycle, RunLifecycle::Stopped);
    let commands = [
        command(0, 0, ADMIN, SimulationCommand::StartRun),
        command(
            1,
            0,
            ADMIN,
            SimulationCommand::SetPacing {
                mode: ClockMode::Accelerated {
                    max_steps_per_advance: 4,
                },
                reason: "deterministic test acceleration".into(),
            },
        ),
        command(2, 0, ADMIN, SimulationCommand::Advance { steps: 2 }),
    ];
    let uninterrupted = commands
        .iter()
        .fold(initial.clone(), |state, command| apply(&state, command));
    assert_eq!(
        uninterrupted.simulation().clock.now,
        LogicalTimeNs::new(2_000_000)
    );
    assert_eq!(
        uninterrupted.simulation().portfolio_ledger.journal().len(),
        1
    );
    assert_eq!(
        uninterrupted
            .simulation()
            .portfolio_ledger
            .balance(PARTICIPANT, CURRENCY)
            .settled,
        MoneyMinor::new(1_000_025)
    );
    let envelope = uninterrupted.snapshot_envelope().unwrap();
    let restored = bunting_engine::EngineSnapshotEnvelope::from_json(&envelope.to_json().unwrap())
        .unwrap()
        .state;
    assert_eq!(restored, uninterrupted);
    let replayed = commands
        .iter()
        .fold(initial, |state, command| apply(&state, command));
    assert_eq!(
        replayed.state_hash().unwrap(),
        uninterrupted.state_hash().unwrap()
    );
    let reset = uninterrupted
        .reset_iteration(IterationId::new(2), &scenario())
        .unwrap();
    let fresh = RunState::from_scenario(RUN, IterationId::new(2), &scenario()).unwrap();
    assert_eq!(reset.state_hash().unwrap(), fresh.state_hash().unwrap());
}

#[test]
fn invalid_paused_advance_rolls_back_every_component() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let active = apply(&initial, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    let paused = apply(&active, &command(1, 0, ADMIN, SimulationCommand::PauseRun));
    let before = paused.state_hash().unwrap();
    assert!(
        paused
            .transition_simulation(&command(
                2,
                0,
                ADMIN,
                SimulationCommand::Advance { steps: 1 },
            ))
            .is_err()
    );
    assert_eq!(paused.state_hash().unwrap(), before);
    assert_eq!(paused.simulation().clock.now, LogicalTimeNs::new(0));
}

#[test]
fn news_tender_otc_composite_facility_and_scoring_share_one_sequence() {
    let mut state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let commands = vec![
        command(0, 0, ADMIN, SimulationCommand::StartRun),
        command(
            1,
            0,
            ADMIN,
            SimulationCommand::PublishNews {
                news_id: NewsId::new(1),
                audience: NewsAudience::Participant(PARTICIPANT),
                headline: "Private allocation".into(),
                body: "Participant-only fact".into(),
            },
        ),
        command(
            2,
            0,
            ADMIN,
            SimulationCommand::OpenTender {
                tender_id: TenderId::new(1),
                participant_id: PARTICIPANT,
                instrument_id: INSTRUMENT,
                side: Side::Buy,
                quantity: QuantityLots::new(5),
                price: PriceTicks::new(100),
                expires_at: LogicalTimeNs::new(10_000_000),
            },
        ),
        command(
            3,
            0,
            PARTICIPANT,
            SimulationCommand::DecideTender {
                tender_id: TenderId::new(1),
                decision: TenderDecision::Accept,
            },
        ),
        command(
            4,
            0,
            PARTICIPANT,
            SimulationCommand::OpenOtc {
                negotiation_id: NegotiationId::new(1),
                counterparty_id: COUNTERPARTY,
                instrument_id: INSTRUMENT,
                side: Side::Sell,
                quantity: QuantityLots::new(2),
                price: PriceTicks::new(101),
                expires_at: LogicalTimeNs::new(10_000_000),
            },
        ),
        command(
            5,
            0,
            COUNTERPARTY,
            SimulationCommand::DecideOtc {
                negotiation_id: NegotiationId::new(1),
                decision: OtcDecision::Accept,
            },
        ),
        command(
            6,
            0,
            PARTICIPANT,
            SimulationCommand::SubmitComposite {
                policy: CompositePolicy::AllOrNone,
                minimum_fill: QuantityLots::new(1),
                legs: vec![CompositeLeg {
                    instrument_id: INSTRUMENT,
                    side: Side::Buy,
                    quantity: QuantityLots::new(1),
                    limit_price: PriceTicks::new(100),
                }],
            },
        ),
        command(
            7,
            0,
            ADMIN,
            SimulationCommand::ScheduleFacilityJob {
                facility_id: FacilityId::new(1),
                participant_id: PARTICIPANT,
                input_quantity: QuantityLots::new(5),
                output_quantity: QuantityLots::new(4),
                completes_at: LogicalTimeNs::new(1_000_000),
            },
        ),
        command(8, 0, ADMIN, SimulationCommand::Advance { steps: 1 }),
        command(9, 1_000_000, ADMIN, SimulationCommand::ScoreIteration),
    ];
    for command in &commands {
        state = apply(&state, command);
    }
    assert_eq!(state.sequence(), EventSequence::new(10));
    assert_eq!(
        state.simulation().private[&PARTICIPANT].news,
        vec![NewsId::new(1)]
    );
    assert!(
        state
            .simulation()
            .private
            .get(&COUNTERPARTY)
            .is_none_or(|view| view.news.is_empty())
    );
    assert_eq!(
        state.simulation().tenders[&TenderId::new(1)].status,
        "accepted"
    );
    assert_eq!(
        state.simulation().otc[&NegotiationId::new(1)].status,
        "accepted"
    );
    assert!(
        state
            .simulation()
            .facility_jobs
            .values()
            .all(|job| job.completed)
    );
    assert_eq!(
        state
            .simulation()
            .portfolio_ledger
            .position(PARTICIPANT, INSTRUMENT)
            .settled,
        QuantityLots::new(999)
    );
    assert_eq!(state.simulation().reports.len(), 1);
}

#[test]
fn checked_in_simulation_fixture_is_strict_and_versioned() {
    let fixture: SimulationScenario =
        serde_json::from_str(include_str!("../../../scenarios/simulation-domain.v1.json")).unwrap();
    fixture.validate().unwrap();
    assert_eq!(fixture.policy_version, SIMULATION_POLICY_VERSION);
    assert_eq!(fixture.instruments.len(), 2);
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../scenarios/simulation-domain.v1.json")).unwrap();
    value["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<SimulationScenario>(value).is_err());
}

#[test]
fn fine_policy_posts_an_exact_balanced_cash_debit() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let before = initial
        .simulation()
        .portfolio_ledger
        .balance(PARTICIPANT, CURRENCY)
        .settled;
    let fined = apply(
        &initial,
        &command(
            0,
            0,
            ADMIN,
            SimulationCommand::ApplyFine {
                participant_id: PARTICIPANT,
                currency_id: CURRENCY,
                amount: MoneyMinor::new(125),
                reason: "late disclosure".to_owned(),
            },
        ),
    );
    assert_eq!(
        fined
            .simulation()
            .portfolio_ledger
            .balance(PARTICIPANT, CURRENCY)
            .settled,
        before.checked_sub(MoneyMinor::new(125)).unwrap()
    );
    assert_eq!(
        fined.simulation().portfolio_ledger.journal()[0].kind,
        TransactionKind::Fine
    );
}

#[test]
fn scenario_publication_is_immutable_versioned_and_idempotent() {
    let definition = scenario();
    let hash = definition.content_hash().unwrap();
    let mut catalog = ScenarioCatalog::default();
    assert_eq!(
        catalog.publish(definition.clone()).unwrap(),
        PublishScenarioOutcome::Published
    );
    assert_eq!(
        catalog.publish(definition).unwrap(),
        PublishScenarioOutcome::AlreadyPublished
    );
    assert_eq!(catalog.list().len(), 1);
    assert_eq!(
        catalog
            .get(ScenarioId::new(1), ScenarioVersion::new(1))
            .unwrap()
            .content_hash,
        hash
    );
}

#[test]
fn released_post_only_policy_is_matched_and_replayable() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let active = apply(&initial, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    let submit = Command {
        run_id: RUN,
        command_id: CommandId::new(2),
        correlation_id: CorrelationId::new(1),
        logical_time: LogicalTimeNs::new(0),
        expected_sequence: EventSequence::new(1),
        actor: PARTICIPANT,
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(1),
            participant_id: PARTICIPANT,
            instrument_id: INSTRUMENT,
            side: Side::Buy,
            quantity: QuantityLots::new(10),
            kind: OrderKind::AdvancedLimit {
                price: PriceTicks::new(100),
                time_in_force: TimeInForcePolicy::Gtc,
                policy: AdvancedOrderPolicy::PostOnly,
            },
        }),
    };
    let state = active.transition(&submit, None).unwrap().candidate;
    assert_eq!(
        state.ownership()[&OrderId::new(1)].remaining_quantity,
        QuantityLots::new(10)
    );
    let restored = bunting_engine::EngineSnapshotEnvelope::from_json(
        &state.snapshot_envelope().unwrap().to_json().unwrap(),
    )
    .unwrap()
    .state;
    assert_eq!(restored.state_hash().unwrap(), state.state_hash().unwrap());
}
