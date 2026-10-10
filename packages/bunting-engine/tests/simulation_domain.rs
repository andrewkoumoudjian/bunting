#![allow(
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::unwrap_used
)]

use bunting_engine::simulation::{
    CashflowKind, FacilityDefinition, FacilityKind, LogicalClock, RunLifecycle,
    SIMULATION_POLICY_VERSION, ScheduledAction, ScheduledActionKind, SimulationError,
    SimulationScenario,
};
use bunting_engine::{
    EngineError, InstrumentDefinition, InstrumentKind, ListingDefinition, ParticipantDefinition,
    PublishScenarioOutcome, RunState, ScenarioCatalog, ScenarioDefinition,
};
use bunting_market_events::{
    ClockMode, Command, CommandPayload, CompositeLeg, CompositePolicy, NewsAudience, OrderKind,
    OtcDecision, Side, SimulationCommand, SimulationCommandRequest, SubmitOrder, TenderDecision,
    TimeInForcePolicy,
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
        RiskLimits::new(
            QuantityLots::new(1_000),
            QuantityLots::new(10_000),
            QuantityLots::new(10_000),
        ),
        BTreeMap::from([(CURRENCY, MoneyMinor::new(1_000_000))]),
        BTreeMap::from([(INSTRUMENT, QuantityLots::new(1_000))]),
    )
}

fn simulation(starts_active: bool) -> SimulationScenario {
    SimulationScenario {
        policy_version: SIMULATION_POLICY_VERSION,
        clock: LogicalClock {
            now: LogicalTimeNs::new(0),
            step_ns: 1_000_000,
            mode: ClockMode::Lockstep,
        },
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
                kind: CashflowKind::Dividend,
            },
        }],
        initial_news: Vec::new(),
        starts_active,
    }
}

fn scenario() -> ScenarioDefinition {
    ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [
            InstrumentDefinition::new(INSTRUMENT, "BNT", CURRENCY, InstrumentKind::Equity)
                .with_opening_mark(PriceTicks::new(100)),
        ],
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
    .with_simulation(simulation(false))
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

fn cash(state: &RunState, participant: ParticipantId) -> MoneyMinor {
    state.ledger().cash(participant, CURRENCY).balance
}

fn limit(
    state: &RunState,
    actor: ParticipantId,
    order_id: u128,
    side: Side,
    price: i64,
    quantity: i64,
) -> Command {
    Command {
        run_id: RUN,
        command_id: CommandId::new(order_id),
        correlation_id: CorrelationId::new(order_id),
        logical_time: LogicalTimeNs::new(0),
        expected_sequence: state.sequence(),
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
            anonymous: false,
        }),
    }
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
        cash(&uninterrupted, PARTICIPANT),
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
fn orders_are_rejected_until_the_run_starts() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let outcome = initial
        .transition(&limit(&initial, PARTICIPANT, 1, Side::Buy, 100, 1))
        .unwrap();
    assert!(!outcome.accepted);
    assert_eq!(outcome.reject_code.as_deref(), Some("RunNotActive"));
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
fn news_tender_otc_facility_and_scoring_settle_through_one_ledger() {
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
            ADMIN,
            SimulationCommand::ScheduleFacilityJob {
                facility_id: FacilityId::new(1),
                participant_id: PARTICIPANT,
                input_quantity: QuantityLots::new(5),
                output_quantity: QuantityLots::new(4),
                completes_at: LogicalTimeNs::new(1_000_000),
            },
        ),
        command(7, 0, ADMIN, SimulationCommand::Advance { steps: 1 }),
        command(8, 1_000_000, ADMIN, SimulationCommand::ScoreIteration),
    ];
    for command in &commands {
        state = apply(&state, command);
    }
    assert_eq!(state.sequence(), EventSequence::new(9));
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
    // 1000 + 5 tender - 2 OTC - 5 facility input + 4 facility output.
    assert_eq!(
        state.ledger().position(PARTICIPANT, INSTRUMENT).quantity,
        QuantityLots::new(1_002)
    );
    assert_eq!(
        state.ledger().position(COUNTERPARTY, INSTRUMENT).quantity,
        QuantityLots::new(1_002)
    );
    assert_eq!(
        cash(&state, PARTICIPANT),
        MoneyMinor::new(1_000_000 - 500 + 202)
    );
    assert_eq!(cash(&state, COUNTERPARTY), MoneyMinor::new(1_000_000 - 202));
    let report = state.simulation().reports.last().unwrap();
    assert_eq!(report.entries.len(), 3);
    assert!(
        report
            .entries
            .iter()
            .all(|entry| entry.score == state.net_liquidation_value(entry.participant_id).unwrap())
    );
}

