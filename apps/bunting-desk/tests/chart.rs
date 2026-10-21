//! The Charting and P&L Chart windows in a real, headless GPUI
//! window: scripted trades become bars, the wheel zooms, a drag
//! pans, a double-click follows again, a second security switches
//! to comparison, and the chart's saved state round-trips through
//! a screen. Setup helpers are copied from `tests/panes.rs` so
//! that file stays its author's.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_arithmetic
)]

use bunting_desk::chart::ChartPanel;
use bunting_desk::{Desk, Outbox, PaneKind, Workspace};
use bunting_trading_state::SessionEvent;
use bunting_trading_state::bunting_client::bnp_wire::{
    ClientMessage, Entry, EntryKind, Level, Liquidity, Listing, ListingInfo, ResumeStatus, Role,
    ServerMessage, Side, Stamp, Welcome,
};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, Entity, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent,
    PlatformInput, Point, ScrollDelta, ScrollWheelEvent, TestAppContext, WindowBounds,
    WindowHandle, WindowOptions, base::Root, point, px, size,
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
const OTHER: Listing = Listing {
    venue_id: 1,
    instrument_id: 9,
};
const SEC: u64 = 1_000_000_000;

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
    panel: Option<Entity<ChartPanel>>,
    workspace: Option<Entity<Workspace>>,
    sent: Recorder,
}

fn isolated_config() -> std::path::PathBuf {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "bunting-desk-chart-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).expect("test config dir");
    dir
}

fn window_options() -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: Point::default(),
            size: size(px(1400.), px(900.)),
        })),
        ..WindowOptions::default()
    }
}

/// A window whose root is one Chart panel on an offline desk.
fn open_panel(cx: &mut TestAppContext) -> Fixture {
    cx.update(gpui_kit::init);
    let sent = Recorder::default();
    let outbox = sent.clone();
    let desk =
        cx.update(|cx| cx.new(|_| Desk::with_outbox(Box::new(outbox), "test venue".to_owned())));
    let shared = desk.clone();
    let slot: Rc<RefCell<Option<Entity<ChartPanel>>>> = Rc::new(RefCell::new(None));
    let kept = slot.clone();
    let window = cx.update(|cx| {
        let (window, _) = gpui_kit::open_window(window_options(), cx, |window, cx| {
            bunting_desk::init_in(&isolated_config(), cx);
            let panel = cx.new(|cx| ChartPanel::new(shared, window, cx));
            *kept.borrow_mut() = Some(panel.clone());
            panel
        })
        .expect("open window");
        window.downcast::<Root>().expect("Base Root")
    });
    Fixture {
        window,
        desk,
        panel: slot.borrow().clone(),
        workspace: None,
        sent,
    }
}

/// A window whose root is the whole Workspace.
fn open_workspace(cx: &mut TestAppContext) -> Fixture {
    cx.update(gpui_kit::init);
    let sent = Recorder::default();
    let outbox = sent.clone();
    let desk =
        cx.update(|cx| cx.new(|_| Desk::with_outbox(Box::new(outbox), "test venue".to_owned())));
    let shared = desk.clone();
    let slot: Rc<RefCell<Option<Entity<Workspace>>>> = Rc::new(RefCell::new(None));
    let kept = slot.clone();
    let window = cx.update(|cx| {
        let (window, _) = gpui_kit::open_window(window_options(), cx, |window, cx| {
            bunting_desk::init_in(&isolated_config(), cx);
            let workspace = cx.new(|cx| Workspace::new(shared, None, window, cx));
            *kept.borrow_mut() = Some(workspace.clone());
            workspace
        })
        .expect("open window");
        window.downcast::<Root>().expect("Base Root")
    });
    Fixture {
        window,
        desk,
        panel: None,
        workspace: slot.borrow().clone(),
        sent,
    }
}

fn apply(fixture: &Fixture, cx: &mut TestAppContext, event: SessionEvent) {
    fixture.desk.update(cx, |desk, cx| {
        let at = desk.now_us();
        desk.apply(event, at);
        cx.notify();
    });
    cx.run_until_parked();
}

