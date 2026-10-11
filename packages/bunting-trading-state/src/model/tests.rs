use super::*;
use crate::ticket::{OrderTicket, TicketKind};
use bunting_client::bnp_wire::{Entry, Level, OpenOrder, Role, Stamp};

const NEAR: Listing = Listing {
    venue_id: 1,
    instrument_id: 1,
};
const FAR: Listing = Listing {
    venue_id: 2,
    instrument_id: 1,
};

fn welcome(resume: ResumeStatus) -> Welcome {
    Welcome {
        version: 1,
        run_id: 1,
        participant_id: 1,
        role: Role::Participant,
        heartbeat_ms: 1_000,
        max_frame_bytes: 65_536,
        resume,
        run_sequence: 10,
    }
}

fn stamp(sequence: u64) -> Stamp {
    Stamp {
        sequence,
        logical_time_ns: sequence * 1_000,
    }
}

fn message(model: &mut Model, message: ServerMessage, now: u64) -> Vec<ClientMessage> {
    model.apply(SessionEvent::Message(message), now)
}

/// A connected model with both listings subscribed; returns the feed
/// request IDs for NEAR and FAR.
fn connected() -> (Model, u32, u32) {
    let mut model = Model::new(1_000);
    let requests = model.apply(SessionEvent::Connected(welcome(ResumeStatus::Live)), 0);
    assert!(matches!(requests[0], ClientMessage::ListingsRequest { .. }));
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, ClientMessage::OpenOrdersRequest { .. }))
    );
    assert!(
        requests
            .iter()
            .any(|r| matches!(r, ClientMessage::AccountRequest { .. }))
    );
    let subscribe = message(
        &mut model,
        ServerMessage::Listings {
            request_id: 1,
            listings: vec![
                ListingInfo {
                    listing: FAR,
                    symbol: "BNT".to_owned(),
                },
                ListingInfo {
                    listing: NEAR,
                    symbol: "BNT".to_owned(),
                },
            ],
        },
        1,
    );
    let ids: Vec<(u32, Listing)> = subscribe
        .iter()
        .filter_map(|request| match request {
            ClientMessage::Subscribe {
                request_id,
                listing,
                ..
            } => Some((*request_id, *listing)),
            _ => None,
        })
        .collect();
    assert_eq!(ids.len(), 2);
    let near = ids.iter().find(|(_, l)| *l == NEAR).map(|(id, _)| *id);
    let far = ids.iter().find(|(_, l)| *l == FAR).map(|(id, _)| *id);
    (model, near.unwrap_or(0), far.unwrap_or(0))
}

fn snapshot(request_id: u32, listing: Listing, bid: i64, ask: i64) -> ServerMessage {
    ServerMessage::MarketSnapshot {
        request_id,
        listing,
        next_report: 1,
        bids: vec![Level {
            price: bid,
            quantity: 10,
        }],
        asks: vec![Level {
            price: ask,
            quantity: 10,
        }],
    }
}

