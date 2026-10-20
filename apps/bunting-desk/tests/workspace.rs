//! The workspace in a real, headless GPUI window (`gpui-kit` test support),
//! fed scripted venue messages through the shared trading model. Clicks go
//! through real hit testing; requests are recorded where the live app would
//! write them to the venue.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use bunting_desk::{Desk, LayoutStore, Outbox, PANELS, Workspace};
use bunting_trading_state::SessionEvent;
use bunting_trading_state::bunting_client::bnp_wire::{
    BookOrder, ClientMessage, EntryKind, Level, Listing, ListingInfo, OrderType, ResumeStatus,
    Role, ServerMessage, Side, Stamp, TapeEntry, TapeQuote, Welcome,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, InputEvent as _, Modifiers, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, Point, TestAppContext, WindowBounds, WindowHandle, WindowOptions,
    base::Root, px, size,
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
    workspace: Entity<Workspace>,
    desk: Entity<Desk>,
    sent: Recorder,
}

fn open(cx: &mut TestAppContext) -> Fixture {
    open_with(cx, None)
}

fn open_with(cx: &mut TestAppContext, store: Option<LayoutStore>) -> Fixture {
    cx.update(gpui_kit::init);
    let sent = Recorder::default();
    let outbox = sent.clone();
    let desk =
        cx.update(|cx| cx.new(|_| Desk::with_outbox(Box::new(outbox), "test venue".to_owned())));
    let shared = desk.clone();
    let (window, workspace) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(2560.), px(1440.)),
            })),
            ..WindowOptions::default()
        };
        let (window, workspace) = gpui_kit::open_window(options, cx, |window, cx| {
            bunting_desk::init(cx);
            cx.new(|cx| Workspace::new(shared, store, window, cx))
        })
        .expect("open window");
        (window.downcast::<Root>().expect("Base Root"), workspace)
    });
    Fixture {
        window,
        workspace,
        desk,
        sent,
    }
}

fn apply(fixture: &Fixture, cx: &mut TestAppContext, event: SessionEvent) {
    fixture.desk.update(cx, |desk, cx| {
        let at = desk.now_us();
        desk.apply(event, at);
        cx.notify();
    });
}