/// Connects and lists BNT on two venues and XYZ on venue 1, with a
/// 99/101 book on each direct feed. Returns the feed requests.
fn connect(fixture: &Fixture, cx: &mut TestAppContext) -> Vec<(u32, Listing)> {
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
                ListingInfo {
                    listing: OTHER,
                    symbol: "XYZ".to_owned(),
                },
            ],
        }),
    );
    let mut feeds = Vec::new();
    for message in fixture.sent.take() {
        if let ClientMessage::Subscribe {
            request_id,
            listing,
            ..
        } = message
            && listing.venue_id != 0
        {
            feeds.push((request_id, listing));
        }
    }
    assert_eq!(feeds.len(), 3, "one direct feed per listing");
    for (request_id, listing) in &feeds {
        apply(
            fixture,
            cx,
            SessionEvent::Message(ServerMessage::MarketSnapshot {
                request_id: *request_id,
                listing: *listing,
                next_report: 1,
                bids: vec![Level {
                    price: 99,
                    quantity: 10,
                }],
                asks: vec![Level {
                    price: 101,
                    quantity: 10,
                }],
            }),
        );
    }
    fixture.sent.take();
    feeds
}

fn request_of(feeds: &[(u32, Listing)], listing: Listing) -> u32 {
    feeds
        .iter()
        .find(|(_, each)| *each == listing)
        .map(|(request_id, _)| *request_id)
        .expect("a feed for the listing")
}

/// Moves the series clock to `at_sec` with a zero-quantity fill:
/// BNP feed updates carry no run time, so bars are placed by the
/// latest stamped report (see the trading-state series docs).
fn advance_clock(fixture: &Fixture, cx: &mut TestAppContext, sequence: u64, at_sec: u64) {
    apply(
        fixture,
        cx,
        SessionEvent::Message(ServerMessage::Fill {
            stamp: Stamp {
                sequence,
                logical_time_ns: at_sec * SEC,
            },
            client_order_id: 900 + sequence,
            order_id: u128::from(900 + sequence),
            listing: NEAR,
            side: Side::Buy,
            price: 100,
            quantity: 0,
            fee: 0,
            liquidity: Liquidity::Maker,
        }),
    );
}

fn trade(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    feeds: &[(u32, Listing)],
    listing: Listing,
    first_report: u64,
    price: i64,
    quantity: i64,
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
                quantity,
            }],
        }),
    );
}

