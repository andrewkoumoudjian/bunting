//! RIT's Charting module (`docs/specs/rit-desk-window-parity.md`):
//! one window kind, two modes. Security mode charts any security's
//! bars — candlesticks by default, a line on request — with volume
//! below and MA/EMA overlays and an RSI sub-pane on request; each
//! venue's listing is its own series, never merged (ADR 0050
//! decision 7). P&L mode charts the trader's derived total P&L over
//! run time. History starts when this desk connected and the P&L is
//! derived from fills; both say so on the window (the parity spec's
//! BNP history and valuation gaps).
//!
//! Every mark is a gpui-kit chart component (`CandlestickChart`,
//! `LineChart`, `AreaChart`, `BarChart` in gpui-component 0.7.1);
//! nothing here paints its own candles, lines or axes. Where 0.7.1
//! has no composed answer — indicator overlays on candles, a price
//! axis on the candlestick chart, time-proportional x scales, pan
//! and zoom — components are composed instead and the seam is noted
//! where it happens.
#![allow(
    clippy::float_arithmetic,
    reason = "chart coordinates and indicator math are f64 by the components' own API"
)]

use crate::desk::Desk;
use crate::format;
use crate::ui;
use bunting_trading_state::bunting_client::bnp_wire::Listing;
use bunting_trading_state::{BAR_SIZES_NS, Bar as MarketBar, Instrument, PnlSample};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::chart::{AreaChart, BarChart, CandlestickChart, LineChart};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent, PanelInfo, PanelState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::plot::AxisLabelPlacement;
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Subscription, TestSupportExt as _, Window, div, px,
};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Securities on one chart at most: the fixed categorical palette
/// has four colours and identity is never carried by a fifth hue
/// borrowed from the up/down pair.
const MAX_SERIES: usize = 4;
/// Bars drawn at most: the latest ones. gpui-component 0.7.1 charts
/// have no pan or zoom, so older bars are one bar-size step away,
/// not one scroll away.
const MAX_VISIBLE_BARS: usize = 600;
/// Height of the volume sub-pane.
const VOLUME_HEIGHT: f32 = 56.;
/// Height of the RSI sub-pane.
const RSI_HEIGHT: f32 = 72.;
/// Height of the P&L sparkline [`pnl_sparkline`] makes for other
/// windows to place.
const SPARKLINE_HEIGHT: f32 = 36.;
/// Periods of the moving averages and the RSI (RIT's defaults).
const MA_PERIOD: usize = 20;
const RSI_PERIOD: usize = 14;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ChartMode {
    Security,
    Pnl,
}

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
pub struct SavedChart {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    candles: Option<bool>,
    #[serde(default)]
    bar_ns: Option<u64>,
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

/// A run-clock instant as `minutes:seconds`, minutes unbounded so
/// every second's label is unique on a band or point scale.
fn run_clock(nanos: u64) -> String {
    let seconds = nanos / 1_000_000_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[allow(
    clippy::cast_precision_loss,
    reason = "prices and volumes are drawn, not settled, as f64"
)]
fn num(value: i64) -> f64 {
    value as f64
}

#[allow(
    clippy::cast_precision_loss,
    reason = "prices and volumes are drawn, not settled, as f64"
)]
fn num_wide(value: i128) -> f64 {
    value as f64
}

