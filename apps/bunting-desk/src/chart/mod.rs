//! RIT's Charting module (`docs/specs/rit-desk-window-parity.md`):
//! one window kind. It charts any security's
//! bars — candlesticks by default, a line on request — with volume
//! below and MA/EMA overlays and an RSI sub-pane on request; each
//! venue's listing is its own series, never merged (ADR 0050
//! decision 7). The trader's own P&L has its own window and a
//! section of Portfolio ([`pnl`]). History starts when this desk
//! connected; the window says so (the parity spec's BNP history
//! gap).
//!
//! The price plot is [`price_chart::PriceChart`] and the P&L plot
//! is [`pnl::PnlChart`], both composed from Longbridge's plot
//! primitives (the finished 0.7.1 chart components have no price
//! axis, zoom or pan, and their area fill cannot baseline at
//! zero).
#![allow(
    clippy::float_arithmetic,
    clippy::cast_precision_loss,
    reason = "chart coordinates and indicator math are f64 by the components' own API"
)]

pub mod pnl;
pub mod price_chart;
pub mod range;

use crate::desk::Desk;
use crate::format;
use crate::ui;
use bunting_trading_state::bunting_client::bnp_wire::Listing;
use bunting_trading_state::{BAR_SIZES_NS, Bar as MarketBar, FINEST_BAR_NS, Instrument};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, PanelInfo, PanelState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Div, Entity, EventEmitter, FocusHandle,
    Focusable, Hsla, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, Render, ScrollDelta,
    ScrollWheelEvent, SharedString, Styled as _, Subscription, TestSupportExt as _, Window, div,
    px,
};
use price_chart::{OtherSeries, PriceChart};
use range::{DEFAULT_BAR, ViewRange, percent_changes, pick_bar_size};
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::rc::Rc;

/// Securities on one chart at most: the fixed categorical palette
/// has four colours and identity is never carried by a fifth hue
/// borrowed from the up/down pair.
const MAX_SERIES: usize = 4;
/// Empty band slots after the last bar while following.
const FOLLOW_MARGIN: usize = 3;
/// Periods of the moving averages and the RSI (RIT's defaults).
const MA_PERIOD: usize = 20;
const RSI_PERIOD: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PriceKind {
    Candles,
    Line,
}

/// Which of the chart's indicators are on.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Indicators {
    ma: bool,
    ema: bool,
    rsi: bool,
}

/// One chosen series: a venue's listing, with the palette slot it
/// took when it was added. The colour follows the listing, never
/// its place in the row, so removing one chip does not repaint the
/// survivors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SeriesChoice {
    listing: Listing,
    color: usize,
}

/// The chart's own settings as saved in a screen (canvas
/// `SavedWindow.state`): plain data, restored by
/// [`ChartPanel::restore`]. Listing IDs are decimal strings because
/// JSON numbers do not hold a `u128`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flat saved record of independent on/off choices"
)]
pub struct SavedChart {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    candles: Option<bool>,
    #[serde(default)]
    bar_ns: Option<u64>,
    /// The user picked the bar size; otherwise it is auto-picked
    /// from trade density and `bar_ns` is just where it landed.
    #[serde(default)]
    bar_picked: bool,
    /// `[venue_id, instrument_id]` per chosen series, in chip order.
    #[serde(default)]
    listings: Vec<[String; 2]>,
    #[serde(default)]
    ma: bool,
    #[serde(default)]
    ema: bool,
    #[serde(default)]
    rsi: bool,
}

/// The fixed categorical series palette, in order, from theme
/// tokens. The chart.1–5 tokens are one blue ramp (a sequential
/// scale), so identity uses distinct base hues; the bullish and
/// bearish pair stays reserved for up and down candles.
fn series_color(slot: usize, cx: &App) -> Hsla {
    let theme = cx.theme();
    match slot % MAX_SERIES {
        0 => theme.chart_2,
        1 => theme.yellow,
        2 => theme.magenta,
        _ => theme.cyan,
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "prices and volumes are drawn, not settled, as f64"
)]
fn num(value: i64) -> f64 {
    value as f64
}