/// Scripts `count` bars one trade each, `step_sec` apart in run
/// time, prices walking around `base`. Feed report numbers and
/// stamp sequences continue from `sequence`.
#[allow(
    clippy::too_many_arguments,
    reason = "a scripting helper spelling out its feed state"
)]
fn script_bars(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    feeds: &[(u32, Listing)],
    listing: Listing,
    count: u64,
    step_sec: u64,
    base: i64,
    sequence: &mut u64,
    report: &mut u64,
) {
    for index in 0..count {
        *sequence += 1;
        advance_clock(fixture, cx, *sequence, index * step_sec);
        *report += 1;
        let wave = (index % 7).cast_signed() - 3;
        trade(fixture, cx, feeds, listing, *report, base + wave, 3);
    }
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

fn click(fixture: &Fixture, cx: &mut TestAppContext, id: &str) {
    let id = id.to_owned();
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        window.click(id, cx);
    })
    .unwrap();
    cx.run_until_parked();
}

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
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::none(),
                click_count: 1,
                first_mouse: false,
            }),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            PlatformInput::MouseUp(MouseUpEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::none(),
                click_count: 1,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

fn panel(fixture: &Fixture) -> &Entity<ChartPanel> {
    fixture.panel.as_ref().expect("a chart panel root")
}

#[gpui_kit::test]
fn scripted_trades_become_bars_on_the_chart(cx: &mut TestAppContext) {
    let fixture = open_panel(cx);
    let feeds = connect(&fixture, cx);
    // Opens with one series: the selected security's venue-1
    // listing, even though BNT trades on two venues.
    assert_eq!(
        panel(&fixture).read_with(cx, |panel, _| panel.listings()),
        vec![NEAR]
    );
    assert!(visible(&fixture, cx, "chart-empty"), "no trades yet");

    let (mut sequence, mut report) = (0_u64, 0_u64);
    script_bars(
        &fixture,
        cx,
        &feeds,
        NEAR,
        24,
        5,
        100,
        &mut sequence,
        &mut report,
    );

    let bars = fixture
        .desk
        .read_with(cx, |desk, _| desk.model().bars(NEAR, 5 * SEC));
    assert_eq!(bars.len(), 24, "one 5 s bar per scripted trade");
    assert_eq!(bars[0].volume, 3);
    assert!(!visible(&fixture, cx, "chart-empty"));
    assert!(visible(&fixture, cx, "chart-price"), "the plot area");
    assert!(visible(&fixture, cx, "chart-chip-1-7"), "the series chip");
    assert_eq!(
        panel(&fixture).read_with(cx, |panel, _| panel.bar_ns()),
        5 * SEC,
        "5 s is the starting bar size"
    );
    // The 15 s segment folds the same trades three to a bar.
    click(&fixture, cx, "chart-bar-15s");
    assert_eq!(
        panel(&fixture).read_with(cx, |panel, _| panel.bar_ns()),
        15 * SEC
    );
    let folded = fixture
        .desk
        .read_with(cx, |desk, _| desk.model().bars(NEAR, 15 * SEC));
    assert_eq!(folded.len(), 8);
    assert_eq!(folded[0].volume, 9);
}

fn wheel(fixture: &Fixture, cx: &mut TestAppContext, lines: f32) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let at = window.find("chart-price".to_owned()).bounds().center();
        window.dispatch_event(
            PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: at,
                delta: ScrollDelta::Lines(point(0., lines)),
                ..Default::default()
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

fn drag_plot(fixture: &Fixture, cx: &mut TestAppContext, dx: f32) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let from = window.find("chart-price".to_owned()).bounds().center();
        window.drag(from, point(from.x + px(dx), from.y), cx);
    })
    .unwrap();
    cx.run_until_parked();
}

fn double_click_plot(fixture: &Fixture, cx: &mut TestAppContext) {
    cx.update_window(fixture.window.into(), |_, window, cx| {
        window.render_frame(cx);
        let at = window.find("chart-price".to_owned()).bounds().center();
        window.dispatch_event(
            PlatformInput::MouseDown(MouseDownEvent {
                button: MouseButton::Left,
                position: at,
                modifiers: Modifiers::none(),
                click_count: 2,
                first_mouse: false,
            }),
            cx,
        );
    })
    .unwrap();
    cx.run_until_parked();
}

#[gpui_kit::test]
fn the_wheel_zooms_a_drag_pans_and_a_double_click_follows_again(cx: &mut TestAppContext) {
    let fixture = open_panel(cx);
    let feeds = connect(&fixture, cx);
    let (mut sequence, mut report) = (0_u64, 0_u64);
    script_bars(
        &fixture,
        cx,
        &feeds,
        NEAR,
        100,
        5,
        100,
        &mut sequence,
        &mut report,
    );
    let range = |fixture: &Fixture, cx: &mut TestAppContext| {
        panel(fixture).read_with(cx, |panel, _| panel.view_range())
    };
    assert_eq!(
        range(&fixture, cx).resolve(100),
        (20, 100),
        "the last 80 by default"
    );
    assert!(range(&fixture, cx).is_following(100));

    wheel(&fixture, cx, -3.);
    let zoomed = range(&fixture, cx).resolve(100);
    assert_eq!(zoomed.1 - zoomed.0, 60, "wheel up zooms in: {zoomed:?}");
    assert!(
        (28..=34).contains(&zoomed.0),
        "the anchored middle bar stays put: {zoomed:?}"
    );

    drag_plot(&fixture, cx, 240.);
    let panned = range(&fixture, cx);
    assert!(
        !panned.is_following(100),
        "dragging right looks back: {:?}",
        panned.resolve(100)
    );
    assert!(
        visible(&fixture, cx, "chart-follow"),
        "the way back is offered"
    );

    double_click_plot(&fixture, cx);
    assert!(
        range(&fixture, cx).is_following(100),
        "double-click follows again"
    );
    assert!(!visible(&fixture, cx, "chart-follow"));

    drag_plot(&fixture, cx, 240.);
    assert!(!range(&fixture, cx).is_following(100));
    click(&fixture, cx, "chart-follow");
    assert!(
        range(&fixture, cx).is_following(100),
        "the button follows too"
    );
}