#[test]
fn listings_group_by_instrument_and_every_venue_gets_a_feed() {
    let (mut model, near, far) = connected();
    let instruments = model.instruments();
    assert_eq!(instruments.len(), 1);
    assert_eq!(
        instruments[0]
            .listings
            .iter()
            .map(|info| info.listing.venue_id)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(
        model.feed(NEAR).map(|f| &f.state),
        Some(&FeedState::Requested)
    );
    message(&mut model, snapshot(near, NEAR, 99, 101), 2);
    message(&mut model, snapshot(far, FAR, 98, 102), 3);
    assert_eq!(model.feed(NEAR).map(|f| &f.state), Some(&FeedState::Live));
    assert_eq!(
        model
            .feed(FAR)
            .and_then(|f| f.book.best_ask())
            .map(|l| l.price),
        Some(102)
    );
}

#[test]
fn trades_reach_the_blotter_with_their_arrival_time_and_a_gap_resubscribes() {
    let (mut model, near, _) = connected();
    message(&mut model, snapshot(near, NEAR, 99, 101), 2);
    let update = |first_report, entries| ServerMessage::MarketUpdate {
        request_id: near,
        listing: NEAR,
        first_report,
        entries,
    };
    let none = message(
        &mut model,
        update(
            1,
            vec![
                Entry {
                    kind: EntryKind::Trade,
                    price: 101,
                    quantity: 4,
                },
                Entry {
                    kind: EntryKind::Ask,
                    price: 101,
                    quantity: 6,
                },
            ],
        ),
        50,
    );
    assert!(none.is_empty());
    assert_eq!(
        model.prints().back().copied(),
        Some(TradePrint {
            listing: NEAR,
            price: 101,
            quantity: 4,
            received_us: 50,
        })
    );
    // Reports 1 and 2 applied; 4 means 3 was missed.
    let again = message(&mut model, update(4, Vec::new()), 60);
    assert!(matches!(
        again.as_slice(),
        [
            ClientMessage::Unsubscribe { request_id },
            ClientMessage::Subscribe { listing: NEAR, .. }
        ] if *request_id == near
    ));
    let feed = model.feed(NEAR);
    assert_eq!(feed.map(|f| f.gaps), Some(1));
    assert_eq!(feed.map(|f| &f.state), Some(&FeedState::Requested));
}

#[test]
fn an_order_runs_its_lifecycle_and_times_the_venue_round_trip() -> Result<(), TicketError> {
    let (mut model, _, _) = connected();
    let mut ticket = OrderTicket::new(Some(FAR), Side::Buy);
    ticket.quantity = 10;
    ticket.price = 100;
    let ClientMessage::NewOrder(order) = model.place(&ticket, 1_000)? else {
        return Err(TicketError::NoListing);
    };
    assert_eq!(order.client_order_id, 1_000);
    assert_eq!(model.orders()[0].status, OrderStatus::PendingNew);
    message(
        &mut model,
        ServerMessage::OrderAccepted {
            stamp: stamp(11),
            client_order_id: 1_000,
            order_id: 77,
            listing: FAR,
            side: Side::Buy,
            quantity: 10,
            price: Some(100),
        },
        5_200,
    );
    assert_eq!(model.orders()[0].answer_us, Some(4_200));
    assert_eq!(
        model.venue_round_trips().get(&2).map(|v| v.last_us),
        Some(4_200)
    );
    message(
        &mut model,
        ServerMessage::Fill {
            stamp: stamp(12),
            client_order_id: 1_000,
            order_id: 77,
            listing: FAR,
            side: Side::Buy,
            price: 100,
            quantity: 4,
            fee: 2,
            liquidity: Liquidity::Taker,
        },
        5_300,
    );
    message(
        &mut model,
        ServerMessage::OrderRested {
            stamp: stamp(12),
            client_order_id: 1_000,
            order_id: 77,
            price: 100,
            remaining: 6,
        },
        5_300,
    );
    let row = &model.orders()[0];
    assert_eq!((row.filled, row.remaining), (4, 6));
    assert_eq!(row.status, OrderStatus::Resting);
    assert_eq!(model.unreported_position(1), 4);
    // The fill makes the account stale; the tick reloads it once the
    // start-up request has been answered.
    assert!(model.tick(5_400).is_empty());
    message(
        &mut model,
        ServerMessage::Account {
            request_id: 3,
            run_sequence: 10,
            cash: Vec::new(),
            positions: Vec::new(),
        },
        5_500,
    );
    assert!(matches!(
        model.tick(300_000).as_slice(),
        [ClientMessage::AccountRequest { .. }]
    ));
    // Cancel, then the venue's answer.
    assert_eq!(
        model.cancel(1_000),
        Some(ClientMessage::CancelOrder {
            client_order_id: 1_000
        })
    );
    assert_eq!(model.cancel(1_000), None, "a cancel is already in flight");
    message(
        &mut model,
        ServerMessage::OrderCanceled {
            stamp: stamp(13),
            client_order_id: 1_000,
            order_id: 77,
            remaining: 6,
            reason: CancelReason::Requested,
        },
        6_000,
    );
    assert_eq!(
        model.orders()[0].status,
        OrderStatus::Canceled(CancelReason::Requested)
    );
    assert_eq!(model.live_orders().count(), 0);
    Ok(())
}

#[test]
fn rejects_and_send_failures_end_pending_orders() -> Result<(), TicketError> {
    let (mut model, _, _) = connected();
    let mut ticket = OrderTicket::new(Some(NEAR), Side::Sell);
    ticket.kind = TicketKind::Market;
    let first = model.place(&ticket, 10)?;
    let second = model.place(&ticket, 11)?;
    message(
        &mut model,
        ServerMessage::OrderRejected {
            stamp: stamp(11),
            client_order_id: 1_000,
            reason: RejectReason::InsufficientInventory,
        },
        20,
    );
    assert_eq!(
        model.orders()[0].status,
        OrderStatus::Rejected("insufficient inventory".to_owned())
    );
    model.send_failed(&second, "not connected", 21);
    assert!(matches!(
        &model.orders()[1].status,
        OrderStatus::Rejected(reason) if reason.contains("not connected")
    ));
    assert!(matches!(first, ClientMessage::NewOrder(_)));
    message(
        &mut model,
        ServerMessage::Reject {
            request_type: msg_type::NEW_ORDER,
            reference: 1_002,
            reason: "rate limit".to_owned(),
        },
        22,
    );
    assert!(
        model
            .notices()
            .iter()
            .any(|notice| notice.text == "order refused: rate limit")
    );
    Ok(())
}

#[test]
fn an_open_orders_snapshot_closes_orders_the_venue_no_longer_lists() -> Result<(), TicketError> {
    let (mut model, _, _) = connected();
    let mut ticket = OrderTicket::new(Some(NEAR), Side::Buy);
    ticket.price = 90;
    model.place(&ticket, 10)?;
    message(
        &mut model,
        ServerMessage::OrderAccepted {
            stamp: stamp(11),
            client_order_id: 1_000,
            order_id: 5,
            listing: NEAR,
            side: Side::Buy,
            quantity: 1,
            price: Some(90),
        },
        20,
    );
    // After a reconnect with a gap, the venue lists one FIX order and not
    // the BNP one, which ended while this client was away.
    model.apply(
        SessionEvent::Disconnected {
            reason: "reset".to_owned(),
        },
        30,
    );
    assert!(!model.is_connected());
    model.apply(SessionEvent::Connected(welcome(ResumeStatus::Gap)), 40);
    message(
        &mut model,
        ServerMessage::OpenOrders {
            request_id: 9,
            run_sequence: 20,
            orders: vec![OpenOrder {
                client_order_id: 0,
                order_id: 6,
                listing: FAR,
                side: Side::Sell,
                price: 120,
                original_quantity: 3,
                remaining_quantity: 2,
            }],
        },
        50,
    );
    assert_eq!(model.orders()[0].status, OrderStatus::Closed);
    let fix_order = &model.orders()[1];
    assert_eq!(fix_order.status, OrderStatus::Resting);
    assert_eq!((fix_order.filled, fix_order.remaining), (1, 2));
    assert_eq!(model.cancel(0), None, "FIX orders are canceled over FIX");
    Ok(())
}

#[test]
fn round_trips_keep_last_min_and_a_bounded_mean() {
    let mut model = Model::new(1);
    for micros in [300, 100, 200] {
        model.apply(SessionEvent::RoundTrip { micros }, 0);
    }
    let trips = model.round_trips();
    assert_eq!(
        (trips.last_us, trips.min_us, trips.max_us, trips.mean_us()),
        (Some(200), Some(100), Some(300), Some(200))
    );
    for _ in 0..100 {
        model.apply(SessionEvent::RoundTrip { micros: 50 }, 0);
    }
    assert_eq!(model.round_trips().samples(), ROUND_TRIP_WINDOW);
    assert_eq!(model.round_trips().mean_us(), Some(50));
}

#[test]
fn the_kill_switch_is_reported_by_the_venue_not_assumed() {
    let (mut model, _, _) = connected();
    assert!(matches!(
        model.kill_switch(1),
        ClientMessage::KillSwitch { .. }
    ));
    assert!(!model.kill_switch_active());
    message(
        &mut model,
        ServerMessage::KillSwitchActivated { stamp: stamp(12) },
        2,
    );
    assert!(model.kill_switch_active());
    assert_eq!(model.last_sequence(), 12);
    assert!(matches!(
        model.reset_kill_switch(3),
        ClientMessage::KillSwitchReset { .. }
    ));
    assert!(model.kill_switch_active());
    message(
        &mut model,
        ServerMessage::KillSwitchReset { stamp: stamp(13) },
        4,
    );
    assert!(!model.kill_switch_active());
}
