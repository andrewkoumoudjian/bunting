//! The restyled pane bodies (ADR 0051 U2b) in a real, headless GPUI
//! window: the ticket's submit label and disabled state, and the empty
//! states of the Blotter, Fills and an empty book side. The small setup
//! helpers are copied from `tests/workspace.rs` so that file stays the
//! U1 agent's.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_arithmetic
)]

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

/// A private configuration directory for one test window, so the desk's
/// appearance settings and user themes never touch the real ones.
fn isolated_config() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bunting-desk-test-config-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("test config dir");
    dir
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
            bunting_desk::init_in(&isolated_config(), cx);
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
fn connect_with_books(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    near_asks: &[Level],
) -> Vec<(u32, Listing)> {
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
    let feeds = subscriptions.clone();
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
    feeds
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
        Some("Submit BUY · 1 BNT @ 99 · Venue 1")
    );
    click(&fixture, cx, "ticket-sell");
    assert_eq!(
        label(&fixture, cx, "ticket-send").as_deref(),
        Some("Submit SELL · 1 BNT @ 99 · Venue 1")
    );
    click(&fixture, cx, "ticket-send");
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1);
    assert_eq!(orders[0].listing, NEAR);
    assert_eq!(orders[0].quantity, 1);
}

#[gpui_kit::test]
fn a_security_on_one_venue_needs_no_venue_choice(cx: &mut TestAppContext) {
    let fixture = open(cx);
    apply(
        &fixture,
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
    let request_id = fixture
        .sent
        .take()
        .into_iter()
        .find_map(|message| match message {
            ClientMessage::ListingsRequest { request_id } => Some(request_id),
            _ => None,
        })
        .expect("a listings request on connect");
    apply(
        &fixture,
        cx,
        SessionEvent::Message(ServerMessage::Listings {
            request_id,
            listings: vec![ListingInfo {
                listing: NEAR,
                symbol: "BNT".to_owned(),
            }],
        }),
    );
    // One listing is no choice, so it is the ticket's venue; with two
    // the participant still picks (see the test above).
    assert_eq!(
        label(&fixture, cx, "ticket-send").as_deref(),
        Some("Submit BUY · 1 BNT · Venue 1")
    );
    click(&fixture, cx, "ticket-market");
    fixture.sent.take();
    click(&fixture, cx, "ticket-send");
    let orders = new_orders(&fixture);
    assert_eq!(orders.len(), 1, "a market order goes without a venue click");
    assert_eq!(orders[0].listing, NEAR);
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

// ---- small windows (fix round 1) ---------------------------------------------

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

/// Opens a bare 1600x940 canvas showing `saved` windows (falling back to
/// `template`), and paints until it settles. Returns the fixture plus
/// the canvas, whose window ids address each floating window.
fn open_canvas(
    cx: &mut TestAppContext,
    template: bunting_desk::Preset,
    saved: Option<&[bunting_desk::canvas::SavedWindow]>,
) -> (Fixture, Entity<bunting_desk::Canvas>) {
    cx.update(gpui_kit::init);
    let sent = Recorder::default();
    let outbox = sent.clone();
    let desk =
        cx.update(|cx| cx.new(|_| Desk::with_outbox(Box::new(outbox), "test venue".to_owned())));
    let canvas = cx.update(|cx| cx.new(|_| bunting_desk::Canvas::new(desk.clone(), (320., 240.))));
    let shown = canvas.clone();
    let saved = saved.map(<[_]>::to_vec);
    let window = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(1600.), px(940.)),
            })),
            ..WindowOptions::default()
        };
        let (window, _) = gpui_kit::open_window(options, cx, |window, cx| {
            bunting_desk::init_in(&isolated_config(), cx);
            shown.update(cx, |canvas, cx| {
                canvas.set_screen(
                    "Test".to_owned(),
                    &template.fractions(),
                    saved.as_deref(),
                    None,
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
    for _ in 0..3 {
        cx.update_window(window.into(), |_, window, cx| {
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }
    (Fixture { window, desk, sent }, canvas)
}

/// True when `inner` lies entirely within `outer` (with a pixel of give
/// for rounding).
fn within(inner: Bounds<gpui_kit::Pixels>, outer: Bounds<gpui_kit::Pixels>) -> bool {
    let give = 1.5;
    f32::from(inner.left()) >= f32::from(outer.left()) - give
        && f32::from(inner.top()) >= f32::from(outer.top()) - give
        && f32::from(inner.right()) <= f32::from(outer.right()) + give
        && f32::from(inner.bottom()) <= f32::from(outer.bottom()) + give
}

/// The floating window holding the only window of `kind` on the canvas.
fn window_bounds_of_kind(
    fixture: &Fixture,
    canvas: &Entity<bunting_desk::Canvas>,
    kind: &str,
    cx: &mut TestAppContext,
) -> Bounds<gpui_kit::Pixels> {
    let id = canvas
        .read_with(cx, |canvas, _| canvas.window_list())
        .into_iter()
        .find(|info| info.kind.name() == kind)
        .expect("a window of the kind")
        .id;
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(format!("window-{id}")).bounds()
    })
    .unwrap()
}

fn element_bounds(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    id: &str,
) -> Bounds<gpui_kit::Pixels> {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.find(id.to_owned()).bounds()
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_ticket_footer_stays_inside_its_window_at_minimum_size(cx: &mut TestAppContext) {
    // Order Entry's minimum floating-window size is 240x260.
    let saved = [bunting_desk::canvas::SavedWindow {
        kind: "ticket".to_owned(),
        rect: bunting_desk::canvas::geometry::Rect::new(6., 6., 240., 260.),
        z: 0,
        minimised: false,
        maximised: false,
        state: None,
    }];
    let (fixture, canvas) = open_canvas(cx, bunting_desk::Preset::Trading, Some(&saved));
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    let window = window_bounds_of_kind(&fixture, &canvas, "ticket", cx);
    assert!(
        (f32::from(window.size.width) - 240.).abs() < 2.,
        "at minimum width, got {}",
        f32::from(window.size.width)
    );
    for id in ["ticket-send", "kill-switch"] {
        assert!(
            within(element_bounds(&fixture, cx, id), window),
            "{id} is inside the ticket window"
        );
    }
}

#[gpui_kit::test]
fn the_ticket_footer_stays_inside_at_market_making_template_size(cx: &mut TestAppContext) {
    let (fixture, canvas) = open_canvas(cx, bunting_desk::Preset::MarketMaking, None);
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    let window = window_bounds_of_kind(&fixture, &canvas, "ticket", cx);
    for id in ["ticket-send", "kill-switch"] {
        assert!(
            within(element_bounds(&fixture, cx, id), window),
            "{id} is inside the ticket window"
        );
    }
}

#[gpui_kit::test]
fn the_ladder_volume_input_fits_five_digits(cx: &mut TestAppContext) {
    let saved = [bunting_desk::canvas::SavedWindow {
        kind: "ladder".to_owned(),
        rect: bunting_desk::canvas::geometry::Rect::new(6., 6., 420., 420.),
        z: 0,
        minimised: false,
        maximised: false,
        state: None,
    }];
    let (fixture, _canvas) = open_canvas(cx, bunting_desk::Preset::Trading, Some(&saved));
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    // The width is set in rems: 112 design px at the default 13 px
    // font is 112 * 13/16 = 91 px, enough for five digits and both
    // steppers at that font size.
    let (bounds, value) = cx
        .update_window(fixture.window.into(), |_, window, cx| {
            window.render_frame(cx);
            let found = window.find("ladder-volume");
            (found.bounds(), found.value().map(str::to_owned))
        })
        .unwrap();
    assert!(
        f32::from(bounds.size.width) >= 90.,
        "the V input fits five digits at the default font, got {}",
        f32::from(bounds.size.width)
    );
    // The input hosts its measure inside the wrapper, so the wrapper
    // itself may expose no value; when it does, it is the default.
    if let Some(value) = value {
        assert_eq!(value, "100");
    }
}

#[test]
fn book_columns_fit_at_the_book_trader_minimum_width() {
    // Book Trader's minimum window is 360 wide: pane padding (16) and
    // card padding (16) leave 328 for the card's content, so each side
    // gets (328 - 12 gap) / 2 = 158 px.
    let side = 158.;
    let fitted = bunting_desk::ui::fit(&bunting_desk::panels::ORDER_COLUMNS, side);
    let kept: Vec<usize> = fitted.iter().map(|fit| fit.index).collect();
    assert_eq!(kept, [0, 2, 3], "broker and ahead drop before clipping");
    assert!(
        fitted
            .iter()
            .all(|fit| fit.width >= fit.col.min_width - 0.01),
        "no fitted header is narrower than its label's minimum"
    );
    let fitted = bunting_desk::ui::fit(&bunting_desk::panels::LEVEL_COLUMNS, side);
    assert!(
        fitted
            .iter()
            .all(|fit| fit.width >= fit.col.min_width - 0.01)
    );
}

// ---- window parity (RIT): Portfolio, Blotter, Transaction Log ---------------------

use bunting_trading_state::bunting_client::bnp_wire::{
    CancelReason, Entry, EntryKind, Liquidity, Side, Stamp,
};

fn request_of(feeds: &[(u32, Listing)], listing: Listing) -> u32 {
    feeds
        .iter()
        .find(|(_, each)| *each == listing)
        .expect("a feed for the listing")
        .0
}

/// A trade on `listing`'s direct feed; `first_report` is the next report
/// number the feed expects.
fn trade(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    feeds: &[(u32, Listing)],
    listing: Listing,
    first_report: u64,
    price: i64,
) {
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::MarketUpdate {
            request_id: request_of(feeds, listing),
            listing,
            first_report,
            entries: vec![Entry {
                kind: EntryKind::Trade,
                price,
                quantity: 1,
            }],
        }),
    );
}

#[allow(clippy::too_many_arguments)]
fn fill(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    sequence: u64,
    client_order_id: u64,
    side: Side,
    price: i64,
    quantity: i64,
    fee: i128,
) {
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::Fill {
            stamp: Stamp {
                sequence,
                logical_time_ns: sequence * 1_000_000_000,
            },
            client_order_id,
            order_id: u128::from(client_order_id),
            listing: NEAR,
            side,
            price,
            quantity,
            fee,
            liquidity: Liquidity::Taker,
        }),
    );
}

fn account(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    sequence: u64,
    balance: i128,
    position: i64,
) {
    use bunting_trading_state::bunting_client::bnp_wire::{CashBalance, Position};
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::Account {
            request_id: 0,
            run_sequence: sequence,
            cash: vec![CashBalance {
                currency_id: 1,
                balance,
                reserved: 0,
            }],
            positions: vec![Position {
                instrument_id: 7,
                position,
                open_buy: 0,
                open_sell: 0,
            }],
        }),
    );
}