#[gpui_kit::test]
fn a_second_security_switches_the_pane_to_comparison(cx: &mut TestAppContext) {
    let fixture = open_panel(cx);
    let feeds = connect(&fixture, cx);
    let (mut sequence, mut report) = (0_u64, 0_u64);
    script_bars(
        &fixture,
        cx,
        &feeds,
        NEAR,
        12,
        5,
        100,
        &mut sequence,
        &mut report,
    );
    let (mut other_sequence, mut other_report) = (1_000_u64, 0_u64);
    script_bars(
        &fixture,
        cx,
        &feeds,
        OTHER,
        12,
        5,
        50,
        &mut other_sequence,
        &mut other_report,
    );
    assert!(
        !panel(&fixture).read_with(cx, ChartPanel::comparison),
        "one series is a price chart"
    );
    click(&fixture, cx, "chart-add");
    click_menu_item(&fixture, cx, "XYZ");
    assert_eq!(
        panel(&fixture).read_with(cx, |panel, _| panel.listings()),
        vec![NEAR, OTHER]
    );
    assert!(
        panel(&fixture).read_with(cx, ChartPanel::comparison),
        "two securities compare in percent"
    );
    assert!(
        visible(&fixture, cx, "chart-chip-1-9"),
        "XYZ has its own chip"
    );
}

#[gpui_kit::test]
fn another_venue_of_the_same_security_overlays_on_the_price_axis(cx: &mut TestAppContext) {
    let fixture = open_panel(cx);
    let feeds = connect(&fixture, cx);
    assert_eq!(
        panel(&fixture).read_with(cx, |panel, _| panel.listings()),
        vec![NEAR],
        "venue 1 only on open"
    );
    let (mut sequence, mut report) = (0_u64, 0_u64);
    script_bars(
        &fixture,
        cx,
        &feeds,
        NEAR,
        12,
        5,
        100,
        &mut sequence,
        &mut report,
    );
    script_bars(
        &fixture,
        cx,
        &feeds,
        FAR,
        12,
        5,
        101,
        &mut sequence,
        &mut report,
    );
    click(&fixture, cx, "chart-add");
    click_menu_item(&fixture, cx, "BNT");
    assert_eq!(
        panel(&fixture).read_with(cx, |panel, _| panel.listings()),
        vec![NEAR, FAR],
        "adding BNT again overlays venue 2"
    );
    assert!(
        !panel(&fixture).read_with(cx, ChartPanel::comparison),
        "same instrument: no percent normalisation"
    );
    assert!(visible(&fixture, cx, "chart-price"), "candles stay");
    assert!(
        visible(&fixture, cx, "chart-chip-2-7"),
        "venue 2 has its chip"
    );
}

fn real_fill(
    fixture: &Fixture,
    cx: &mut TestAppContext,
    sequence: u64,
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
                logical_time_ns: sequence * SEC,
            },
            client_order_id: sequence,
            order_id: u128::from(sequence),
            listing: NEAR,
            side,
            price,
            quantity,
            fee,
            liquidity: Liquidity::Taker,
        }),
    );
}

