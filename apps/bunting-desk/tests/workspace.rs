//! The workspace in a real, headless GPUI window (`gpui-kit` test support),
//! fed scripted venue messages through the shared trading model. Clicks go
//! through real hit testing; requests are recorded where the live app would
//! write them to the venue.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bunting_desk::{Desk, Outbox, PANELS, Workspace};
use bunting_trading_state::SessionEvent;
use bunting_trading_state::bunting_client::bnp_wire::{
    BookOrder, ClientMessage, EntryKind, Level, Listing, ListingInfo, OrderType, ResumeStatus,
    Role, ServerMessage, Side, Stamp, TapeEntry, TapeQuote, Welcome,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Point, TestAppContext, WindowBounds, WindowHandle,
    WindowOptions, base::Root, px, size,
};
use std::cell::RefCell;
use std::rc::Rc;

const NEAR: Listing = Listing {
    venue_id: 1,
    instrument_id: 7,
};
const FAR: Listing = Listing {
    venue_id: 2,
    instrument_id: 7,
};

#[derive(Clone, Default)]
struct Recorder(Rc<RefCell<Vec<ClientMessage>>>);

impl Outbox for Recorder {
    fn send(&self, message: &ClientMessage) -> Result<(), String> {
        self.0.borrow_mut().push(message.clone());
        Ok(())
    }
}

impl Recorder {
    fn take(&self) -> Vec<ClientMessage> {
        std::mem::take(&mut self.0.borrow_mut())
    }
}

struct Fixture {
    window: WindowHandle<Root>,
    desk: Entity<Desk>,
    sent: Recorder,
}

fn open(cx: &mut TestAppContext) -> Fixture {
    cx.update(gpui_kit::init);
    let sent = Recorder::default();
    let outbox = sent.clone();
    let desk =
        cx.update(|cx| cx.new(|_| Desk::with_outbox(Box::new(outbox), "test venue".to_owned())));
    let shared = desk.clone();
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1600.), px(1000.)),
            })),
            ..WindowOptions::default()
        };
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            cx.new(|cx| Workspace::new(shared, window, cx))
        })
        .expect("open window");
        window.downcast::<Root>().expect("Base Root")
    });
    Fixture { window, desk, sent }
}

fn apply(fixture: &Fixture, cx: &mut TestAppContext, event: SessionEvent) {
    fixture.desk.update(cx, |desk, cx| {
        let at = desk.now_us();
        desk.apply(event, at);
        cx.notify();
    });
}

/// Clicks an element in a fresh frame, then lets observers run.
fn click(fixture: &Fixture, cx: &mut TestAppContext, id: &str) {
    let id = id.to_owned();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

/// Connects, lists one instrument on two venues and gives each a book.
fn connect_with_books(fixture: &Fixture, cx: &mut TestAppContext) {
    apply(
        fixture,
        cx,
        SessionEvent::Connected(Welcome {
            version: 1,
            run_id: 1,
            participant_id: 42,
            role: Role::Participant,
            heartbeat_ms: 1_000,
            max_frame_bytes: 65_536,
            resume: ResumeStatus::Live,
            run_sequence: 0,
        }),
    );
    let listings_request = fixture
        .sent
        .take()
        .into_iter()
        .find_map(|message| match message {
            ClientMessage::ListingsRequest { request_id } => Some(request_id),
            _ => None,
        })
        .expect("a listings request on connect");
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::Listings {
            request_id: listings_request,
            listings: vec![
                ListingInfo {
                    listing: NEAR,
                    symbol: "BNT".to_owned(),
                },
                ListingInfo {
                    listing: FAR,
                    symbol: "BNT".to_owned(),
                },
            ],
        }),
    );
    let mut subscriptions = Vec::new();
    let mut tape = None;
    for message in fixture.sent.take() {
        if let ClientMessage::Subscribe {
            request_id,
            listing,
            ..
        } = message
        {
            // Venue 0 is the instrument's consolidated tape.
            if listing.venue_id == 0 {
                tape = Some(request_id);
            } else {
                subscriptions.push((request_id, listing));
            }
        }
    }
    assert_eq!(subscriptions.len(), 2, "one direct feed per venue");
    for (request_id, listing) in subscriptions {
        apply(
            fixture,
            cx,
            SessionEvent::Message(book_snapshot(request_id, listing)),
        );
    }
    send_tape(fixture, cx, tape.expect("a tape subscription"));
}

