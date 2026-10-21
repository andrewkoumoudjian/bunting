#![allow(clippy::panic, clippy::unwrap_used)]

use bunting_market_events::{OrderKind, Side};
use bunting_market_types::{InstrumentId, MoneyMinor, ParticipantId, PriceTicks, QuantityLots};
use quarcc_execution_engine::command::ExecutionIntent;
use quarcc_execution_engine::ids::{ClientOrderId, IntentId, LocalOrderId, ReportId, VenueOrderId};
use quarcc_execution_engine::lifecycle::OrderLifecycle;
use quarcc_execution_engine::normalized_report::{NormalizedVenueReport, VenueReportKind};
use quarcc_execution_engine::order::DesiredOrder;
use quarcc_execution_engine::positions::AuthoritativePosition;
use quarcc_execution_engine::reconciliation::{AuthoritativeOpenOrder, AuthoritativeVenueSnapshot};
use quarcc_execution_engine::{
    ExecutionAction, ExecutionActionBuffer, ExecutionConfig, ExecutionEngine, QuarccExecutionEngine,
};

fn desired(client: u128) -> DesiredOrder {
    DesiredOrder {
        client_order_id: ClientOrderId::new(client),
        instrument_id: InstrumentId::new(11),
        participant_id: ParticipantId::new(22),
        side: Side::Buy,
        quantity: QuantityLots::new(10),
        kind: OrderKind::Limit {
            price: PriceTicks::new(100),
        },
        anonymous: false,
    }
}

fn submit(engine: &mut QuarccExecutionEngine, client: u128) -> LocalOrderId {
    let mut output = ExecutionActionBuffer::with_limit(16);
    engine
        .submit_intent(
            ExecutionIntent::Submit {
                intent_id: IntentId::new(client),
                order: desired(client),
            },
            &mut output,
        )
        .unwrap();
    match &output.as_slice()[0] {
        ExecutionAction::Submit { local_order_id, .. } => *local_order_id,
        other => panic!("unexpected action: {other:?}"),
    }
}

fn report(id: u128, client: u128, venue: &str, kind: VenueReportKind) -> NormalizedVenueReport {
    NormalizedVenueReport {
        report_id: ReportId::new(id),
        source_sequence: u64::try_from(id).ok(),
        client_order_id: Some(ClientOrderId::new(client)),
        local_order_id: None,
        venue_order_id: Some(VenueOrderId::new(venue)),
        leaves_quantity: None,
        kind,
    }
}

#[test]
fn fill_before_ack_and_duplicate_reports_are_idempotent() {
    let mut engine = QuarccExecutionEngine::new(ExecutionConfig::default());
    let local = submit(&mut engine, 1);
    let mut output = ExecutionActionBuffer::with_limit(16);
    let fill = report(
        2,
        1,
        "V1",
        VenueReportKind::Fill {
            last_quantity: QuantityLots::new(4),
            cumulative_quantity: QuantityLots::new(4),
            price: PriceTicks::new(101),
        },
    );
    engine.apply_venue_report(&fill, &mut output).unwrap();
    engine.apply_venue_report(&fill, &mut output).unwrap();
    engine
        .apply_venue_report(&report(3, 1, "V1", VenueReportKind::Accepted), &mut output)
        .unwrap();
    let order = engine.order(local).unwrap();
    assert_eq!(order.lifecycle, OrderLifecycle::PartiallyFilled);
    assert_eq!(order.filled_quantity, QuantityLots::new(4));
}

#[test]
fn snapshot_restore_and_replay_are_equivalent() {
    let mut uninterrupted = QuarccExecutionEngine::new(ExecutionConfig::default());
    submit(&mut uninterrupted, 7);
    let mut restored = QuarccExecutionEngine::restore(uninterrupted.snapshot()).unwrap();
    let reports = [
        report(10, 7, "V7", VenueReportKind::Accepted),
        report(
            11,
            7,
            "V7",
            VenueReportKind::Fill {
                last_quantity: QuantityLots::new(10),
                cumulative_quantity: QuantityLots::new(10),
                price: PriceTicks::new(100),
            },
        ),
    ];
    for venue_report in reports {
        let mut left = ExecutionActionBuffer::with_limit(16);
        let mut right = ExecutionActionBuffer::with_limit(16);
        uninterrupted
            .apply_venue_report(&venue_report, &mut left)
            .unwrap();
        restored
            .apply_venue_report(&venue_report, &mut right)
            .unwrap();
        assert_eq!(left, right);
    }
    assert_eq!(uninterrupted.snapshot(), restored.snapshot());
}