/// Simple moving average of the last `period` closes ending at each
/// bar. The warm-up bars average what exists so far: a point scale
/// cannot leave the first bars blank and still line the rest up
/// with the candles under them.
fn sma(closes: &[f64], period: usize) -> Vec<f64> {
    let period = period.max(1);
    let mut sum = 0.;
    closes
        .iter()
        .enumerate()
        .map(|(index, close)| {
            sum += close;
            if index >= period {
                sum -= closes[index - period];
            }
            let count = (index + 1).min(period);
            #[allow(
                clippy::cast_precision_loss,
                reason = "a bar count, far inside f64's exact integers"
            )]
            {
                sum / count as f64
            }
        })
        .collect()
}

/// Exponential moving average with the usual `2 / (period + 1)`
/// smoothing, seeded on the first close.
fn ema(closes: &[f64], period: usize) -> Vec<f64> {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a period, far inside f64's exact integers"
    )]
    let alpha = 2. / (period.max(1) as f64 + 1.);
    let mut average: Option<f64> = None;
    closes
        .iter()
        .map(|close| {
            let next = average.map_or(*close, |held| held + alpha * (close - held));
            average = Some(next);
            next
        })
        .collect()
}

/// RSI with Wilder smoothing. The first bar has no change and
/// reads 50, the neutral midpoint; warm-up averages use the
/// changes that exist.
#[allow(
    clippy::cast_precision_loss,
    reason = "periods and bar counts, far inside f64's exact integers"
)]
fn rsi(closes: &[f64], period: usize) -> Vec<f64> {
    let period_f = period.max(1) as f64;
    let mut values = Vec::with_capacity(closes.len());
    let (mut gain, mut loss) = (0., 0.);
    for (index, close) in closes.iter().enumerate() {
        if index > 0 {
            let change = close - closes[index - 1];
            let (up, down) = (change.max(0.), (-change).max(0.));
            if index == 1 {
                (gain, loss) = (up, down);
            } else {
                let span = period_f.min((index) as f64);
                gain = gain + (up - gain) / span;
                loss = loss + (down - loss) / span;
            }
        }
        values.push(if loss == 0. {
            if gain == 0. { 50. } else { 100. }
        } else {
            100. - 100. / (1. + gain / loss)
        });
    }
    values
}

/// Another series' closes lined up on the primary series' bar
/// starts: a bar the other venue did not trade in carries its last
/// close forward (the first close backwards to the start), because
/// the band scale has one slot per primary bar and no gaps.
fn aligned_closes(primary: &[MarketBar], other: &[MarketBar]) -> Option<Vec<f64>> {
    let mut held: Option<f64> = None;
    let mut next = other.iter().peekable();
    let mut closes = Vec::with_capacity(primary.len());
    for bar in primary {
        while next
            .peek()
            .is_some_and(|other| other.start_ns <= bar.start_ns)
        {
            held = next.next().map(|other| num(other.close));
        }
        closes.push(held.or_else(|| other.first().map(|other| num(other.close)))?);
    }
    Some(closes)
}

/// A drag-pan in progress: where it started, and the range and
/// geometry it started from.
#[derive(Clone, Copy)]
struct Drag {
    start_x: f32,
    range: ViewRange,
    slot_px: f32,
    total: usize,
}

pub struct ChartPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    ticker: Entity<InputState>,
    kind: PriceKind,
    /// Index into [`BAR_SIZES_NS`].
    bar: usize,
    series: Vec<SeriesChoice>,
    indicators: Indicators,
    /// The desk's instrument has been taken as the first series
    /// (or a saved screen named the series): never seed again.
    seeded: bool,
    /// Why the last ticker did not add, shown beside the field.
    problem: Option<String>,
    /// Bars on screen: zoom, pan and follow state.
    view: ViewRange,
    /// The user picked the bar size (stops auto-picking).
    bar_picked: bool,
    /// The listing the automatic bar size was picked for.
    picked_for: Option<Listing>,
    /// Primary bars at the current size, refreshed each render
    /// for the zoom/pan handlers.
    last_total: usize,
    drag: Option<Drag>,
    /// Plot area bounds and measured right gutter, shared with
    /// the plot element each frame.
    plot_bounds: Rc<Cell<Bounds<Pixels>>>,
    plot_gutter: Rc<Cell<f32>>,
    _observe: Subscription,
    _ticker_enter: Subscription,
}