/// The near venue sends price levels; the far one order by order: two
/// offers at 102 (3 then 2) and a bid at 98.
fn book_snapshot(request_id: u32, listing: Listing) -> ServerMessage {
    if listing == NEAR {
        ServerMessage::MarketSnapshot {
            request_id,
            listing,
            next_report: 1,
            bids: vec![Level {
                price: 99,
                quantity: 10,
            }],
            asks: vec![Level {
                price: 101,
                quantity: 5,
            }],
        }
    } else {
        let order = |side, reference, price, quantity, queue_position| BookOrder {
            side,
            reference,
            price,
            quantity,
            broker: Some(17),
            queue_position,
        };
        ServerMessage::OrderSnapshot {
            request_id,
            listing,
            next_report: 1,
            complete: true,
            orders: vec![
                order(Side::Sell, 1, 102, 3, 1),
                order(Side::Sell, 2, 102, 2, 2),
                order(Side::Buy, 3, 98, 10, 1),
            ],
        }
    }
}

/// The hub's tape: each venue's best offer, then a trade on the far venue.
fn send_tape(fixture: &Fixture, cx: &mut TestAppContext, tape: u32) {
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::TapeSnapshot {
            request_id: tape,
            instrument_id: 7,
            last_report: 0,
            quotes: vec![
                TapeQuote {
                    venue_id: 1,
                    side: Side::Sell,
                    price: 101,
                    quantity: 5,
                },
                TapeQuote {
                    venue_id: 2,
                    side: Side::Sell,
                    price: 102,
                    quantity: 5,
                },
            ],
        }),
    );
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::TapeUpdate {
            request_id: tape,
            instrument_id: 7,
            first_report: 1,
            entries: vec![TapeEntry {
                venue_id: 2,
                kind: EntryKind::Trade,
                price: 102,
                quantity: 1,
            }],
        }),
    );
    fixture.sent.take();
}

#[gpui_kit::test]
fn every_panel_is_laid_out_and_the_first_tab_of_each_group_is_visible(cx: &mut TestAppContext) {
    let fixture = open(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        for visible in ["books", "ticket", "tape", "orders"] {
            assert!(window.find(visible).visible(), "{visible} is visible");
        }
        assert_eq!(PANELS.len(), 8);
        assert!(window.find("connection").visible());
    })
    .unwrap();
    assert!(
        fixture
            .desk
            .read_with(cx, |desk, _| bunting_desk::panels::connection_label(desk))
            .starts_with("Not connected")
    );
}

#[gpui_kit::test]
fn each_venue_book_is_shown_on_its_own_and_an_order_goes_to_the_chosen_venue(
    cx: &mut TestAppContext,
) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("venue-1").visible());
        assert!(window.find("venue-2").visible());
        // Order by order where the venue sends it, price levels where not.
        assert!(window.find("ask-order-2-0").visible());
        assert!(window.find("ask-order-2-1").visible());
        assert!(window.find("bid-order-2-0").visible());
        assert!(window.find("bid-1-0").visible());
        // No venue is chosen for the participant: sending does nothing.
        window.click("ticket-send", cx);
    })
    .unwrap();
    assert!(fixture.sent.take().is_empty(), "no order without a venue");
    // Picking the second order in the far venue's offer queue chooses that
    // venue and price.
    click(&fixture, cx, "ask-order-2-1");
    click(&fixture, cx, "ticket-send");
    let sent = fixture.sent.take();
    let [ClientMessage::NewOrder(order)] = sent.as_slice() else {
        panic!("one new order, not {sent:?}");
    };
    assert_eq!(order.listing, FAR);
    assert_eq!(order.side, Side::Buy);
    assert_eq!(order.order_type, OrderType::Limit { price: 102 });
    assert_eq!(order.quantity, 1);
}