/// The latest `MAX_VISIBLE_BARS` of `bars`.
fn visible(bars: &[MarketBar]) -> &[MarketBar] {
    &bars[bars.len().saturating_sub(MAX_VISIBLE_BARS)..]
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

/// One bar as the chart components read it.
#[derive(Clone)]
struct PriceRow {
    label: SharedString,
    bar: MarketBar,
}

impl PriceRow {
    fn of(bar: MarketBar) -> Self {
        Self {
            label: SharedString::from(run_clock(bar.start_ns)),
            bar,
        }
    }
}

/// One point of an overlay line, sharing the price rows' labels.
#[derive(Clone)]
struct LineRow {
    label: SharedString,
    value: f64,
}

/// `values` lined up on `rows`: one line point per price row.
fn line_rows(rows: &[PriceRow], values: &[f64]) -> Vec<LineRow> {
    rows.iter()
        .zip(values)
        .map(|(row, value)| LineRow {
            label: row.label.clone(),
            value: *value,
        })
        .collect()
}

/// Another series' closes lined up on the primary series' bar
/// starts: a bar the other venue did not trade in carries its last
/// close forward (the first close backwards to the start), because
/// a 0.7.1 point scale has no gaps and no time axis.
fn aligned_closes(primary: &[PriceRow], other: &[MarketBar]) -> Option<Vec<f64>> {
    let mut held: Option<f64> = None;
    let mut next = other.iter().peekable();
    let mut closes = Vec::with_capacity(primary.len());
    for row in primary {
        while next
            .peek()
            .is_some_and(|bar| bar.start_ns <= row.bar.start_ns)
        {
            held = next.next().map(|bar| num(bar.close));
        }
        closes.push(held.or_else(|| other.first().map(|bar| num(bar.close)))?);
    }
    Some(closes)
}

pub struct ChartPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    ticker: Entity<InputState>,
    mode: ChartMode,
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
            mode: ChartMode::Security,
            kind: PriceKind::Candles,
            bar: 0,
            series: Vec::new(),
            indicators: Indicators::default(),
            seeded: false,
            problem: None,
            _ticker_enter: enter,
        }
    }

    /// The saved form of this window's choices.
    #[must_use]
    pub fn saved(&self) -> SavedChart {
        SavedChart {
            mode: Some(
                match self.mode {
                    ChartMode::Security => "security",
                    ChartMode::Pnl => "pnl",
                }
                .to_owned(),
            ),
            candles: Some(self.kind == PriceKind::Candles),
            bar_ns: Some(self.bar_ns()),
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
        self.mode = match saved.mode.as_deref() {
            Some("pnl") => ChartMode::Pnl,
            _ => ChartMode::Security,
        };
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

    /// Whether the P&L mode is the one showing.
    #[must_use]
    pub const fn shows_pnl(&self) -> bool {
        matches!(self.mode, ChartMode::Pnl)
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

    // ---- rendering ------------------------------------------------------

    /// Row one: the mode, then (Security) candles-or-line and the
    /// bar size ("S:").
    fn mode_row(&self, cx: &mut Context<Self>) -> Div {
        let mut row = ui::toolbar(cx).w_full().child(
            ButtonGroup::new("chart-mode")
                .xsmall()
                .child(ui::segment(
                    "chart-mode-security",
                    "Security",
                    self.mode == ChartMode::Security,
                ))
                .child(ui::segment(
                    "chart-mode-pnl",
                    "P&L",
                    self.mode == ChartMode::Pnl,
                ))
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    this.mode = if clicks.contains(&0) {
                        ChartMode::Security
                    } else {
                        ChartMode::Pnl
                    };
                    cx.notify();
                })),
        );
        if self.mode == ChartMode::Security {
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
                        }
                        cx.notify();
                    })),
                );
        }
        row
    }

    /// Row two (Security): the ticker field, the "+" security menu,
    /// and "Ind:" with its "+" indicator menu.
    fn pick_row(&self, cx: &mut Context<Self>) -> Div {
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
        for (label, on, slot, color) in [
            ("MA(20)", self.indicators.ma, 0_usize, cx.theme().foreground),
            (
                "EMA(20)",
                self.indicators.ema,
                1,
                cx.theme().muted_foreground,
            ),
            ("RSI(14)", self.indicators.rsi, 2, cx.theme().chart_3),
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

    /// A line chart sharing the price plot: same labels, pinned to
    /// the price extent, no axes or grid of its own, and no hover —
    /// the chart underneath keeps the crosshair. Neither chart
    /// draws x labels in this pane (they sit under the lowest
    /// sub-pane), so both y scales span the same pixels: the
    /// candlestick scale's `[height, 10]` is the line scale's with
    /// `y_padding(10, 0)`.
    fn overlay(
        id: SharedString,
        data: Vec<LineRow>,
        color: Hsla,
        name: impl Into<SharedString>,
        extent: (f64, f64),
    ) -> Div {
        div().absolute().inset_0().child(
            LineChart::new(data)
                .id(id)
                .x(|row| row.label.clone())
                .y(|row| row.value)
                .stroke(color)
                .linear()
                .name(name)
                .y_domain(extent.0, extent.1)
                .y_padding(10., 0.)
                .x_axis(false)
                .y_axis(false)
                .grid(false)
                .interactive(false)
                .appear(false),
        )
    }

    /// The Security mode's chart stack: price, volume under it,
    /// RSI under that. The x labels sit under the lowest pane.
    #[expect(
        clippy::too_many_lines,
        reason = "the price, volume and RSI panes line up on one set of rows"
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
        let rows: Vec<PriceRow> = visible(&all).iter().copied().map(PriceRow::of).collect();
        let closes: Vec<f64> = rows.iter().map(|row| num(row.bar.close)).collect();
        // The extent every overlaid line is pinned to: the primary
        // bars' own range (the candlestick chart's scale), widened
        // by any other series' closes in Line kind, where the lines
        // are the chart and nothing may clip.
        let (mut lo, mut hi) = rows.iter().fold((f64::MAX, f64::MIN), |(lo, hi), row| {
            (lo.min(num(row.bar.low)), hi.max(num(row.bar.high)))
        });
        let mut others: Vec<(SeriesChoice, Vec<f64>)> = Vec::new();
        for choice in self.series.iter().skip(1) {
            let bars = model.bars(choice.listing, self.bar_ns());
            if let Some(aligned) = aligned_closes(&rows, visible(&bars)) {
                if self.kind == PriceKind::Line {
                    for value in &aligned {
                        lo = lo.min(*value);
                        hi = hi.max(*value);
                    }
                }
                others.push((*choice, aligned));
            }
        }
        if hi <= lo {
            hi = lo + 1.;
        }
        let margin = (rows.len() / 8).max(1);
        let theme = cx.theme();
        let uid = format!(
            "chart-{}-{}",
            primary.listing.venue_id, primary.listing.instrument_id
        );
        // Price pane.
        let mut price = div()
            .id("chart-price")
            .test_support()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full();
        match self.kind {
            PriceKind::Candles => {
                let tip_name = SharedString::from(symbol.clone());
                price = price.child(
                    div().absolute().inset_0().child(
                        CandlestickChart::new(rows.clone())
                            .id(SharedString::from(format!("{uid}-candles")))
                            .x(|row| row.label.clone())
                            .open(|row| num(row.bar.open))
                            .high(|row| num(row.bar.high))
                            .low(|row| num(row.bar.low))
                            .close(|row| num(row.bar.close))
                            .tick_margin(margin)
                            .x_axis(false)
                            .appear(false)
                            .tooltip_content(move |row: &PriceRow, _, cx| {
                                candle_tip(&tip_name, row, cx)
                            }),
                    ),
                );
            }
            PriceKind::Line => {
                price = price.child(
                    div().absolute().inset_0().child(
                        LineChart::new(line_rows(&rows, &closes))
                            .id(SharedString::from(format!("{uid}-line")))
                            .x(|row| row.label.clone())
                            .y(|row| row.value)
                            .stroke(series_color(primary.color, cx))
                            .linear()
                            .name(SharedString::from(symbol.clone()))
                            .y_domain(lo, hi)
                            .y_axis(true)
                            .y_axis_label_placement(AxisLabelPlacement::Inside)
                            .y_tick_count(4)
                            .y_tick_format(|value| format!("{value:.0}"))
                            .x_axis(false)
                            .x_tick_count(6)
                            .tooltip_title(|row: &LineRow| {
                                SharedString::from(format!("{} run time", row.label))
                            })
                            .tooltip_value(|_, value| SharedString::from(tick_text(value)))
                            .appear(false),
                    ),
                );
            }
        }
        // Other venues and securities over the primary, each its own
        // labelled line in its own colour — never merged.
        for (choice, aligned) in &others {
            let name = model.symbol(choice.listing).map_or_else(
                || format!("V{}", choice.listing.venue_id),
                |symbol| format!("{symbol} V{}", choice.listing.venue_id),
            );
            price = price.child(Self::overlay(
                SharedString::from(format!(
                    "chart-{}-{}-line",
                    choice.listing.venue_id, choice.listing.instrument_id
                )),
                line_rows(&rows, aligned),
                series_color(choice.color, cx),
                name,
                (lo, hi),
            ));
        }
        // MA and EMA over the primary series' closes.
        if self.indicators.ma {
            price = price.child(Self::overlay(
                SharedString::from(format!("{uid}-ma")),
                line_rows(&rows, &sma(&closes, MA_PERIOD)),
                theme.foreground,
                "MA(20)",
                (lo, hi),
            ));
        }
        if self.indicators.ema {
            price = price.child(Self::overlay(
                SharedString::from(format!("{uid}-ema")),
                line_rows(&rows, &ema(&closes, MA_PERIOD)),
                theme.muted_foreground,
                "EMA(20)",
                (lo, hi),
            ));
        }
        // Volume pane: the primary series' bars, coloured by the
        // bar's direction (the RIT/Bloomberg convention), bands
        // lining up with the candles above.
        let (bullish, bearish) = (theme.chart_bullish, theme.chart_bearish);
        let volume = div()
            .id("chart-volume")
            .test_support()
            .w_full()
            .h(px(VOLUME_HEIGHT))
            .flex_shrink_0()
            .child(
                BarChart::new(rows.clone())
                    .id(SharedString::from(format!("{uid}-volume")))
                    .band(|row| row.label.clone())
                    .value(|row| num_wide(row.bar.volume))
                    .fill(move |row: &PriceRow, _, _, _| {
                        if row.bar.close >= row.bar.open {
                            bullish.opacity(0.55)
                        } else {
                            bearish.opacity(0.55)
                        }
                    })
                    .tick_margin(margin)
                    .label_axis(!self.indicators.rsi)
                    .value_axis(true)
                    .value_axis_label_placement(AxisLabelPlacement::Inside)
                    .value_tick_count(2)
                    .value_tick_format(|value| format!("{value:.0}"))
                    .grid(false)
                    .name("Volume")
                    .tooltip_title(|row: &PriceRow| {
                        SharedString::from(format!("{} run time", row.label))
                    })
                    .tooltip_value(|row: &PriceRow, _| {
                        SharedString::from(format!("{} lots", row.bar.volume))
                    })
                    .appear(false),
            );
        let mut stack = v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(price)
            .child(volume);
        if self.indicators.rsi {
            stack = stack.child(ui::section_label("RSI(14)", cx)).child(
                div()
                    .id("chart-rsi")
                    .test_support()
                    .w_full()
                    .h(px(RSI_HEIGHT))
                    .flex_shrink_0()
                    .child(
                        LineChart::new(line_rows(&rows, &rsi(&closes, RSI_PERIOD)))
                            .id(SharedString::from(format!("{uid}-rsi")))
                            .x(|row| row.label.clone())
                            .y(|row| row.value)
                            .stroke(series_color(primary.color, cx))
                            .linear()
                            .name("RSI(14)")
                            .y_domain(0., 100.)
                            .y_axis(true)
                            .y_axis_label_placement(AxisLabelPlacement::Inside)
                            .y_tick_count(3)
                            .y_tick_format(|value| format!("{value:.0}"))
                            .reference_line(30.)
                            .reference_line(70.)
                            .x_tick_count(6)
                            .tooltip_title(|row: &LineRow| {
                                SharedString::from(format!("{} run time", row.label))
                            })
                            .tooltip_value(|_, value| SharedString::from(format!("{value:.1}")))
                            .appear(false),
                    ),
            );
        }
        stack.into_any_element()
    }

    /// The P&L mode: current realized, unrealized and total over
    /// an area chart of total P&L against run time, zero lined.
    fn pnl_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let totals = model.pnl_totals();
        let theme = cx.theme();
        let signed = |value: i128| match value.cmp(&0) {
            std::cmp::Ordering::Less => theme.chart_bearish,
            std::cmp::Ordering::Greater => theme.chart_bullish,
            std::cmp::Ordering::Equal => theme.foreground,
        };
        let stat = |id: &'static str, label: &'static str, value: i128| {
            div().id(id).test_support().child(ui::stat_colored(
                label,
                format!("{value:+}"),
                signed(value),
                cx,
            ))
        };
        let mut body = v_flex()
            .flex_1()
            .min_h_0()
            .w_full()
            .gap(px(ui::PANE_GAP))
            .child(
                h_flex()
                    .w_full()
                    .gap_4()
                    .child(stat("chart-pnl-realized", "Realized", totals.realized))
                    .child(stat(
                        "chart-pnl-unrealized",
                        "Unrealized",
                        totals.unrealized,
                    ))
                    .child(stat("chart-pnl-total", "Total", totals.total)),
            );
        let samples = model.pnl_series();
        if samples.is_empty() {
            return body
                .child(ui::empty_state(
                    "chart-pnl-empty",
                    IconName::Frame,
                    "No fills yet — P&L appears after your first trade",
                    None,
                    cx,
                ))
                .into_any_element();
        }
        let points: Vec<LineRow> = samples
            .iter()
            .skip(samples.len().saturating_sub(MAX_VISIBLE_BARS))
            .map(|sample| LineRow {
                label: SharedString::from(run_clock(sample.time_ns)),
                value: num_wide(sample.total),
            })
            .collect();
        let color = signed(totals.total);
        body = body.child(
            div()
                .id("chart-pnl-area")
                .test_support()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    AreaChart::new(points)
                        .id("chart-pnl-chart")
                        .x(|row| row.label.clone())
                        .y(|row| row.value)
                        .stroke(color)
                        .fill(color.opacity(0.25))
                        .linear()
                        .name("Total P&L")
                        .reference_line(0.)
                        .y_axis(true)
                        .y_tick_count(4)
                        .y_tick_format(|value| format!("{value:+.0}"))
                        .x_tick_count(6)
                        .tooltip_title(|row: &LineRow| {
                            SharedString::from(format!("{} run time", row.label))
                        })
                        .tooltip_value(|_, _, value| SharedString::from(format!("{value:+.0}")))
                        .appear(false),
                ),
        );
        body.into_any_element()
    }
}