/// Clicks a menu item by its label, with real mouse events at its bounds.
/// Menu items carry no caller-chosen IDs, so they are found through the
/// observed elements' roles and labels.
fn click_menu_item(fixture: &Fixture, cx: &mut TestAppContext, label: &str) {
    let label = label.to_owned();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let item = gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .find(|snap| {
                snap.role() == Some(gpui_kit::Role::MenuItem) && snap.label() == Some(&*label)
            })
            .unwrap_or_else(|| panic!("menu item {label} is not shown"));
        let at = item.bounds().center();
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Left,
                position: at,
                modifiers: gpui_kit::Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseUp(gpui_kit::MouseUpEvent {
                button: gpui_kit::MouseButton::Left,
                position: at,
                modifiers: gpui_kit::Modifiers::none(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

/// Opens another window of the named kind from the sidebar's "+" menu.
fn add_pane(fixture: &Fixture, cx: &mut TestAppContext, kind: &str) {
    let title = bunting_desk::panes::PaneKind::from_name(kind)
        .expect("a pane kind")
        .title();
    click(fixture, cx, "windows-add");
    click_menu_item(fixture, cx, title);
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

/// Clicks an element with a mouse button and modifiers held.
fn click_with(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    id: &str,
    button: MouseButton,
    modifiers: Modifiers,
) {
    let id = id.to_owned();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let position = window.find(id).bounds().center();
        for event in [
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers,
            }
            .to_platform_input(),
            MouseDownEvent {
                button,
                position,
                modifiers,
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            MouseUpEvent {
                button,
                position,
                modifiers,
                click_count: 1,
            }
            .to_platform_input(),
        ] {
            window.dispatch_event(event, cx);
            window.render_frame(cx);
        }
    })
    .unwrap();
    cx.run_until_parked();
}

/// Waits (up to two seconds) until an element stops moving, for
/// components that animate on the wall clock.
fn settle(fixture: &Fixture, cx: &mut TestAppContext, id: &str) {
    let mut last = None;
    for _ in 0..40 {
        cx.run_until_parked();
        let bounds = cx
            .update_window(fixture.window.into(), |_, window, cx| {
                window.render_frame(cx);
                window.find(id.to_owned()).bounds()
            })
            .unwrap();
        if last == Some(bounds) {
            return;
        }
        last = Some(bounds);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Presses a key in a fresh frame, then lets observers run.
fn press(fixture: &Fixture, cx: &mut TestAppContext, key: &str) {
    let key = key.to_owned();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.press(&key, cx);
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
fn every_trading_template_pane_is_a_visible_window(cx: &mut TestAppContext) {
    let fixture = open(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        for visible in PANELS {
            assert!(window.find(visible).visible(), "{visible} is visible");
        }
        assert_eq!(PANELS.len(), 7);
        assert!(window.find("connection").visible());
        assert!(window.find("canvas").visible());
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
            reference: Some(4),
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
    // The dialog slides in on the wall clock: press its button once it
    // has stopped moving.
    settle(&fixture, cx, "ok");
    click(&fixture, cx, "ok");
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

/// The panes on the shown screen, by name, back to front.
fn panes(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<String> {
    fixture.workspace.read_with(cx, |workspace, cx| {
        workspace
            .canvas()
            .read(cx)
            .dump()
            .into_iter()
            .map(|saved| saved.kind)
            .collect()
    })
}

/// The shown screen's saved windows (kind and geometry), back to front.
fn saved_windows(
    fixture: &Fixture,
    cx: &mut TestAppContext,
) -> Vec<bunting_desk::canvas::SavedWindow> {
    fixture
        .workspace
        .read_with(cx, |workspace, cx| workspace.canvas().read(cx).dump())
}

#[gpui_kit::test]
fn screens_switch_and_panes_open_more_than_once(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let trading = panes(&fixture, cx);
    assert_eq!(trading.iter().filter(|name| *name == "ticket").count(), 1);
    // The market-making template has a ladder instead.
    click(&fixture, cx, "workspace-1");
    let market_making = panes(&fixture, cx);
    assert_eq!(
        market_making
            .iter()
            .filter(|name| *name == "ladder")
            .count(),
        1
    );
    assert_eq!(
        market_making
            .iter()
            .filter(|name| *name == "ticket")
            .count(),
        1
    );
    // Another ticket opens beside the others.
    add_pane(&fixture, cx, "ticket");
    assert_eq!(
        panes(&fixture, cx)
            .iter()
            .filter(|name| *name == "ticket")
            .count(),
        2
    );
    // Going back shows the first workspace as it was, and returning keeps
    // the added pane.
    click(&fixture, cx, "workspace-0");
    assert_eq!(panes(&fixture, cx), trading);
    click(&fixture, cx, "workspace-1");
    assert_eq!(
        panes(&fixture, cx)
            .iter()
            .filter(|name| *name == "ticket")
            .count(),
        2
    );
    // A new screen starts from a chosen template.
    click(&fixture, cx, "workspace-new");
    click_menu_item(&fixture, cx, "Trading");
    let (names, active) = fixture
        .workspace
        .read_with(cx, |workspace, _| workspace.screens());
    assert_eq!(names.len(), 4);
    assert_eq!(active, 3);
}

#[gpui_kit::test]
fn saved_screens_come_back_with_their_geometry(cx: &mut TestAppContext) {
    let dir = std::env::temp_dir().join(format!(
        "bunting-desk-workspace-test-{}",
        std::process::id()
    ));
    let store = LayoutStore::at(dir.join("workspaces.json"));
    let fixture = open_with(cx, Some(store.clone()));
    click(&fixture, cx, "workspace-2");
    add_pane(&fixture, cx, "notices");
    press(&fixture, cx, "secondary-s");
    let before = saved_windows(&fixture, cx);
    assert!(
        dir.join("workspaces.json").exists(),
        "a v4 file was written"
    );
    let reopened = open_with(cx, Some(store));
    let (_, active) = reopened
        .workspace
        .read_with(cx, |workspace, _| workspace.screens());
    assert_eq!(active, 2, "the last screen shown");
    assert_eq!(
        saved_windows(&reopened, cx),
        before,
        "the same windows at the same geometry"
    );
    std::fs::remove_dir_all(dir).unwrap();
}

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

#[gpui_kit::test]
fn one_click_sends_to_the_clicked_venue_at_once(cx: &mut TestAppContext) {
    use bunting_trading_state::bunting_client::bnp_wire::TimeInForce;
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    // Off by default: a click only picks the price.
    click(&fixture, cx, "bid-order-2-0");
    assert!(new_orders(&fixture).is_empty());
    click(&fixture, cx, "books-one-click");
    // Left-click a bid: buy V (100) at its price + O (0) on that venue.
    click(&fixture, cx, "bid-order-2-0");
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, FAR);
    assert_eq!(orders[0].side, Side::Buy);
    assert_eq!(orders[0].quantity, 100);
    assert_eq!(orders[0].order_type, OrderType::Limit { price: 98 });
    // Right-click the near venue's asks: market buy.
    click_with(
        &fixture,
        cx,
        "ask-1-0",
        MouseButton::Right,
        Modifiers::default(),
    );
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, NEAR);
    assert_eq!(orders[0].side, Side::Buy);
    assert_eq!(orders[0].order_type, OrderType::Market);
    // Shift+right-click the far venue's second offer: sweep both at 102.
    click_with(
        &fixture,
        cx,
        "ask-order-2-1",
        MouseButton::Right,
        Modifiers::shift(),
    );
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, FAR);
    assert_eq!(orders[0].order_type, OrderType::Limit { price: 102 });
    assert_eq!(orders[0].quantity, 5);
    assert_eq!(orders[0].time_in_force, TimeInForce::Ioc);
}

#[gpui_kit::test]
fn alt_click_cancels_your_orders_at_a_price(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    click(&fixture, cx, "books-one-click");
    click(&fixture, cx, "bid-1-0");
    let client_order_id = new_orders(&fixture)[0].client_order_id;
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
            quantity: 100,
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
            remaining: 100,
            reference: None,
        }),
    );
    // Nothing of yours at the ask: nothing sent, and the book says so.
    click_with(&fixture, cx, "ask-1-0", MouseButton::Left, Modifiers::alt());
    assert!(fixture.sent.take().is_empty());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("books-problem").visible());
    })
    .unwrap();
    click_with(&fixture, cx, "bid-1-0", MouseButton::Left, Modifiers::alt());
    assert_eq!(
        fixture.sent.take(),
        vec![ClientMessage::CancelOrder { client_order_id }]
    );
}