#[test]
fn report_permutations_converge_for_ack_and_cumulative_fills() {
    let permutations = [[0, 1, 2], [1, 0, 2], [1, 2, 0], [0, 2, 1]];
    let reports = [
        report(20, 9, "V9", VenueReportKind::Accepted),
        report(
            21,
            9,
            "V9",
            VenueReportKind::Fill {
                last_quantity: QuantityLots::new(4),
                cumulative_quantity: QuantityLots::new(4),
                price: PriceTicks::new(100),
            },
        ),
        report(
            22,
            9,
            "V9",
            VenueReportKind::Fill {
                last_quantity: QuantityLots::new(6),
                cumulative_quantity: QuantityLots::new(10),
                price: PriceTicks::new(100),
            },
        ),
    ];
    let mut snapshots = Vec::new();
    for permutation in permutations {
        let mut engine = QuarccExecutionEngine::new(ExecutionConfig::default());
        submit(&mut engine, 9);
        let mut output = ExecutionActionBuffer::with_limit(16);
        for index in permutation {
            engine
                .apply_venue_report(&reports[index], &mut output)
                .unwrap();
        }
        snapshots.push(engine.snapshot());
    }
    for snapshot in &snapshots[1..] {
        assert_eq!(snapshot.orders, snapshots[0].orders);
        assert_eq!(snapshot.positions, snapshots[0].positions);
    }
}

#[test]
fn reconciliation_discovers_orders_and_uses_authoritative_positions() {
    let mut engine = QuarccExecutionEngine::new(ExecutionConfig::default());
    let snapshot = AuthoritativeVenueSnapshot {
        committed_sequence: 50,
        open_orders: vec![AuthoritativeOpenOrder {
            client_order_id: Some(ClientOrderId::new(30)),
            venue_order_id: VenueOrderId::new("V30"),
            order: desired(30),
            filled_quantity: QuantityLots::new(2),
        }],
        positions: vec![AuthoritativePosition {
            instrument_id: InstrumentId::new(11),
            quantity: QuantityLots::new(200),
            average_price: Some(PriceTicks::new(99)),
            realized_pnl: MoneyMinor::new(12),
        }],
    };
    let mut output = ExecutionActionBuffer::with_limit(16);
    engine.reconcile(&snapshot, &mut output).unwrap();
    assert_eq!(
        engine
            .order_by_client(ClientOrderId::new(30))
            .unwrap()
            .lifecycle,
        OrderLifecycle::ExternallyDiscovered
    );
    assert_eq!(
        engine.snapshot().positions[&InstrumentId::new(11)].quantity,
        QuantityLots::new(200)
    );
}

#[test]
fn a_long_session_keeps_bounded_order_report_and_intent_memory() {
    let mut engine = QuarccExecutionEngine::new(ExecutionConfig {
        max_orders: 4,
        max_seen_reports: 4,
        ..ExecutionConfig::default()
    });
    let mut output = ExecutionActionBuffer::with_limit(16);
    // Twenty orders, each accepted and then canceled by the venue: four
    // times the order memory and ten times the report memory.
    for client in 1..=20_u128 {
        let local = submit(&mut engine, client);
        let venue = local.get().to_string();
        let accepted = report(client * 2, client, &venue, VenueReportKind::Accepted);
        let canceled = report(client * 2 + 1, client, &venue, VenueReportKind::Cancelled);
        engine.apply_venue_report(&accepted, &mut output).unwrap();
        engine.apply_venue_report(&canceled, &mut output).unwrap();
        // A report the window has forgotten still counts as seen.
        engine.apply_venue_report(&accepted, &mut output).unwrap();
        let snapshot = engine.snapshot();
        assert!(
            snapshot.orders.len() <= 4,
            "{} orders",
            snapshot.orders.len()
        );
        assert!(snapshot.seen_reports.len() <= 4);
        assert!(snapshot.processed_intents.len() <= 4);
        assert_eq!(
            snapshot.orders.get(&local).map(|order| order.lifecycle),
            Some(OrderLifecycle::Cancelled)
        );
        // A restart restores what a long session saved.
        QuarccExecutionEngine::restore(snapshot).unwrap();
    }
    // An old intent replayed after its ID left the window is still a
    // duplicate: nothing is sent again.
    let mut replay = ExecutionActionBuffer::with_limit(16);
    engine
        .submit_intent(
            ExecutionIntent::Submit {
                intent_id: IntentId::new(1),
                order: desired(1),
            },
            &mut replay,
        )
        .unwrap();
    assert!(replay.as_slice().is_empty());
}

#[test]
fn the_open_order_limit_refuses_before_anything_is_sent() {
    let mut engine = QuarccExecutionEngine::new(ExecutionConfig {
        max_orders: 2,
        ..ExecutionConfig::default()
    });
    submit(&mut engine, 1);
    submit(&mut engine, 2);
    let before = engine.snapshot();
    let mut output = ExecutionActionBuffer::with_limit(16);
    let refused = engine
        .submit_intent(
            ExecutionIntent::Submit {
                intent_id: IntentId::new(3),
                order: desired(3),
            },
            &mut output,
        )
        .unwrap_err();
    assert!(refused.is_pre_trade_refusal());
    assert!(output.as_slice().is_empty());
    assert_eq!(engine.snapshot(), before);
}