impl ChartPanel {
    pub fn new(desk: Entity<Desk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let ticker = cx.new(|cx| InputState::new(window, cx).placeholder("Ticker"));
        let enter = cx.subscribe_in(
            &ticker,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.add_ticker(window, cx);
                }
            },
        );
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            ticker,
            kind: PriceKind::Candles,
            bar: DEFAULT_BAR,
            series: Vec::new(),
            indicators: Indicators::default(),
            seeded: false,
            problem: None,
            view: ViewRange::default(),
            bar_picked: false,
            picked_for: None,
            last_total: 0,
            drag: None,
            plot_bounds: Rc::new(Cell::new(Bounds::default())),
            plot_gutter: Rc::new(Cell::new(56.)),
            _ticker_enter: enter,
        }
    }

    /// The saved form of this window's choices.
    #[must_use]
    pub fn saved(&self) -> SavedChart {
        SavedChart {
            // Written by the two-mode chart this replaced; always
            // "security" now, and ignored on restore.
            mode: Some("security".to_owned()),
            candles: Some(self.kind == PriceKind::Candles),
            bar_ns: Some(self.bar_ns()),
            bar_picked: self.bar_picked,
            listings: self
                .series
                .iter()
                .map(|choice| {
                    [
                        choice.listing.venue_id.to_string(),
                        choice.listing.instrument_id.to_string(),
                    ]
                })
                .collect(),
            ma: self.indicators.ma,
            ema: self.indicators.ema,
            rsi: self.indicators.rsi,
        }
    }

    /// Takes back the choices [`Self::saved`] wrote. Listings the
    /// venue no longer lists are kept: their chips name the venue
    /// and the chart is empty until the listing returns.
    pub fn restore(&mut self, saved: &SavedChart) {
        self.kind = if saved.candles.unwrap_or(true) {
            PriceKind::Candles
        } else {
            PriceKind::Line
        };
        if let Some(bar) = saved
            .bar_ns
            .and_then(|ns| BAR_SIZES_NS.iter().position(|&size| size == ns))
        {
            self.bar = bar;
        }
        self.bar_picked = saved.bar_picked;
        self.view = ViewRange::default();
        self.series.clear();
        for [venue, instrument] in &saved.listings {
            if let (Ok(venue_id), Ok(instrument_id)) =
                (venue.parse::<u128>(), instrument.parse::<u128>())
            {
                self.push_series(Listing {
                    venue_id,
                    instrument_id,
                });
            }
        }
        self.indicators = Indicators {
            ma: saved.ma,
            ema: saved.ema,
            rsi: saved.rsi,
        };
        self.seeded = true;
    }

    // ---- reading, for the window tests --------------------------------

    /// The chosen series' listings, in chip order; the first is the
    /// primary series the candles and indicators read.
    #[must_use]
    pub fn listings(&self) -> Vec<Listing> {
        self.series.iter().map(|choice| choice.listing).collect()
    }

    /// The bar size in run-time nanoseconds.
    #[must_use]
    pub const fn bar_ns(&self) -> u64 {
        BAR_SIZES_NS[self.bar]
    }

    /// `(MA, EMA, RSI)` on or off.
    #[must_use]
    pub const fn indicators(&self) -> (bool, bool, bool) {
        (self.indicators.ma, self.indicators.ema, self.indicators.rsi)
    }

    /// The visible-bar range (zoom, pan, follow).
    #[must_use]
    pub const fn view_range(&self) -> ViewRange {
        self.view
    }

    /// Whether two or more series with data put the price pane in
    /// comparison (percent-change) mode.
    #[must_use]
    pub fn comparison(&self, cx: &App) -> bool {
        self.series.len() > 1
            && self.series.iter().skip(1).any(|choice| {
                !self
                    .desk
                    .read(cx)
                    .model()
                    .bars(choice.listing, self.bar_ns())
                    .is_empty()
            })
    }

    // ---- changing the series -------------------------------------------

    fn push_series(&mut self, listing: Listing) -> bool {
        if self.series.len() == MAX_SERIES
            || self.series.iter().any(|choice| choice.listing == listing)
        {
            return false;
        }
        let color = (0..MAX_SERIES)
            .find(|slot| self.series.iter().all(|choice| choice.color != *slot))
            .unwrap_or(0);
        self.series.push(SeriesChoice { listing, color });
        true
    }

    /// Adds every venue's listing of `instrument`, each as its own
    /// series: a security on two venues is two series labelled by
    /// venue, never one merged line.
    fn add_instrument(&mut self, instrument: &Instrument, cx: &mut Context<Self>) {
        for info in &instrument.listings {
            let already = self
                .series
                .iter()
                .any(|choice| choice.listing == info.listing);
            if !already && !self.push_series(info.listing) {
                self.problem = Some(format!(
                    "{MAX_SERIES} series at most; {} venue {} left off",
                    instrument.symbol, info.listing.venue_id
                ));
                break;
            }
        }
        self.seeded = true;
        cx.notify();
    }

    /// Enter in the ticker field: add the named security's venues.
    fn add_ticker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.ticker.read(cx).value().trim().to_owned();
        if typed.is_empty() {
            return;
        }
        let instrument = self
            .desk
            .read(cx)
            .instruments()
            .into_iter()
            .find(|instrument| instrument.symbol.eq_ignore_ascii_case(&typed));
        if let Some(instrument) = instrument {
            self.problem = None;
            self.ticker.update(cx, |state, cx| {
                state.set_value(String::new(), window, cx);
            });
            self.add_instrument(&instrument, cx);
        } else {
            self.problem = Some(format!("no security named {typed}"));
            cx.notify();
        }
    }

    fn remove_series(&mut self, listing: Listing, cx: &mut Context<Self>) {
        self.series.retain(|choice| choice.listing != listing);
        cx.notify();
    }

    /// The first time the window can, it charts the desk's current
    /// instrument, so opening it over a live market draws at once.
    fn seed(&mut self, cx: &mut Context<Self>) {
        if self.seeded || !self.series.is_empty() {
            return;
        }
        if let Some(instrument) = self.desk.read(cx).instrument() {
            for info in &instrument.listings {
                self.push_series(info.listing);
            }
            self.seeded = true;
        }
    }

    /// Per-render upkeep: seed the desk's instrument, auto-pick
    /// the bar size for a new primary series (until the user picks
    /// their own), and count its bars for the zoom/pan handlers.
    fn prepare(&mut self, cx: &mut Context<Self>) {
        self.seed(cx);
        let Some(primary) = self.series.first().map(|choice| choice.listing) else {
            self.last_total = 0;
            return;
        };
        let desk = self.desk.read(cx);
        let model = desk.model();
        if !self.bar_picked && self.picked_for != Some(primary) {
            let fine = model.bars(primary, FINEST_BAR_NS);
            if let (Some(first), Some(last)) = (fine.first(), fine.last()) {
                self.bar = pick_bar_size(fine.len(), last.start_ns.saturating_sub(first.start_ns));
                self.picked_for = Some(primary);
            }
        }
        self.last_total = model.bars(primary, self.bar_ns()).len();
    }

    /// Plot width inside the axis gutter, from the last frame.
    fn plot_width(&self) -> f32 {
        (self.plot_bounds.get().size.width.as_f32() - self.plot_gutter.get()).max(1.)
    }

    fn zoom_at(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let bounds = self.plot_bounds.get();
        let fraction =
            ((event.position.x - bounds.origin.x).as_f32() / self.plot_width()).clamp(0., 1.);
        let steps = match event.delta {
            ScrollDelta::Lines(point) => f64::from(point.y),
            ScrollDelta::Pixels(point) => f64::from(point.y.as_f32()) / 50.,
        };
        self.view
            .zoom(self.last_total, f64::from(fraction), 1.1_f64.powf(steps));
        cx.notify();
    }

    fn drag_start(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        if event.click_count >= 2 {
            self.view.follow_latest();
            cx.notify();
            return;
        }
        let (start, end) = self.view.resolve(self.last_total);
        let slots = (end - start)
            + if self.view.is_following(self.last_total) {
                FOLLOW_MARGIN
            } else {
                0
            };
        self.drag = Some(Drag {
            start_x: event.position.x.as_f32(),
            range: self.view,
            slot_px: self.plot_width() / slots.max(1) as f32,
            total: self.last_total,
        });
    }

    fn drag_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        let Some(drag) = self.drag else {
            return;
        };
        if drag.slot_px <= 0. {
            return;
        }
        #[allow(
            clippy::cast_possible_truncation,
            reason = "a pan distance in bars, clamped by ViewRange::pan"
        )]
        let bars = ((event.position.x.as_f32() - drag.start_x) / drag.slot_px).round() as isize;
        self.view = drag.range;
        self.view.pan(drag.total, -bars);
        cx.notify();
    }

    // ---- rendering ------------------------------------------------------

    /// The one toolbar row (owner review): ticker field, "+"
    /// security menu, "Ind:" indicator menu, then Candles|Line and
    /// "S:" bar size on the right.
    #[expect(
        clippy::too_many_lines,
        reason = "one toolbar row: ticker, two menus and two segmented controls"
    )]
    fn toolbar_row(&self, cx: &mut Context<Self>) -> Div {
        let instruments = self.desk.read(cx).instruments();
        let chosen: Vec<Listing> = self.listings();
        let panel = cx.entity();
        let add_menu = Button::new("chart-add")
            .icon(IconName::Plus)
            .ghost()
            .xsmall()
            .tooltip("Add a security")
            .dropdown_menu_with_anchor(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu;
                if instruments.is_empty() {
                    menu = menu.item(PopupMenuItem::new("No securities listed yet").disabled(true));
                }
                for instrument in &instruments {
                    let panel = panel.clone();
                    let picked = instrument.clone();
                    let all_on = instrument
                        .listings
                        .iter()
                        .all(|info| chosen.contains(&info.listing));
                    menu = menu.item(
                        PopupMenuItem::new(instrument.symbol.clone())
                            .checked(all_on)
                            .on_click(move |_, _, cx| {
                                panel.update(cx, |this, cx| {
                                    this.problem = None;
                                    this.add_instrument(&picked, cx);
                                });
                            }),
                    );
                }
                menu
            });
        let panel = cx.entity();
        let (ma, ema, rsi_on) = (self.indicators.ma, self.indicators.ema, self.indicators.rsi);
        let ind_menu = Button::new("chart-ind")
            .icon(IconName::Plus)
            .ghost()
            .xsmall()
            .tooltip("Add an indicator")
            .dropdown_menu_with_anchor(gpui_kit::Anchor::TopLeft, move |menu, _, _| {
                let mut menu = menu;
                for (label, on, slot) in [
                    ("MA(20)", ma, 0_usize),
                    ("EMA(20)", ema, 1),
                    ("RSI(14)", rsi_on, 2),
                ] {
                    let panel = panel.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(on).on_click(
                        move |_, _, cx| {
                            panel.update(cx, |this, cx| {
                                match slot {
                                    0 => this.indicators.ma = !this.indicators.ma,
                                    1 => this.indicators.ema = !this.indicators.ema,
                                    _ => this.indicators.rsi = !this.indicators.rsi,
                                }
                                cx.notify();
                            });
                        },
                    ));
                }
                menu
            });
        let mut row = ui::toolbar(cx)
            .w_full()
            .child(
                div()
                    .id("chart-ticker")
                    .test_support()
                    .w(px(110.))
                    .child(Input::new(&self.ticker).xsmall()),
            )
            .child(add_menu)
            .child(ui::muted("Ind:", cx))
            .child(ind_menu);
        if let Some(problem) = &self.problem {
            row = row.child(
                div()
                    .id("chart-problem")
                    .test_support()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().warning)
                    .child(problem.clone()),
            );
        }
        let mut bars = ButtonGroup::new("chart-bars").xsmall();
        for (index, label) in ["1s", "5s", "15s", "1m", "5m"].into_iter().enumerate() {
            bars = bars.child(ui::segment(
                SharedString::from(format!("chart-bar-{label}")),
                label,
                self.bar == index,
            ));
        }
        row = row
            .child(div().flex_1())
            .child(div().w(px(1.)).h(px(14.)).bg(cx.theme().border))
            .child(
                ButtonGroup::new("chart-kind")
                    .xsmall()
                    .child(ui::segment(
                        "chart-candles",
                        "Candles",
                        self.kind == PriceKind::Candles,
                    ))
                    .child(ui::segment(
                        "chart-line",
                        "Line",
                        self.kind == PriceKind::Line,
                    ))
                    .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                        this.kind = if clicks.contains(&0) {
                            PriceKind::Candles
                        } else {
                            PriceKind::Line
                        };
                        cx.notify();
                    })),
            )
            .child(ui::muted("S:", cx))
            .child(
                bars.on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    if let Some(&index) = clicks.first() {
                        this.bar = index.min(BAR_SIZES_NS.len() - 1);
                        this.bar_picked = true;
                    }
                    cx.notify();
                })),
            );

        row
    }

    /// The chosen series and indicators as removable chips, the
    /// legend for two or more series: colour dot, symbol, venue
    /// where the security lists on more than one, and last price.
    #[expect(
        clippy::too_many_lines,
        reason = "the series and indicator chips are one legend row with the same removable chip shape"
    )]
    fn chips(&self, cx: &mut Context<Self>) -> Div {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let instruments = desk.instruments();
        let muted = cx.theme().muted;
        let foreground = cx.theme().foreground;
        let panel = cx.entity();
        let mut row = h_flex().w_full().flex_wrap().gap_1().items_center();
        for choice in &self.series {
            let listing = choice.listing;
            let instrument = instruments
                .iter()
                .find(|each| each.instrument_id == listing.instrument_id);
            let mut label = instrument.map_or_else(
                || format!("Instrument {}", listing.instrument_id),
                |each| each.symbol.clone(),
            );
            if instrument.is_some_and(|each| each.listings.len() > 1) {
                label = format!("{label} · V{}", listing.venue_id);
            }
            if let Some(last) = model.last_price(listing) {
                label = format!("{label} · {}", format::price(Some(last)));
            }
            let remove = panel.clone();
            row = row.child(
                h_flex()
                    .id(SharedString::from(format!(
                        "chart-chip-{}-{}",
                        listing.venue_id, listing.instrument_id
                    )))
                    .test_support()
                    .h(px(22.))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded(px(4.))
                    .bg(muted)
                    .child(ui::status_dot(series_color(choice.color, cx)))
                    .child(
                        div()
                            .text_size(ui::rem(ui::LABEL_SIZE))
                            .text_color(foreground)
                            .child(label),
                    )
                    .child(
                        Button::new(SharedString::from(format!(
                            "chart-remove-{}-{}",
                            listing.venue_id, listing.instrument_id
                        )))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip("Remove this series")
                        .on_click(move |_, _, cx| {
                            remove.update(cx, |this, cx| this.remove_series(listing, cx));
                        }),
                    ),
            );
        }
        let closes: Vec<f64> = self
            .series
            .first()
            .map(|primary| {
                model
                    .bars(primary.listing, self.bar_ns())
                    .iter()
                    .map(|bar| num(bar.close))
                    .collect()
            })
            .unwrap_or_default();
        let value = |values: Vec<f64>| {
            values
                .last()
                .map(|last| format!(" {last:.1}"))
                .unwrap_or_default()
        };
        let ma_label = format!("MA(20){}", value(sma(&closes, MA_PERIOD)));
        let ema_label = format!("EMA(20){}", value(ema(&closes, MA_PERIOD)));
        let rsi_label = format!("RSI(14){}", value(rsi(&closes, RSI_PERIOD)));
        for (label, on, slot, color) in [
            (ma_label, self.indicators.ma, 0_usize, cx.theme().foreground),
            (
                ema_label,
                self.indicators.ema,
                1,
                cx.theme().muted_foreground,
            ),
            (rsi_label, self.indicators.rsi, 2, cx.theme().chart_3),
        ] {
            if !on {
                continue;
            }
            let remove = panel.clone();
            row = row.child(
                h_flex()
                    .id(SharedString::from(format!("chart-ind-chip-{slot}")))
                    .test_support()
                    .h(px(22.))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded(px(4.))
                    .bg(muted)
                    .child(ui::status_dot(color))
                    .child(
                        div()
                            .text_size(ui::rem(ui::LABEL_SIZE))
                            .text_color(foreground)
                            .child(label),
                    )
                    .child(
                        Button::new(SharedString::from(format!("chart-ind-remove-{slot}")))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .tooltip("Remove this indicator")
                            .on_click(move |_, _, cx| {
                                remove.update(cx, |this, cx| {
                                    match slot {
                                        0 => this.indicators.ma = false,
                                        1 => this.indicators.ema = false,
                                        _ => this.indicators.rsi = false,
                                    }
                                    cx.notify();
                                });
                            }),
                    ),
            );
        }
        row
    }

    /// The Security mode's plot: [`PriceChart`] over the visible
    /// range, inside the wrapper that owns wheel zoom, drag pan
    /// and double-click follow. With two or more series that have
    /// data the pane switches to comparison (percent change).
    #[expect(
        clippy::too_many_lines,
        reason = "data preparation and the plot wrapper's handlers read best whole"
    )]
    fn security_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let Some(primary) = self.series.first().copied() else {
            return ui::empty_state(
                "chart-empty",
                IconName::Frame,
                "Type a ticker and press Enter, or add one with +",
                None,
                cx,
            )
            .into_any_element();
        };
        let symbol = model
            .symbol(primary.listing)
            .map_or_else(|| primary.listing.instrument_id.to_string(), str::to_owned);
        let all = model.bars(primary.listing, self.bar_ns());
        if all.is_empty() {
            return ui::empty_state(
                "chart-empty",
                IconName::Frame,
                format!("No trades yet for {symbol}"),
                None,
                cx,
            )
            .into_any_element();
        }
        let venue_suffix = |choice: &SeriesChoice| -> String {
            let name = model
                .symbol(choice.listing)
                .map_or_else(|| choice.listing.instrument_id.to_string(), str::to_owned);
            let multi = self
                .series
                .iter()
                .filter(|each| each.listing.instrument_id == choice.listing.instrument_id)
                .count()
                > 1;
            if multi {
                format!("{name} · V{}", choice.listing.venue_id)
            } else {
                name
            }
        };
        let closes: Vec<f64> = all.iter().map(|bar| num(bar.close)).collect();
        let (start, end) = self.view.resolve(all.len());
        let slice = |values: &[f64]| values[start..end].to_vec();
        let mut others: Vec<(SeriesChoice, Vec<f64>)> = Vec::new();
        for choice in self.series.iter().skip(1) {
            let bars = model.bars(choice.listing, self.bar_ns());
            if let Some(aligned) = aligned_closes(&all, &bars) {
                others.push((*choice, aligned));
            }
        }
        let comparison = !others.is_empty();
        let mut chart = PriceChart::new(
            SharedString::from(format!(
                "price-{}-{}",
                primary.listing.venue_id, primary.listing.instrument_id
            )),
            all[start..end].to_vec(),
            self.plot_gutter.clone(),
        )
        .candles(self.kind == PriceKind::Candles)
        .comparison(comparison)
        .name(venue_suffix(&primary))
        .primary_color(series_color(primary.color, cx))
        .margin_slots(if self.view.is_following(all.len()) {
            FOLLOW_MARGIN
        } else {
            0
        })
        .rsi(
            self.indicators
                .rsi
                .then(|| slice(&rsi(&closes, RSI_PERIOD))),
        );
        if comparison {
            let mut lines = vec![OtherSeries {
                name: SharedString::from(venue_suffix(&primary)),
                values: percent_changes(&slice(&closes), closes[start]),
                color: series_color(primary.color, cx),
            }];
            for (choice, aligned) in &others {
                lines.push(OtherSeries {
                    name: SharedString::from(venue_suffix(choice)),
                    values: percent_changes(&slice(aligned), aligned[start]),
                    color: series_color(choice.color, cx),
                });
            }
            chart = chart.others(lines);
        } else {
            if self.indicators.ma {
                chart = chart.ma(Some(slice(&sma(&closes, MA_PERIOD))));
            }
            if self.indicators.ema {
                chart = chart.ema(Some(slice(&ema(&closes, MA_PERIOD))));
            }
        }
        let bounds_cell = self.plot_bounds.clone();
        let following = self.view.is_following(all.len());
        let mut area = div()
            .id("chart-price")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .on_scroll_wheel(cx.listener(|this, event, _, cx| this.zoom_at(event, cx)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event, _, cx| this.drag_start(event, cx)),
            )
            .on_mouse_move(cx.listener(|this, event, _, cx| this.drag_move(event, cx)))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, _| this.drag = None),
            )
            .child(
                gpui_kit::canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    |_, (), _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(chart);
        if !following {
            area = area.child(
                div()
                    .absolute()
                    .bottom(px(24.))
                    .right(px(self.plot_gutter.get() + 8.))
                    .child(
                        Button::new("chart-follow")
                            .ghost()
                            .xsmall()
                            .label("Follow latest")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.view.follow_latest();
                                cx.notify();
                            })),
                    ),
            );
        }
        area.into_any_element()
    }
}

