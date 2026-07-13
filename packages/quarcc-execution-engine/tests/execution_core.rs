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