#[test]
fn a_cancel_rejected_because_the_order_already_filled_waits_for_the_fill() {
    // The venue matched the order against an incoming taker while our
    // cancel was in flight: the cancel is rejected as unknown before the
    // fill report reaches us. Seen live on the practice venue, where the
    // reject used to stop the whole runtime.
    let mut engine = QuarccExecutionEngine::new(ExecutionConfig::default());
    let local = submit(&mut engine, 1);
    let mut output = ExecutionActionBuffer::with_limit(16);
    engine
        .apply_venue_report(&report(2, 1, "V1", VenueReportKind::Accepted), &mut output)
        .unwrap();
    engine
        .submit_intent(
            ExecutionIntent::Cancel {
                intent_id: IntentId::new(10),
                client_order_id: ClientOrderId::new(1),
            },
            &mut output,
        )
        .unwrap();
    assert_eq!(
        engine.order(local).unwrap().lifecycle,
        OrderLifecycle::PendingCancel
    );
    let rejected = report(
        3,
        1,
        "V1",
        VenueReportKind::Rejected {
            reason: "UnknownOrder".to_owned(),
        },
    );
    engine.apply_venue_report(&rejected, &mut output).unwrap();
    assert_eq!(engine.order(local).unwrap().lifecycle, OrderLifecycle::Live);
    let fill = report(
        4,
        1,
        "V1",
        VenueReportKind::Fill {
            last_quantity: QuantityLots::new(10),
            cumulative_quantity: QuantityLots::new(10),
            price: PriceTicks::new(100),
        },
    );
    engine.apply_venue_report(&fill, &mut output).unwrap();
    let order = engine.order(local).unwrap();
    assert_eq!(order.lifecycle, OrderLifecycle::Filled);
    assert_eq!(order.filled_quantity, QuantityLots::new(10));
}

fn fill(id: u128, last: i64, cumulative: i64) -> NormalizedVenueReport {
    report(
        id,
        1,
        "V1",
        VenueReportKind::Fill {
            last_quantity: QuantityLots::new(last),
            cumulative_quantity: QuantityLots::new(cumulative),
            price: PriceTicks::new(100),
        },
    )
}

/// A live order with a cancel in flight, as the reconcile path leaves it.
fn live_then_cancelling() -> (QuarccExecutionEngine, LocalOrderId) {
    let mut engine = QuarccExecutionEngine::new(ExecutionConfig::default());
    let local = submit(&mut engine, 1);
    let mut output = ExecutionActionBuffer::with_limit(16);
    engine
        .apply_venue_report(&report(2, 1, "V1", VenueReportKind::Accepted), &mut output)
        .unwrap();
    engine
        .submit_intent(
            ExecutionIntent::Cancel {
                intent_id: IntentId::new(10),
                client_order_id: ClientOrderId::new(1),
            },
            &mut output,
        )
        .unwrap();
    (engine, local)
}

fn apply(engine: &mut QuarccExecutionEngine, report: &NormalizedVenueReport) {
    let mut output = ExecutionActionBuffer::with_limit(16);
    engine.apply_venue_report(report, &mut output).unwrap();
}

#[test]
fn a_fill_committed_before_a_cancel_may_arrive_after_it() {
    let (mut engine, local) = live_then_cancelling();
    apply(&mut engine, &report(3, 1, "V1", VenueReportKind::Cancelled));
    apply(&mut engine, &fill(4, 4, 4));
    let order = engine.order(local).unwrap();
    assert_eq!(order.lifecycle, OrderLifecycle::Cancelled);
    assert_eq!(order.filled_quantity, QuantityLots::new(4));
    // The position follows the fill.
    assert_eq!(
        engine
            .snapshot()
            .positions
            .values()
            .map(|position| position.quantity.get().abs())
            .sum::<i64>(),
        4
    );
}

#[test]
fn reports_about_a_finished_order_change_nothing() {
    for finish in [
        fill(3, 10, 10),
        report(3, 1, "V1", VenueReportKind::Cancelled),
    ] {
        let (mut engine, local) = live_then_cancelling();
        apply(&mut engine, &finish);
        let settled = engine.order(local).unwrap().lifecycle;
        let reports = [
            report(5, 1, "V1", VenueReportKind::Cancelled),
            report(
                6,
                1,
                "V1",
                VenueReportKind::Rejected {
                    reason: "UnknownOrder".to_owned(),
                },
            ),
            report(
                7,
                1,
                "V1",
                VenueReportKind::CancelRejected {
                    reason: "UnknownOrder".to_owned(),
                },
            ),
            report(8, 1, "V1", VenueReportKind::Expired),
            report(9, 1, "V1", VenueReportKind::Accepted),
        ];
        for each in &reports {
            apply(&mut engine, each);
            // Applied twice: duplicates are harmless too.
            apply(&mut engine, each);
        }
        assert_eq!(engine.order(local).unwrap().lifecycle, settled);
    }
}

#[test]
fn a_cancel_reject_after_the_fill_is_not_an_error() {
    let (mut engine, local) = live_then_cancelling();
    // The fill reaches us first this time, then the cancel's reject.
    apply(&mut engine, &fill(3, 10, 10));
    apply(
        &mut engine,
        &report(
            4,
            1,
            "V1",
            VenueReportKind::Rejected {
                reason: "UnknownOrder".to_owned(),
            },
        ),
    );
    assert_eq!(
        engine.order(local).unwrap().lifecycle,
        OrderLifecycle::Filled
    );
}