#[gpui_kit::test]
fn the_command_palette_and_keys_run_the_same_commands(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let fills = |cx: &mut TestAppContext| {
        panes(&fixture, cx)
            .iter()
            .filter(|name| *name == "fills")
            .count()
    };
    let before = fills(cx);
    press(&fixture, cx, "secondary-shift-p");
    assert!(
        fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.palette_open())
    );
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("palette").visible());
        window.input("Open Fills", cx);
    })
    .unwrap();
    press(&fixture, cx, "enter");
    assert!(
        !fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.palette_open())
    );
    assert_eq!(fills(cx), before + 1);
    // Keys switch workspaces.
    press(&fixture, cx, "ctrl-tab");
    assert_eq!(
        fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.screens().1),
        1
    );
    press(&fixture, cx, "ctrl-shift-tab");
    assert_eq!(
        fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.screens().1),
        0
    );
    // The top bar's button opens it too; escape closes it.
    click(&fixture, cx, "open-palette");
    assert!(
        fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.palette_open())
    );
    press(&fixture, cx, "escape");
    assert!(
        !fixture
            .workspace
            .read_with(cx, |workspace, _| workspace.palette_open())
    );
}

#[gpui_kit::test]
fn the_ladder_trades_and_cancels_on_its_own_venue(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    add_pane(&fixture, cx, "ladder");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        // No venue until the trader picks one.
        assert!(window.try_find("ladder-bid-99").is_none());
    })
    .unwrap();
    click(&fixture, cx, "ladder-venue-1");
    // Centred on the near venue's mid (99 × 101): click its bid size.
    click(&fixture, cx, "ladder-bid-99");
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, NEAR);
    assert_eq!(orders[0].side, Side::Buy);
    assert_eq!(orders[0].order_type, OrderType::Limit { price: 99 });
    assert_eq!(orders[0].quantity, 100);
    let client_order_id = orders[0].client_order_id;
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
            quantity: 100,
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
            remaining: 100,
            reference: None,
        }),
    );
    // Your quantity shows beside the price; clicking it cancels.
    click(&fixture, cx, "ladder-mybid-99");
    assert_eq!(
        fixture.sent.take(),
        vec![ClientMessage::CancelOrder { client_order_id }]
    );
    // Space recentres without sending anything.
    click(&fixture, cx, "ladder-centre");
    press(&fixture, cx, "space");
    assert!(fixture.sent.take().is_empty());
}

