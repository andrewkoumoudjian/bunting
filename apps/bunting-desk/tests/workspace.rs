//! The workspace in a real, headless GPUI window (`gpui-kit` test support),
//! fed scripted venue messages through the shared trading model. Clicks go
//! through real hit testing; requests are recorded where the live app would
//! write them to the venue.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bunting_desk::{Desk, Outbox, PANELS, Workspace};
use bunting_trading_state::SessionEvent;
use bunting_trading_state::bunting_client::bnp_wire::{
    ClientMessage, Level, Listing, ListingInfo, OrderType, ResumeStatus, Role, ServerMessage, Side,
    Stamp, Welcome,
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
    let subscriptions: Vec<(u32, Listing)> = fixture
        .sent
        .take()
        .into_iter()
        .filter_map(|message| match message {
            // Venue 0 is the instrument's consolidated tape.
            ClientMessage::Subscribe {
                request_id,
                listing,
                ..
            } if listing.venue_id != 0 => Some((request_id, listing)),
            _ => None,
        })
        .collect();
    assert_eq!(subscriptions.len(), 2, "one direct feed per venue");
    for (request_id, listing) in subscriptions {
        let (bid, ask) = if listing == NEAR {
            (99, 101)
        } else {
            (98, 102)
        };
        apply(
            fixture,
            cx,
            SessionEvent::Message(ServerMessage::MarketSnapshot {
                request_id,
                listing,
                next_report: 1,
                bids: vec![Level {
                    price: bid,
                    quantity: 10,
                }],
                asks: vec![Level {
                    price: ask,
                    quantity: 5,
                }],
            }),
        );
    }
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
        // No venue is chosen for the participant: sending does nothing.
        window.click("ticket-send", cx);
    })
    .unwrap();
    assert!(fixture.sent.take().is_empty(), "no order without a venue");
    // Picking the far venue's best ask chooses that venue and price.
    click(&fixture, cx, "ask-2-0");
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
        window.click("ok", cx);
    })
    .unwrap();
    let sent = fixture.sent.take();
    assert!(
        matches!(sent.as_slice(), [ClientMessage::KillSwitch { .. }]),
        "{sent:?}"
    );
}