impl Render for ChartPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.seed(cx);
        let mut body = ui::pane_body("chart", &self.focus).child(self.mode_row(cx));
        body = match self.mode {
            ChartMode::Security => {
                let history = ui::muted(
                    "History starts when the desk connected · each venue is its own series",
                    cx,
                );
                body.child(self.pick_row(cx))
                    .child(self.chips(cx))
                    .child(self.security_body(cx))
                    .child(history)
            }
            ChartMode::Pnl => {
                let known = self.desk.read(cx).model().pnl_totals().multipliers_known;
                let mut label = "Derived from your fills and the best bid/ask; \
                                 the venue's own valuation is not yet on BNP"
                    .to_owned();
                if !known {
                    label.push_str(" · assumes contract size 1");
                }
                body.child(self.pnl_body(cx)).child(ui::muted(label, cx))
            }
        };
        body
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

/// A tick value an axis or tooltip reads: whole ticks plain, an
/// average's fraction to one decimal.
fn tick_text(value: f64) -> String {
    if value.fract() == 0. {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// The candlestick hover card: run time, OHLC and volume, text in
/// text tokens. The highlight band over the hovered candle is the
/// chart component's own.
fn candle_tip(symbol: &str, row: &PriceRow, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let foreground = cx.theme().foreground;
    let line = |label: &'static str, value: String| {
        h_flex()
            .gap_2()
            .child(
                div()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(muted)
                    .child(label),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(foreground)
                    .child(value),
            )
    };
    v_flex()
        .gap_1()
        .min_w(px(132.))
        .font_features(crate::theme::tabular())
        .child(
            div()
                .text_size(ui::rem(ui::LABEL_SIZE))
                .text_color(foreground)
                .child(format!("{symbol} · {} run time", row.label)),
        )
        .child(line("Open", format::price(Some(row.bar.open))))
        .child(line("High", format::price(Some(row.bar.high))))
        .child(line("Low", format::price(Some(row.bar.low))))
        .child(line("Close", format::price(Some(row.bar.close))))
        .child(line("Volume", format!("{}", row.bar.volume)))
        .into_any_element()
}

/// A compact total-P&L sparkline for another window to place (the
/// Portfolio window is another change's; the coordinator places
/// this there). No axes: the window it sits in carries the figures.
#[must_use]
pub fn pnl_sparkline(samples: &VecDeque<PnlSample>, cx: &App) -> AnyElement {
    if samples.len() < 2 {
        return div()
            .id("pnl-sparkline-empty")
            .test_support()
            .h(px(SPARKLINE_HEIGHT))
            .child(ui::muted("No P&L yet", cx))
            .into_any_element();
    }
    let total = samples.back().map_or(0, |sample| sample.total);
    let color = if total < 0 {
        cx.theme().chart_bearish
    } else {
        cx.theme().chart_bullish
    };
    let points: Vec<LineRow> = samples
        .iter()
        .skip(samples.len().saturating_sub(MAX_VISIBLE_BARS))
        .map(|sample| LineRow {
            label: SharedString::from(run_clock(sample.time_ns)),
            value: num_wide(sample.total),
        })
        .collect();
    div()
        .id("pnl-sparkline")
        .test_support()
        .w_full()
        .h(px(SPARKLINE_HEIGHT))
        .child(
            AreaChart::new(points)
                .id("pnl-sparkline-chart")
                .x(|row| row.label.clone())
                .y(|row| row.value)
                .stroke(color)
                .fill(color.opacity(0.25))
                .linear()
                .reference_line(0.)
                .x_axis(false)
                .y_axis(false)
                .grid(false)
                .interactive(false)
                .appear(false),
        )
        .into_any_element()
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
        let primary: Vec<PriceRow> = [0, 1, 2, 3]
            .into_iter()
            .map(|second| PriceRow::of(bar(second * 1_000_000_000, 100)))
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

    #[test]
    fn run_clock_labels_every_second_uniquely() {
        assert_eq!(run_clock(0), "0:00");
        assert_eq!(run_clock(65_000_000_000), "1:05");
        assert_eq!(run_clock(3_661_000_000_000), "61:01");
    }
}