#[gpui_kit::test]
fn the_ticket_sends_an_iceberg(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    // Pick the near venue's bid: venue and price go on the ticket.
    click(&fixture, cx, "bid-1-0");
    click(&fixture, cx, "ticket-display");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("1", cx);
    })
    .unwrap();
    click(&fixture, cx, "ticket-send");
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, NEAR);
    assert_eq!(orders[0].display_quantity, Some(1));
}

fn run_stream(fixture: &Fixture, cx: &mut TestAppContext) {
    use bunting_trading_state::bunting_client::bnp_wire::{
        NegotiationInfo, NegotiationStatus, TenderInfo, TenderStatus,
    };
    let stamp = |sequence| Stamp {
        sequence,
        logical_time_ns: 30_000_000_000,
    };
    for message in [
        ServerMessage::RunStatus {
            stamp: stamp(20),
            status: "active".to_owned(),
        },
        ServerMessage::ListingHalt {
            stamp: stamp(21),
            instrument_id: 7,
            halted: true,
        },
        ServerMessage::News {
            stamp: stamp(22),
            news_id: 5,
            headline: "Rates cut".to_owned(),
            body: "By 25 basis points.".to_owned(),
        },
        ServerMessage::Tender {
            stamp: stamp(23),
            tender: TenderInfo {
                tender_id: 9,
                participant_id: 42,
                instrument_id: 7,
                side: Side::Sell,
                quantity: 500,
                price: 103,
                expires_at_ns: 60_000_000_000,
                status: TenderStatus::Open,
            },
        },
        ServerMessage::Negotiation {
            stamp: stamp(24),
            negotiation: NegotiationInfo {
                negotiation_id: 77,
                proposer_id: 8,
                counterparty_id: 42,
                instrument_id: 7,
                side: Side::Buy,
                quantity: 200,
                price: 100,
                expires_at_ns: 90_000_000_000,
                status: NegotiationStatus::Proposed,
                awaiting: 42,
            },
        },
    ] {
        apply(fixture, cx, SessionEvent::Message(message));
    }
}

#[gpui_kit::test]
fn news_halts_and_run_status_are_shown(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    run_stream(&fixture, cx);
    add_pane(&fixture, cx, "news");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("run-status").visible());
        assert!(window.find("news-5").visible());
        assert!(window.find("news-text").visible());
        // The template's Book Trader window shows the halt.
        assert!(window.find("books-halted").visible());
    })
    .unwrap();
}

#[gpui_kit::test]
fn tenders_and_otc_are_answered_from_the_deals_pane(cx: &mut TestAppContext) {
    use bunting_trading_state::bunting_client::bnp_wire::OtcDecision;
    let fixture = open(cx);
    connect_with_books(&fixture, cx);
    run_stream(&fixture, cx);
    add_pane(&fixture, cx, "deals");
    // Maximise it so every action column is on screen.
    let deals = window_id(&fixture, cx, "deals");
    click(&fixture, cx, &format!("win-max-{deals}"));
    click(&fixture, cx, "tender-accept-9");
    let sent = fixture.sent.take();
    assert!(
        matches!(
            sent.as_slice(),
            [ClientMessage::TenderDecision {
                tender_id: 9,
                accept: true,
                ..
            }]
        ),
        "{sent:?}"
    );
    click(&fixture, cx, "otc-accept-77");
    let sent = fixture.sent.take();
    assert!(
        matches!(
            sent.as_slice(),
            [ClientMessage::OtcDecision {
                negotiation_id: 77,
                decision: OtcDecision::Accept,
                ..
            }]
        ),
        "{sent:?}"
    );
    // A proposal to participant 8 at 101, expiring 60 s after the latest
    // report's run time.
    click(&fixture, cx, "otc-counterparty");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("8", cx);
    })
    .unwrap();
    click(&fixture, cx, "otc-price");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.input("101", cx);
    })
    .unwrap();
    click(&fixture, cx, "otc-propose");
    let sent = fixture.sent.take();
    assert!(
        matches!(
            sent.as_slice(),
            [ClientMessage::OtcPropose {
                counterparty_id: 8,
                instrument_id: 7,
                side: Side::Buy,
                quantity: 100,
                price: 101,
                expires_at_ns: 90_000_000_000,
                ..
            }]
        ),
        "{sent:?}"
    );
}