#[test]
fn unsupported_economic_actions_fail_closed() {
    let state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let active = apply(&state, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    let composite = command(
        1,
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
    );
    assert_eq!(
        active.transition_simulation(&composite).unwrap_err(),
        EngineError::Simulation(SimulationError::Unsupported)
    );
    let mut config = simulation(true);
    config.scheduled_actions.push(ScheduledAction {
        action_id: 2,
        effective_at: LogicalTimeNs::new(5),
        kind: ScheduledActionKind::Deliver {
            participant_id: PARTICIPANT,
            instrument_id: INSTRUMENT,
            quantity: QuantityLots::new(1),
        },
    });
    assert!(scenario().with_simulation(config).is_err());
}

#[test]
fn unfunded_tender_acceptance_is_rejected_without_mutation() {
    let state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let mut state = apply(&state, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    state = apply(
        &state,
        &command(
            1,
            0,
            ADMIN,
            SimulationCommand::OpenTender {
                tender_id: TenderId::new(9),
                participant_id: PARTICIPANT,
                instrument_id: INSTRUMENT,
                side: Side::Buy,
                quantity: QuantityLots::new(1_000_000),
                price: PriceTicks::new(100),
                expires_at: LogicalTimeNs::new(10),
            },
        ),
    );
    let before = state.state_hash().unwrap();
    let accept = command(
        2,
        0,
        PARTICIPANT,
        SimulationCommand::DecideTender {
            tender_id: TenderId::new(9),
            decision: TenderDecision::Accept,
        },
    );
    assert!(state.transition_simulation(&accept).is_err());
    assert_eq!(state.state_hash().unwrap(), before);
}

#[test]
fn checked_in_scenario_fixture_is_strict_and_versioned() {
    let fixture: ScenarioDefinition =
        serde_json::from_str(include_str!("../../../scenarios/simulation-domain.v2.json")).unwrap();
    fixture.validate().unwrap();
    assert_eq!(fixture.instruments().len(), 2);
    RunState::from_scenario(RUN, IterationId::new(1), &fixture).unwrap();
    let mut value: serde_json::Value =
        serde_json::from_str(include_str!("../../../scenarios/simulation-domain.v2.json")).unwrap();
    value["unknown"] = serde_json::json!(true);
    assert!(serde_json::from_value::<ScenarioDefinition>(value).is_err());
}

#[test]
fn fine_policy_debits_cash_exactly() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let before = cash(&initial, PARTICIPANT);
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
        cash(&fined, PARTICIPANT),
        before.checked_sub(MoneyMinor::new(125)).unwrap()
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
fn post_only_order_rests_and_replays() {
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
            kind: OrderKind::LimitWithPolicy {
                price: PriceTicks::new(100),
                time_in_force: TimeInForcePolicy::Gtc,
                post_only: true,
                display_quantity: None,
            },
            anonymous: false,
        }),
    };
    let state = active.transition(&submit).unwrap().candidate;
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

#[test]
fn opening_marks_value_endowments_and_fills_realize_actual_pnl() {
    let state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    assert_eq!(
        state.ledger().position(PARTICIPANT, INSTRUMENT).cost_basis,
        MoneyMinor::new(100_000)
    );
    assert_eq!(
        state.net_liquidation_value(PARTICIPANT).unwrap(),
        MoneyMinor::new(1_100_000)
    );
    let mut active = apply(&state, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    for (actor, id, side) in [
        (COUNTERPARTY, 101_u128, Side::Sell),
        (PARTICIPANT, 102_u128, Side::Buy),
    ] {
        let order = limit(&active, actor, id, side, 110, 2);
        active = active.transition(&order).unwrap().candidate;
    }
    let ledger = active.ledger();
    assert_eq!(
        ledger.position(COUNTERPARTY, INSTRUMENT).cost_basis,
        MoneyMinor::new(99_800)
    );
    assert_eq!(
        ledger.position(COUNTERPARTY, INSTRUMENT).realized_pnl,
        MoneyMinor::new(20)
    );
    // The fill is the instrument's mark for every holder immediately.
    assert_eq!(
        active.net_liquidation_value(COUNTERPARTY).unwrap(),
        MoneyMinor::new(1_000_220 + 998 * 110)
    );
    assert_eq!(
        active.net_liquidation_value(ADMIN).unwrap(),
        MoneyMinor::new(1_110_000)
    );
    assert_eq!(
        ledger.unrealized_pnl(ADMIN, INSTRUMENT).unwrap(),
        MoneyMinor::new(10_000)
    );
    let restored = bunting_engine::EngineSnapshotEnvelope::from_json(
        &active.snapshot_envelope().unwrap().to_json().unwrap(),
    )
    .unwrap();
    assert_eq!(
        restored.state.state_hash().unwrap(),
        active.state_hash().unwrap()
    );
}

#[test]
fn opening_positions_require_an_opening_mark() {
    let unmarked = ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [InstrumentDefinition::new(
            INSTRUMENT,
            "BNT",
            CURRENCY,
            InstrumentKind::Equity,
        )],
        [ListingDefinition::new(
            ListingKey::new(VenueId::new(1), INSTRUMENT),
            "BNT".into(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(10_000)).unwrap(),
        )
        .unwrap()],
        [participant(PARTICIPANT)],
    );
    assert!(unmarked.is_err());
}