impl Render for ChartPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.prepare(cx);
        let any_indicator = self.indicators.ma || self.indicators.ema || self.indicators.rsi;
        ui::pane_body("chart", &self.focus)
            .child(self.toolbar_row(cx))
            .when(!self.series.is_empty() || any_indicator, |body| {
                body.child(self.chips(cx))
            })
            .child(self.security_body(cx))
            .child(ui::note(
                "chart-history",
                "History starts when the desk connected",
                "Bars are built from trades this desk has seen since it connected; \
                 earlier history is not on BNP yet. Each venue is its own series.",
                cx,
            ))
    }
}

// The dock-panel boilerplate every pane shares (see
// `panels::dock_panel!`), written out because this panel also
// dumps its own state.
impl BasePanel for ChartPanel {
    fn panel_name(&self) -> &'static str {
        "chart"
    }

    fn dump(&self, _: &App) -> PanelState {
        PanelState {
            panel_name: "chart".to_owned(),
            children: Vec::new(),
            info: PanelInfo::Panel(
                serde_json::to_value(self.saved()).unwrap_or(serde_json::Value::Null),
            ),
        }
    }
}

impl Panel for ChartPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::theme::pane_title("Charting", None)
    }
}

impl EventEmitter<PanelEvent> for ChartPanel {}