/// Buy 10 at 100 (fee 2), sell 4 at 103 (a rebate of 1): long 6 at cost
/// 100, with the venue's account at sequence 2.
fn script_trading(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<(u32, Listing)> {
    let feeds = connect_with_books(
        fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    fill(fixture, cx, 1, 11, Side::Buy, 100, 10, 2);
    fill(fixture, cx, 2, 12, Side::Sell, 103, 4, -1);
    account(fixture, cx, 2, 100_000, 6);
    feeds
}

fn rows(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<bunting_desk::portfolio::Row> {
    fixture
        .desk
        .read_with(cx, |desk, _| bunting_desk::portfolio::rows(desk.model()))
}

#[gpui_kit::test]
fn the_portfolio_row_derives_from_fills_and_books(cx: &mut TestAppContext) {
    let fixture = open(cx);
    let feeds = script_trading(&fixture, cx);
    trade(&fixture, cx, &feeds, NEAR, 1, 100);
    trade(&fixture, cx, &feeds, NEAR, 2, 101);
    let rows = rows(&fixture, cx);
    assert_eq!(rows.len(), 1, "one row per listed security");
    let row = &rows[0];
    assert_eq!(row.symbol, "BNT");
    assert_eq!(row.position, Some(6));
    assert_eq!(row.cost.as_deref(), Some("100.0"));
    // 4 lots closed at +3 each, less the net fee of 1: 12 - 2 + 1.
    assert_eq!(row.realized, 11);
    // Best bid 99 on both venues: (99 - 100) x 6 lots.
    assert_eq!(row.unrealized, Some(-6));
    assert_eq!(row.nlv, Some(6 * 99));
    assert_eq!(row.bid.map(|(price, _)| price), Some(99));
    assert_eq!(row.ask, Some((101, 1)), "the best ask names its venue");
    assert_eq!(row.volume, 2);
    let texts = row.texts();
    assert_eq!(texts[3], "6");
    assert_eq!(texts[2], "1*", "contract size is assumed until sent");
    assert_eq!(texts[7], "101 · V1");
    assert!(visible(&fixture, cx, "positions-table"));
}

#[gpui_kit::test]
fn last_ticks_up_then_down_until_the_price_changes(cx: &mut TestAppContext) {
    use std::cmp::Ordering;
    let fixture = open(cx);
    let feeds = script_trading(&fixture, cx);
    trade(&fixture, cx, &feeds, NEAR, 1, 100);
    assert_eq!(rows(&fixture, cx)[0].tick, None, "one trade has no tick");
    trade(&fixture, cx, &feeds, NEAR, 2, 101);
    let row = rows(&fixture, cx).remove(0);
    assert_eq!(
        (row.last, row.tick),
        (Some((101, 1)), Some(Ordering::Greater))
    );
    trade(&fixture, cx, &feeds, NEAR, 3, 101);
    assert_eq!(
        rows(&fixture, cx)[0].tick,
        Some(Ordering::Greater),
        "an unchanged price keeps the last tick"
    );
    trade(&fixture, cx, &feeds, NEAR, 4, 99);
    let row = rows(&fixture, cx).remove(0);
    assert_eq!((row.last, row.tick), (Some((99, 1)), Some(Ordering::Less)));
}

#[gpui_kit::test]
fn the_transaction_log_lists_fills_and_fees_with_a_running_balance(cx: &mut TestAppContext) {
    use bunting_desk::transactions::{Kind, entries};
    let fixture = open(cx);
    script_trading(&fixture, cx);
    let list = fixture.desk.read_with(cx, |desk, _| entries(desk.model()));
    let summary: Vec<(Kind, i128, Option<i128>)> = list
        .iter()
        .map(|entry| (entry.kind, entry.flow, entry.balance))
        .collect();
    assert_eq!(
        summary,
        [
            (Kind::Buy, -1_000, Some(99_589)),
            (Kind::Fee, -2, Some(99_587)),
            (Kind::Sell, 412, Some(99_999)),
            (Kind::Rebate, 1, Some(100_000)),
        ]
    );
    let text = fixture.desk.read_with(cx, |desk, _| {
        bunting_desk::transactions::copy_text(desk.model())
    });
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0].split('\t').count(), 7, "a header row of 7 columns");
    assert_eq!(lines.len(), 5);
    assert!(
        lines[1].contains("Buy fill\t100\t10\t-1,000\t99,589"),
        "{}",
        lines[1]
    );
}

#[gpui_kit::test]
fn the_blotter_tabs_cancel_and_copy(cx: &mut TestAppContext) {
    use bunting_desk::blotter::{Tab, copy_text};
    let fixture = open(cx);
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    // A bid for 1 on the near venue, via the ticket.
    click(&fixture, cx, "bid-1-0");
    let _ = fixture.sent.take();
    click(&fixture, cx, "ticket-send");
    let id = new_orders(&fixture)[0].client_order_id;
    let stamp = |sequence| Stamp {
        sequence,
        logical_time_ns: 0,
    };
    apply(
        &fixture,
        cx,
        SessionEvent::Message(ServerMessage::OrderAccepted {
            stamp: stamp(1),
            client_order_id: id,
            order_id: 5,
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
            stamp: stamp(2),
            client_order_id: id,
            order_id: 5,
            price: 99,
            remaining: 1,
            reference: None,
        }),
    );
    assert!(
        visible(&fixture, cx, &format!("cancel-{id}")),
        "Live tab has Cancel"
    );
    click(&fixture, cx, &format!("cancel-{id}"));
    assert!(fixture.sent.take().contains(&ClientMessage::CancelOrder {
        client_order_id: id
    }));
    apply(
        &fixture,
        cx,
        SessionEvent::Message(ServerMessage::OrderCanceled {
            stamp: stamp(3),
            client_order_id: id,
            order_id: 5,
            remaining: 1,
            reason: CancelReason::Requested,
        }),
    );
    assert!(visible(&fixture, cx, "orders-empty"), "nothing live now");
    click(&fixture, cx, "blotter-tab-done");
    assert!(visible(&fixture, cx, &format!("order-status-{id}")));
    let text = fixture
        .desk
        .read_with(cx, |desk, _| copy_text(desk.model(), Tab::Done));
    assert_eq!(text.lines().count(), 2, "header and the cancelled order");
    assert!(text.contains("canceled"));
    click(&fixture, cx, "blotter-tab-endowments");
    assert!(visible(&fixture, cx, "blotter-endowments-empty"));
    click(&fixture, cx, "blotter-tab-tenders");
    assert!(visible(&fixture, cx, "blotter-tenders-empty"));
}

#[gpui_kit::test]
fn copy_to_clipboard_writes_tab_separated_text(cx: &mut TestAppContext) {
    let fixture = open(cx);
    script_trading(&fixture, cx);
    let text = fixture.desk.read_with(cx, |desk, _| {
        bunting_desk::portfolio::copy_text(desk.model())
    });
    cx.update(|cx| bunting_desk::ui::copy_to_clipboard(text.clone(), cx));
    let copied = cx
        .update(|cx| cx.read_from_clipboard())
        .and_then(|item| item.text())
        .expect("text on the clipboard");
    assert_eq!(copied, text);
    let mut lines = copied.lines();
    let header = lines.next().expect("header");
    assert!(header.starts_with("ticker\ttype\tsize\tposition\tcost\tlast\tbid\task\tNLV"));
    assert!(lines.next().expect("row").starts_with("BNT\t—\t1*\t6\t100"));
}

// ---- window parity (RIT): Order Entry, Kill All, Close all ------------------------

/// Rests a one-lot bid through the ticket and returns its client order ID.
fn rest_a_bid(fixture: &Fixture, cx: &mut TestAppContext) -> u64 {
    click(fixture, cx, "bid-1-0");
    let _ = fixture.sent.take();
    click(fixture, cx, "ticket-send");
    let id = new_orders(fixture)[0].client_order_id;
    let stamp = |sequence| Stamp {
        sequence,
        logical_time_ns: 0,
    };
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::OrderAccepted {
            stamp: stamp(1),
            client_order_id: id,
            order_id: 5,
            listing: NEAR,
            side: Side::Buy,
            quantity: 1,
            price: Some(99),
        }),
    );
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::OrderRested {
            stamp: stamp(2),
            client_order_id: id,
            order_id: 5,
            price: 99,
            remaining: 1,
            reference: None,
        }),
    );
    id
}