#[test]
fn fills_fines_and_scores_reconcile_from_one_ledger() {
    let mut state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    state = apply(&state, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    let sell = limit(&state, COUNTERPARTY, 101, Side::Sell, 10, 2);
    state = state.transition(&sell).unwrap().candidate;
    let buy = limit(&state, PARTICIPANT, 102, Side::Buy, 10, 2);
    state = state.transition(&buy).unwrap().candidate;
    assert_eq!(cash(&state, PARTICIPANT), MoneyMinor::new(999_980));
    assert_eq!(cash(&state, COUNTERPARTY), MoneyMinor::new(1_000_020));
    assert_eq!(
        state.ledger().position(PARTICIPANT, INSTRUMENT).quantity,
        QuantityLots::new(1_002)
    );
    assert_eq!(
        state.ledger().position(COUNTERPARTY, INSTRUMENT).quantity,
        QuantityLots::new(998)
    );
    let version = state.sequence().get();
    state = apply(
        &state,
        &command(
            version,
            0,
            ADMIN,
            SimulationCommand::ApplyFine {
                participant_id: PARTICIPANT,
                currency_id: CURRENCY,
                amount: MoneyMinor::new(5),
                reason: "test fine".to_owned(),
            },
        ),
    );
    assert_eq!(cash(&state, PARTICIPANT), MoneyMinor::new(999_975));
    let score_version = state.sequence().get();
    state = apply(
        &state,
        &command(score_version, 0, ADMIN, SimulationCommand::ScoreIteration),
    );
    let report = state.simulation().reports.last().unwrap();
    assert_eq!(report.entries.len(), 3);
    // Every participant is valued at the last trade (10) after the fill.
    let participant_score = report
        .entries
        .iter()
        .find(|entry| entry.participant_id == PARTICIPANT)
        .unwrap();
    assert_eq!(
        participant_score.score,
        MoneyMinor::new(999_975 + 1_002 * 10)
    );
    let replay = bunting_engine::EngineSnapshotEnvelope::from_json(
        &state.snapshot_envelope().unwrap().to_json().unwrap(),
    )
    .unwrap();
    assert_eq!(
        replay.state.state_hash().unwrap(),
        state.state_hash().unwrap()
    );
}

const GOLDEN: &str = "competition-full-run.v2.json";

#[test]
fn full_competition_run_matches_ledger_score_and_transcript_golden() {
    let mut state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let commands = [
        command(0, 0, ADMIN, SimulationCommand::StartRun),
        command(
            1,
            0,
            ADMIN,
            SimulationCommand::ApplyFine {
                participant_id: PARTICIPANT,
                currency_id: CURRENCY,
                amount: MoneyMinor::new(125),
                reason: "late disclosure".to_owned(),
            },
        ),
        command(2, 0, ADMIN, SimulationCommand::Advance { steps: 1 }),
        command(3, 1_000_000, ADMIN, SimulationCommand::Advance { steps: 1 }),
        command(4, 2_000_000, ADMIN, SimulationCommand::ScoreIteration),
        command(
            5,
            2_000_000,
            ADMIN,
            SimulationCommand::Terminate {
                reason: "golden run complete".to_owned(),
            },
        ),
    ];
    let mut transcript = Vec::new();
    for command in commands {
        let outcome = state.transition_simulation(&command).unwrap();
        transcript.extend(
            outcome
                .events
                .iter()
                .map(|event| serde_json::to_value(event).unwrap()),
        );
        state = outcome.candidate;
    }
    let ledger = state
        .participants()
        .keys()
        .map(|participant| {
            serde_json::json!({
                "participant_id": participant,
                "cash": state.ledger().cash(*participant, CURRENCY),
                "position": state.ledger().position(*participant, INSTRUMENT),
            })
        })
        .collect::<Vec<_>>();
    let actual = serde_json::json!({
        "state_hash": state.state_hash().unwrap(),
        "sequence": state.sequence(),
        "lifecycle": state.simulation().lifecycle,
        "logical_time": state.simulation().clock.now,
        "ledger": ledger,
        "scores": state.simulation().reports,
        "transcript": transcript,
    });
    let path = format!(
        "{}/../../tests/goldens/{GOLDEN}",
        env!("CARGO_MANIFEST_DIR")
    );
    // Goldens are only ever regenerated from an actual replay, never edited by hand.
    if std::env::var_os("BUNTING_BLESS").is_some() {
        std::fs::write(&path, serde_json::to_string_pretty(&actual).unwrap() + "\n").unwrap();
    }
    let expected: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(actual, expected);
}

fn policy_order(
    state: &RunState,
    actor: ParticipantId,
    order_id: u128,
    side: Side,
    price: i64,
    quantity: i64,
    time_in_force: TimeInForcePolicy,
    post_only: bool,
    display: Option<i64>,
    logical_time: u64,
) -> Command {
    let mut command = limit(state, actor, order_id, side, price, quantity);
    command.logical_time = LogicalTimeNs::new(logical_time);
    if let CommandPayload::SubmitOrder(order) = &mut command.payload {
        order.kind = OrderKind::LimitWithPolicy {
            price: PriceTicks::new(price),
            time_in_force,
            post_only,
            display_quantity: display.map(QuantityLots::new),
        };
    }
    command
}

#[test]
fn post_only_rejects_crossing_and_iceberg_hides_reserve() {
    let state = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let mut state = apply(&state, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    let iceberg = policy_order(
        &state,
        COUNTERPARTY,
        1,
        Side::Sell,
        105,
        50,
        TimeInForcePolicy::Gtc,
        false,
        Some(10),
        0,
    );
    state = state.transition(&iceberg).unwrap().candidate;
    let key = ListingKey::new(VenueId::new(1), INSTRUMENT);
    assert_eq!(
        state.visible_levels(key).unwrap().1,
        vec![(PriceTicks::new(105), QuantityLots::new(10))]
    );
    // Reserve inventory covers the full iceberg, not only its display.
    assert_eq!(
        state.ledger().position(COUNTERPARTY, INSTRUMENT).reserved,
        QuantityLots::new(50)
    );
    let crossing = policy_order(
        &state,
        PARTICIPANT,
        2,
        Side::Buy,
        105,
        1,
        TimeInForcePolicy::Gtc,
        true,
        None,
        0,
    );
    let outcome = state.transition(&crossing).unwrap();
    assert!(!outcome.accepted);
    assert_eq!(outcome.reject_code.as_deref(), Some("PostOnlyWouldCross"));
    state = outcome.candidate;
    let sweep = limit(&state, PARTICIPANT, 3, Side::Buy, 105, 25);
    state = state.transition(&sweep).unwrap().candidate;
    assert_eq!(
        state.ledger().position(PARTICIPANT, INSTRUMENT).quantity,
        QuantityLots::new(1_025)
    );
    assert_eq!(
        state.visible_levels(key).unwrap().1,
        vec![(PriceTicks::new(105), QuantityLots::new(5))]
    );
}

#[test]
fn gtd_orders_expire_on_the_logical_clock_and_release_reservations() {
    let initial = RunState::from_scenario(RUN, IterationId::new(1), &scenario()).unwrap();
    let mut state = apply(&initial, &command(0, 0, ADMIN, SimulationCommand::StartRun));
    let gtd = policy_order(
        &state,
        PARTICIPANT,
        1,
        Side::Buy,
        90,
        10,
        TimeInForcePolicy::Gtd {
            expires_at: LogicalTimeNs::new(1_000_000),
        },
        false,
        None,
        0,
    );
    state = state.transition(&gtd).unwrap().candidate;
    assert_eq!(
        state.ledger().cash(PARTICIPANT, CURRENCY).reserved,
        MoneyMinor::new(900)
    );
    let already_expired = policy_order(
        &state,
        PARTICIPANT,
        3,
        Side::Buy,
        90,
        1,
        TimeInForcePolicy::Gtd {
            expires_at: LogicalTimeNs::new(0),
        },
        false,
        None,
        0,
    );
    assert_eq!(
        state
            .transition(&already_expired)
            .unwrap()
            .reject_code
            .as_deref(),
        Some("InvalidTimeInForce")
    );
    let advance = command(
        state.sequence().get(),
        0,
        ADMIN,
        SimulationCommand::Advance { steps: 1 },
    );
    let outcome = state.transition_simulation(&advance).unwrap();
    assert!(outcome.events.iter().any(|event| matches!(
        event.payload,
        bunting_market_events::EventPayload::OrderCanceled {
            order_id,
            reason: bunting_market_events::CancelReason::Expired,
            ..
        } if order_id == OrderId::new(1)
    )));
    state = outcome.candidate;
    assert_eq!(
        state.ledger().cash(PARTICIPANT, CURRENCY).reserved,
        MoneyMinor::new(0)
    );
    assert!(
        state
            .visible_levels(ListingKey::new(VenueId::new(1), INSTRUMENT))
            .unwrap()
            .0
            .is_empty()
    );
    let restored = bunting_engine::EngineSnapshotEnvelope::from_json(
        &state.snapshot_envelope().unwrap().to_json().unwrap(),
    )
    .unwrap();
    assert_eq!(restored.state, state);
}
