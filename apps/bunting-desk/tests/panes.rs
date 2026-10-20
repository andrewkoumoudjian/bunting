//! The restyled pane bodies (ADR 0051 U2b) in a real, headless GPUI
//! window: the ticket's submit label and disabled state, and the empty
//! states of the Blotter, Fills and an empty book side. The small setup
//! helpers are copied from `tests/workspace.rs` so that file stays the
//! U1 agent's.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bunting_desk::{Desk, Outbox, Workspace};
use bunting_trading_state::SessionEvent;
use bunting_trading_state::bunting_client::bnp_wire::{
    ClientMessage, Level, Listing, ListingInfo, ResumeStatus, Role, ServerMessage, Welcome,
};
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
            bunting_desk::init(cx);
            cx.new(|cx| Workspace::new(shared, None, window, cx))
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

/// The accessibility label of an element, from a fresh frame. A
/// button's label is its visible label.
fn label(fixture: &Fixture, cx: &mut TestAppContext, id: &str) -> Option<String> {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(id.to_owned()).label().map(str::to_owned)
    })
    .unwrap()
}

/// New-order requests recorded since the last take.
fn new_orders(fixture: &Fixture) -> Vec<bunting_trading_state::bunting_client::bnp_wire::NewOrder> {
    fixture
        .sent
        .take()
        .into_iter()
        .filter_map(|message| match message {
            ClientMessage::NewOrder(order) => Some(order),
            _ => None,
        })
        .collect()
}

fn visible(fixture: &Fixture, cx: &mut TestAppContext, id: &str) -> bool {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window
            .try_find(id.to_owned())
            .is_some_and(|found| found.visible())
    })
    .unwrap()
}

/// Connects, lists BNT on two venues and gives each a level book. The
/// near venue's asks are the caller's, so a test can empty that side.
fn connect_with_books(fixture: &Fixture, cx: &mut TestAppContext, near_asks: &[Level]) {
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
    for message in fixture.sent.take() {
        if let ClientMessage::Subscribe {
            request_id,
            listing,
            ..
        } = message
        {
            // Venue 0 is the instrument's consolidated tape.
            if listing.venue_id != 0 {
                subscriptions.push((request_id, listing));
            }
        }
    }
    assert_eq!(subscriptions.len(), 2, "one direct feed per venue");
    for (request_id, listing) in subscriptions {
        let asks = if listing == NEAR {
            near_asks.to_vec()
        } else {
            vec![Level {
                price: 102,
                quantity: 5,
            }]
        };
        apply(
            fixture,
            cx,
            SessionEvent::Message(ServerMessage::MarketSnapshot {
                request_id,
                listing,
                next_report: 1,
                bids: vec![Level {
                    price: 99,
                    quantity: 10,
                }],
                asks,
            }),
        );
    }
    fixture.sent.take();
}

#[gpui_kit::test]
fn the_submit_label_names_side_quantity_price_and_venue(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    // No venue chosen yet: the button says so, and pressing it sends
    // nothing and does not even reach the ticket's error line.
    assert_eq!(
        label(&fixture, cx, "ticket-send").as_deref(),
        Some("Choose a venue")
    );
    click(&fixture, cx, "ticket-send");
    assert!(new_orders(&fixture).is_empty(), "no venue, no order");
    assert!(
        !visible(&fixture, cx, "ticket-problem"),
        "a disabled button runs nothing"
    );
    // Picking the near venue's bid puts its venue and price on the
    // ticket; the label now names the whole order, and it sends.
    click(&fixture, cx, "bid-1-0");
    assert_eq!(
        label(&fixture, cx, "ticket-send").as_deref(),
        Some("Buy 1 BNT @ 99 on Venue 1")
    );
    click(&fixture, cx, "ticket-sell");
    assert_eq!(
        label(&fixture, cx, "ticket-send").as_deref(),
        Some("Sell 1 BNT @ 99 on Venue 1")
    );
    click(&fixture, cx, "ticket-send");
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, NEAR);
    assert_eq!(orders[0].quantity, 1);
}

#[gpui_kit::test]
fn empty_blotter_and_fills_show_their_empty_states(cx: &mut TestAppContext) {
    let fixture = open(cx);
    assert!(visible(&fixture, cx, "orders-empty"), "No open orders");
    assert!(visible(&fixture, cx, "fills-empty"), "No fills yet");
}

#[gpui_kit::test]
fn an_empty_ask_side_says_no_asks(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx, &[]);
    assert!(visible(&fixture, cx, "no-asks-1"), "No asks on Venue 1");
    assert!(!visible(&fixture, cx, "no-bids-1"), "bids are shown");
    assert!(!visible(&fixture, cx, "no-asks-2"), "Venue 2 has asks");
}