fn step(fixture: &Fixture, cx: &mut TestAppContext, field: &str, button: &str) {
    let (field, button) = (field.to_owned(), button.to_owned());
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.within(field).click(button, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_steppers_move_volume_by_ten_lots_and_price_by_one_tick(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    click(&fixture, cx, "bid-1-0");
    let send = |cx: &mut TestAppContext| label(&fixture, cx, "ticket-send").unwrap();
    assert_eq!(send(cx), "Submit BUY · 1 BNT @ 99 · Venue 1");
    step(&fixture, cx, "ticket-quantity", "increment");
    step(&fixture, cx, "ticket-quantity", "increment");
    assert_eq!(send(cx), "Submit BUY · 21 BNT @ 99 · Venue 1");
    step(&fixture, cx, "ticket-quantity", "decrement");
    assert_eq!(send(cx), "Submit BUY · 11 BNT @ 99 · Venue 1");
    step(&fixture, cx, "ticket-price", "increment");
    assert_eq!(send(cx), "Submit BUY · 11 BNT @ 100 · Venue 1");
    step(&fixture, cx, "ticket-price", "decrement");
    step(&fixture, cx, "ticket-price", "decrement");
    assert_eq!(send(cx), "Submit BUY · 11 BNT @ 98 · Venue 1");
}

#[gpui_kit::test]
fn plus_in_the_frame_opens_another_order_entry(cx: &mut TestAppContext) {
    let (fixture, canvas) = open_canvas(cx, bunting_desk::Preset::Trading, None);
    let tickets = |cx: &mut TestAppContext| {
        canvas
            .read_with(cx, |canvas, _| canvas.window_list())
            .into_iter()
            .filter(|info| info.kind.name() == "ticket")
            .map(|info| info.id)
            .collect::<Vec<_>>()
    };
    let before = tickets(cx);
    assert_eq!(before.len(), 1);
    click(&fixture, cx, &format!("win-add-{}", before[0]));
    let after = tickets(cx);
    assert_eq!(after.len(), 2, "a second Order Entry window");
}

#[gpui_kit::test]
fn kill_all_orders_cancels_every_live_order(cx: &mut TestAppContext) {
    let fixture = open(cx);
    connect_with_books(
        &fixture,
        cx,
        &[Level {
            price: 101,
            quantity: 5,
        }],
    );
    let id = rest_a_bid(&fixture, cx);
    let _ = fixture.sent.take();
    click(&fixture, cx, "kill-all-orders");
    assert_eq!(
        fixture.sent.take(),
        [ClientMessage::CancelOrder {
            client_order_id: id
        }]
    );
}

#[gpui_kit::test]
fn close_all_closes_every_window_and_reopen_brings_one_back(cx: &mut TestAppContext) {
    let fixture = open(cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
    })
    .unwrap();
    assert!(visible(&fixture, cx, "ticket"));
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(bunting_desk::actions::CloseAllWindows), cx);
    })
    .unwrap();
    cx.run_until_parked();
    for pane in bunting_desk::PANELS {
        assert!(!visible(&fixture, cx, pane), "{pane} closed");
    }
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.dispatch_action(Box::new(bunting_desk::actions::ReopenClosedWindow), cx);
    })
    .unwrap();
    cx.run_until_parked();
    let open: usize = bunting_desk::PANELS
        .iter()
        .filter(|pane| visible(&fixture, cx, pane))
        .count();
    assert_eq!(open, 1, "the last closed window is back");
}