// ---- canvas windows (ADR 0051) -------------------------------------------

/// The id of the frontmost window of the named kind.
fn window_id(fixture: &Fixture, cx: &mut TestAppContext, kind: &str) -> u64 {
    fixture
        .workspace
        .read_with(cx, |workspace, cx| {
            workspace
                .canvas()
                .read(cx)
                .window_list()
                .into_iter()
                .find(|info| info.kind.name() == kind)
                .map(|info| info.id)
        })
        .expect("a window of that kind")
}

/// The saved rect of the named kind's window.
fn window_rect(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    kind: &str,
) -> bunting_desk::canvas::geometry::Rect {
    saved_windows(fixture, cx)
        .into_iter()
        .find(|saved| saved.kind == kind)
        .expect("a window of that kind")
        .rect
}

/// Drags an element's centre by a delta, with real mouse events.
fn drag_by(fixture: &Fixture, cx: &mut TestAppContext, id: &str, dx: f32, dy: f32) {
    let id = id.to_owned();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let from = window.find(id).bounds().center();
        let to = gpui_kit::point(from.x + px(dx), from.y + px(dy));
        window.drag(from, to, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
#[allow(
    clippy::float_arithmetic,
    reason = "expected drag deltas in logical pixels"
)]
fn a_window_moves_by_its_title_and_resizes_from_its_corner(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let tape = window_id(&fixture, cx, "tape");
    let before = window_rect(&fixture, cx, "tape");
    drag_by(&fixture, cx, &format!("win-title-{tape}"), -300., 150.);
    let moved = window_rect(&fixture, cx, "tape");
    assert!(
        (moved.x - (before.x - 300.)).abs() < 1.,
        "x moved: {before:?} -> {moved:?}"
    );
    assert!(
        (moved.y - (before.y + 150.)).abs() < 1.,
        "y moved: {before:?} -> {moved:?}"
    );
    assert_eq!((moved.w, moved.h), (before.w, before.h), "size kept");

    let books = window_id(&fixture, cx, "books");
    let before = window_rect(&fixture, cx, "books");
    drag_by(
        &fixture,
        cx,
        &format!("win-grip-{books}-bottom-right"),
        100.,
        80.,
    );
    let resized = window_rect(&fixture, cx, "books");
    assert!(
        (resized.w - (before.w + 100.)).abs() < 1.,
        "width grew: {before:?} -> {resized:?}"
    );
    assert!(
        (resized.h - (before.h + 80.)).abs() < 1.,
        "height grew: {before:?} -> {resized:?}"
    );
    assert_eq!((resized.x, resized.y), (before.x, before.y), "corner kept");
}

#[gpui_kit::test]
fn a_closed_window_reopens_with_its_geometry(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let tape = window_id(&fixture, cx, "tape");
    let before = window_rect(&fixture, cx, "tape");
    // Focus it first so its frame actions are showing, then close it.
    click(&fixture, cx, &format!("win-title-{tape}"));
    click(&fixture, cx, &format!("win-close-{tape}"));
    assert!(!panes(&fixture, cx).contains(&"tape".to_owned()));
    press(&fixture, cx, "secondary-shift-t");
    assert!(panes(&fixture, cx).contains(&"tape".to_owned()));
    assert_eq!(window_rect(&fixture, cx, "tape"), before);
}

#[gpui_kit::test]
fn tile_arranges_the_windows_in_a_grid(cx: &mut TestAppContext) {
    use bunting_desk::canvas::geometry::{GUTTER, Rect};
    let fixture = open(cx);
    // One frame so the canvas knows its painted size.
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    press(&fixture, cx, "secondary-alt-t");
    let (size, saved) = fixture.workspace.read_with(cx, |workspace, cx| {
        let canvas = workspace.canvas().read(cx);
        (canvas.size(), canvas.dump())
    });
    let expected = bunting_desk::canvas::geometry::tile(
        saved.len(),
        Rect::new(0., 0., size.0, size.1),
        GUTTER,
    );
    let actual: Vec<Rect> = saved.into_iter().map(|each| each.rect).collect();
    assert_eq!(actual, expected);
}

#[gpui_kit::test]
fn switching_screens_and_back_restores_the_geometry(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let tape = window_id(&fixture, cx, "tape");
    drag_by(&fixture, cx, &format!("win-title-{tape}"), -200., 100.);
    let before = saved_windows(&fixture, cx);
    click(&fixture, cx, "workspace-1");
    click(&fixture, cx, "workspace-0");
    assert_eq!(saved_windows(&fixture, cx), before);
}