impl Focusable for ChartPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "the indicator fixtures use integers and exact halves, so equality is the assertion"
)]
mod tests {
    use super::*;

    #[test]
    fn moving_averages_track_a_constant_series() {
        let closes = vec![10.; 30];
        assert!(sma(&closes, MA_PERIOD).iter().all(|v| *v == 10.));
        assert!(ema(&closes, MA_PERIOD).iter().all(|v| *v == 10.));
    }

    #[test]
    fn sma_averages_the_last_period_closes() {
        let closes: Vec<f64> = (1..=25).map(f64::from).collect();
        let values = sma(&closes, 20);
        // Warm-up: the mean of what exists.
        assert_eq!(values[0], 1.);
        assert_eq!(values[1], 1.5);
        // Bar 20 (index 19): mean of 1..=20. Bar 21: mean of 2..=21.
        assert_eq!(values[19], 10.5);
        assert_eq!(values[20], 11.5);
        assert_eq!(values[24], 15.5);
    }

    #[test]
    fn ema_smooths_toward_new_closes() {
        let values = ema(&[10., 20.], 20);
        assert_eq!(values[0], 10.);
        // 10 + (2/21) * (20 - 10), short of the new close.
        assert!(values[1] > 10. && values[1] < 11.5, "{}", values[1]);
    }