#[gpui_kit::test]
fn a_resting_order_can_be_canceled_from_the_orders_panel(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    click(&fixture, cx, "bid-1-0");
    click(&fixture, cx, "ticket-send");
    let client_order_id = match fixture.sent.take().as_slice() {
        [ClientMessage::NewOrder(order)] => order.client_order_id,
        other => panic!("one new order, not {other:?}"),
    };
    let stamp = Stamp {
        sequence: 5,
        logical_time_ns: 1_000,
    };
    apply(
        &fixture,
        cx,
        SessionEvent::Message(ServerMessage::OrderAccepted {
            stamp,
            client_order_id,
            order_id: 900,
            listing: NEAR,
            side: Side::Buy,
            quantity: 1,
            price: Some(99),
        }),
    );
    apply(
        &fixture,
        cx,
        SessionEvent::Message(ServerMessage::OrderRested {
            stamp,
            client_order_id,
            order_id: 900,
            price: 99,
            remaining: 1,
        }),
    );
    click(&fixture, cx, &format!("cancel-{client_order_id}"));
    assert_eq!(
        fixture.sent.take(),
        vec![ClientMessage::CancelOrder { client_order_id }]
    );
}

#[gpui_kit::test]
fn the_kill_switch_is_sent_only_after_confirmation(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click("kill-switch", cx);
        window.render_frame(cx);
        assert!(window.has_active_dialog(cx), "a confirmation is shown");
    })
    .unwrap();
    assert!(
        fixture.sent.take().is_empty(),
        "nothing is sent before confirming"
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("ok").visible(), "the confirm button is shown");
        window.click("ok", cx);
    })
    .unwrap();
    let sent = fixture.sent.take();
    assert!(
        matches!(sent.as_slice(), [ClientMessage::KillSwitch { .. }]),
        "{sent:?}"
    );
}

#[gpui_kit::test]
fn the_books_switch_between_orders_and_levels(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    let queue = fixture.desk.read_with(cx, |desk, _| {
        let feed = desk.model().feed(FAR).expect("far feed");
        assert!(feed.order_by_order);
        feed.book
            .orders(Side::Sell)
            .iter()
            .map(|order| (order.position_at_price, order.quantity_ahead, order.broker))
            .collect::<Vec<_>>()
    });
    assert_eq!(queue, vec![(1, 0, Some(17)), (2, 3, Some(17))]);
    click(&fixture, cx, "books-levels");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("ask-2-0").visible(), "levels of the far venue");
        assert!(window.try_find("ask-order-2-0").is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn the_tape_shows_every_venue_and_its_trades(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("tape-consolidated").visible());
    })
    .unwrap();
    let (best, trades) = fixture.desk.read_with(cx, |desk, _| {
        let model = desk.model();
        let tape = model.tape(7).expect("tape");
        (
            tape.book
                .best(Side::Sell)
                .map(|(venue, level)| (venue, level.price)),
            model
                .tape_prints()
                .iter()
                .map(|print| (print.listing, print.price))
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(best, Some((1, 101)));
    assert_eq!(trades, vec![(FAR, 102)]);
    click(&fixture, cx, "tape-direct");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("tape-direct").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn an_active_kill_switch_can_be_reset(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    apply(
        &fixture,
        cx,
        SessionEvent::Message(ServerMessage::KillSwitchActivated {
            stamp: Stamp {
                sequence: 9,
                logical_time_ns: 2_000,
            },
        }),
    );
    fixture.sent.take();
    click(&fixture, cx, "kill-switch-reset");
    let sent = fixture.sent.take();
    assert!(
        matches!(sent.as_slice(), [ClientMessage::KillSwitchReset { .. }]),
        "{sent:?}"
    );
}