#[gpui_kit::test]
fn portfolio_and_the_pnl_window_show_the_same_derived_figures(cx: &mut TestAppContext) {
    let fixture = open_workspace(cx);
    let _feeds = connect(&fixture, cx);
    // Buy 10 at 100 (fee 2), sell 4 at 103 (rebate 1): realized
    // 12 - 2 + 1 = 11 at contract size 1.
    real_fill(&fixture, cx, 1, Side::Buy, 100, 10, 2);
    real_fill(&fixture, cx, 2, Side::Sell, 103, 4, -1);
    let totals = fixture
        .desk
        .read_with(cx, |desk, _| desk.model().pnl_totals());
    assert_eq!(totals.realized, 11);
    assert!(!totals.multipliers_known, "BNP sends no contract size yet");

    assert!(visible(&fixture, cx, "pf-pnl-realized"), "the P&L stats");
    assert!(visible(&fixture, cx, "pf-pnl-chart"), "the P&L chart");
    assert!(
        visible(&fixture, cx, "positions-table"),
        "holdings stay visible"
    );
    click(&fixture, cx, "pf-pnl-toggle");
    assert!(
        !visible(&fixture, cx, "pf-pnl-realized"),
        "the section collapses"
    );
    click(&fixture, cx, "pf-pnl-toggle");
    assert!(
        visible(&fixture, cx, "pf-pnl-realized"),
        "and expands again"
    );
    let workspace = fixture.workspace.clone().expect("a workspace root");
    cx.update_window(fixture.window.into(), |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.add_pane(PaneKind::Pnl, window, cx);
        });
    })
    .unwrap();
    cx.run_until_parked();
    assert!(visible(&fixture, cx, "pnl-chart"), "the P&L Chart window");
    assert!(visible(&fixture, cx, "pnl-total"));
}

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

/// A bare canvas window showing `saved` windows over the Trading
/// template, settled over a few frames.
fn open_canvas(
    cx: &mut TestAppContext,
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
    let template = bunting_desk::Preset::Trading;
    let window = cx.update(|cx| {
        let (window, _) = gpui_kit::open_window(window_options(), cx, |window, cx| {
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
    (
        Fixture {
            window,
            desk,
            panel: None,
            workspace: None,
            sent,
        },
        canvas,
    )
}

#[gpui_kit::test]
fn chart_state_survives_a_saved_screen_round_trip(cx: &mut TestAppContext) {
    // Layout-v4 records without the state field still load.
    let old: bunting_desk::canvas::SavedWindow = serde_json::from_value(serde_json::json!({
        "kind": "chart",
        "rect": {"x": 0.0, "y": 0.0, "w": 720.0, "h": 480.0},
        "z": 0,
        "minimised": false,
        "maximised": false
    }))
    .expect("a v4 saved window");
    assert!(old.state.is_none());

    let (fixture, canvas) = open_canvas(cx, None);
    connect(&fixture, cx);
    cx.update_window(fixture.window.into(), |_, window, cx| {
        canvas.update(cx, |canvas, cx| {
            canvas.open(PaneKind::Chart, window, cx);
        });
        window.render_frame(cx);
    })
    .unwrap();
    // Overlay venue 2 so the saved state holds two series.
    click(&fixture, cx, "chart-add");
    click_menu_item(&fixture, cx, "BNT");
    cx.run_until_parked();
    let saved = canvas.read_with(cx, bunting_desk::Canvas::dump);
    let chart = saved
        .iter()
        .find(|each| each.kind == "chart")
        .expect("a saved chart window");
    let state = chart.state.clone().expect("the chart dumps its state");
    assert_eq!(state["bar_ns"], 5 * SEC, "the starting bar size is saved");
    assert_eq!(
        state["listings"],
        serde_json::json!([["1", "7"], ["2", "7"]]),
        "each venue is saved as its own series"
    );

    let (_fixture, canvas) = open_canvas(cx, Some(&saved));
    let again = canvas.read_with(cx, bunting_desk::Canvas::dump);
    let restored = again
        .iter()
        .find(|each| each.kind == "chart")
        .expect("the chart window restored");
    assert_eq!(restored.state, chart.state, "state round-trips unchanged");
    assert_eq!(restored.rect, chart.rect);
}