    #[test]
    fn rsi_is_100_when_only_rising_0_when_only_falling_50_when_flat() {
        let rising: Vec<f64> = (1..=20).map(f64::from).collect();
        assert_eq!(rsi(&rising, RSI_PERIOD).last(), Some(&100.));
        let falling: Vec<f64> = (1..=20).rev().map(f64::from).collect();
        assert_eq!(rsi(&falling, RSI_PERIOD).last(), Some(&0.));
        assert!(rsi(&[7.; 20], RSI_PERIOD).iter().all(|v| *v == 50.));
    }

    fn bar(start_ns: u64, close: i64) -> MarketBar {
        MarketBar {
            start_ns,
            open: close,
            high: close,
            low: close,
            close,
            volume: 1,
        }
    }

    #[test]
    fn other_series_closes_line_up_on_the_primary_bars() {
        let primary: Vec<MarketBar> = [0, 1, 2, 3]
            .into_iter()
            .map(|second| bar(second * 1_000_000_000, 100))
            .collect();
        // Traded in seconds 1 and 3 only: second 0 backfills the
        // first close, second 2 carries second 1's forward.
        let other = vec![bar(1_000_000_000, 50), bar(3_000_000_000, 55)];
        assert_eq!(
            aligned_closes(&primary, &other),
            Some(vec![50., 50., 50., 55.])
        );
        assert_eq!(aligned_closes(&primary, &[]), None);
    }
}