// ---- deferred templates and reflow ----------------------------------------

/// A window holding only a canvas, so its painted size is exact.
struct CanvasHost {
    canvas: Entity<bunting_desk::Canvas>,
}

impl gpui_kit::Render for CanvasHost {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl gpui_kit::IntoElement {
        use gpui_kit::{ParentElement as _, Styled as _};
        gpui_kit::div().size_full().child(self.canvas.clone())
    }
}

/// Opens a 1600x940 canvas whose seed size is deliberately wrong, shows a
/// screen on it, and paints until it settles.
fn open_canvas(
    cx: &mut TestAppContext,
    saved: Option<&[bunting_desk::canvas::SavedWindow]>,
    layout_size: Option<(f32, f32)>,
) -> (WindowHandle<Root>, Entity<bunting_desk::Canvas>) {
    cx.update(gpui_kit::init);
    let desk = cx.update(|cx| {
        cx.new(|_| Desk::with_outbox(Box::new(Recorder::default()), "test venue".to_owned()))
    });
    let canvas = cx.update(|cx| cx.new(|_| bunting_desk::Canvas::new(desk, (320., 240.))));
    let shown = canvas.clone();
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1600.), px(940.)),
            })),
            ..WindowOptions::default()
        };
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            bunting_desk::init(cx);
            shown.update(cx, |canvas, cx| {
                canvas.set_screen(
                    "Trading".to_owned(),
                    &bunting_desk::Preset::Trading.fractions(),
                    saved,
                    layout_size,
                    window,
                    cx,
                );
            });
            cx.new(|_| CanvasHost {
                canvas: shown.clone(),
            })
        })
        .expect("open window");
        window.downcast::<Root>().expect("Base Root")
    });
    // First paint records the real bounds; the next frame settles.
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    (window, canvas)
}

#[gpui_kit::test]
#[allow(clippy::float_arithmetic, reason = "expected edges in logical pixels")]
fn a_template_waits_for_the_first_painted_bounds(cx: &mut TestAppContext) {
    let (_, canvas) = open_canvas(cx, None, None);
    let saved = canvas.read_with(cx, |canvas, _| canvas.dump());
    assert_eq!(saved.len(), 7, "the Trading template's windows");
    let rightmost = saved
        .iter()
        .map(|each| each.rect.right())
        .fold(f32::MIN, f32::max);
    let bottom = saved
        .iter()
        .map(|each| each.rect.bottom())
        .fold(f32::MIN, f32::max);
    assert!(
        (rightmost - (1600. - 6.)).abs() < 0.01,
        "rightmost edge at the gutter: {rightmost}"
    );
    assert!(
        (bottom - (940. - 6.)).abs() < 0.01,
        "bottom edge at the gutter: {bottom}"
    );
}

#[gpui_kit::test]
#[allow(clippy::float_arithmetic, reason = "expected edges in logical pixels")]
fn a_screen_from_a_larger_canvas_reflows_to_this_one(cx: &mut TestAppContext) {
    use bunting_desk::canvas::SavedWindow;
    use bunting_desk::canvas::geometry::Rect;
    // Windows laid out on a 2560x1440 canvas, shown on 1600x940.
    let saved = vec![
        SavedWindow {
            kind: "books".to_owned(),
            rect: Rect::new(6., 6., 1200., 700.),
            z: 0,
            minimised: false,
            maximised: false,
        },
        SavedWindow {
            kind: "tape".to_owned(),
            rect: Rect::new(1212., 6., 1342., 700.),
            z: 1,
            minimised: false,
            maximised: false,
        },
    ];
    let (_, canvas) = open_canvas(cx, Some(&saved), Some((2560., 1440.)));
    let flowed = canvas.read_with(cx, |canvas, _| canvas.dump());
    let tape = flowed
        .iter()
        .find(|each| each.kind == "tape")
        .expect("tape kept");
    assert!(
        (tape.rect.right() - (1600. - 6.)).abs() < 1.,
        "tape reflowed onto the smaller canvas: {:?}",
        tape.rect
    );
    assert_eq!(
        canvas.read_with(cx, |canvas, _| canvas.layout_size()),
        Some((1600., 940.)),
        "the new layout size is recorded"
    );
}
