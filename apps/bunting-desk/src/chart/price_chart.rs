//! `PriceChart`: the Charting window's price plot — candles or a
//! line, volume and RSI sub-panes on one x scale and one crosshair,
//! a right price axis with nice ticks, a dashed last-price line
//! with its axis tag, and an OHLCV legend line.
//!
//! Longbridge's finished chart components cannot draw this
//! (0.7.1's `CandlestickChart` has no price axis), so the plot is
//! composed here from the same primitives those components use:
//! `Plot`/`PlotElement`, `ScaleBand`/`ScaleLinear`, `Grid`, the
//! `Line` shape, and `PlotLabel` text. The structure (scales in
//! paint, hover resolved in `tooltip_state`, marks as quads and
//! shapes) follows gpui-component 0.7.1's `candlestick_chart.rs`
//! and `line_chart.rs` (Apache License 2.0, Longbridge). Zoom and
//! pan live in the panel: it owns the [`super::range::ViewRange`]
//! and hands this plot the visible bars each frame.
#![allow(
    clippy::float_arithmetic,
    clippy::cast_precision_loss,
    reason = "chart coordinates are f32/f64 by the plot primitives' own API"
)]

use super::range::{format_tick, nice_ticks, pick_time_step, run_clock_axis};
use bunting_trading_state::Bar as MarketBar;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::plot::label::{Text, measure_text_width};
use gpui_kit::component::plot::scale::{Scale as _, ScaleBand, ScaleLinear};
use gpui_kit::component::plot::shape::Line;
use gpui_kit::component::plot::{
    Curve, Grid, Plot, PlotElement, PlotHover, PlotLabel, TooltipState,
};
use gpui_kit::{
    AnyElement, App, Bounds, ElementId, Hsla, IntoElement, Pixels, Point, SharedString, TextAlign,
    Window, fill, point, px, size,
};
use std::cell::Cell;
use std::rc::Rc;

/// Right-axis gutter used until the first frame measures the
/// widest price label.
const DEFAULT_GUTTER: f32 = 56.;
/// Height reserved under the panes for the run-time axis.
const TIME_AXIS_H: f32 = 18.;
/// Design sizes at the base font (see `crate::ui::rem`).
const AXIS_FONT: f32 = 11.;
const LEGEND_FONT: f32 = 12.;
/// Widest candle body, as in the component charts.
const MAX_BODY: f32 = 30.;

/// One more series drawn over the price pane, already aligned to
/// the primary bars (closes, or percent changes in comparison).
#[derive(Clone)]
pub struct OtherSeries {
    pub name: SharedString,
    pub values: Vec<f64>,
    pub color: Hsla,
}

/// One horizontal band of the plot, in coordinates relative to the
/// plot's top-left.
#[derive(Clone, Copy)]
struct Pane {
    top: f32,
    height: f32,
}

impl Pane {
    fn bounds(self, whole: &Bounds<Pixels>, plot_w: f32) -> Bounds<Pixels> {
        Bounds::new(
            point(whole.origin.x, whole.origin.y + px(self.top)),
            size(px(plot_w), px(self.height)),
        )
    }

    fn contains(self, y: f32) -> bool {
        (self.top..self.top + self.height).contains(&y)
    }
}

struct Layout {
    plot_w: f32,
    price: Pane,
    volume: Pane,
    rsi: Option<Pane>,
    /// Top of the time axis strip.
    axis_top: f32,
}

pub struct PriceChart {
    id: ElementId,
    bars: Vec<MarketBar>,
    candles: bool,
    comparison: bool,
    name: SharedString,
    primary_color: Hsla,
    others: Vec<OtherSeries>,
    ma: Option<Vec<f64>>,
    ema: Option<Vec<f64>>,
    rsi: Option<Vec<f64>>,
    margin_slots: usize,
    /// The measured right-axis gutter, shared back to the panel
    /// for its zoom/pan arithmetic.
    gutter_out: Rc<Cell<f32>>,
    gutter: f32,
    hovered: Option<TooltipState>,
}

impl PriceChart {
    pub fn new(id: impl Into<ElementId>, bars: Vec<MarketBar>, gutter_out: Rc<Cell<f32>>) -> Self {
        Self {
            id: id.into(),
            bars,
            candles: true,
            comparison: false,
            name: SharedString::default(),
            primary_color: Hsla::default(),
            others: Vec::new(),
            ma: None,
            ema: None,
            rsi: None,
            margin_slots: 0,
            gutter_out,
            gutter: DEFAULT_GUTTER,
            hovered: None,
        }
    }

    #[must_use]
    pub fn candles(mut self, candles: bool) -> Self {
        self.candles = candles;
        self
    }

    #[must_use]
    pub fn comparison(mut self, comparison: bool) -> Self {
        self.comparison = comparison;
        self
    }

    #[must_use]
    pub fn name(mut self, name: impl Into<SharedString>) -> Self {
        self.name = name.into();
        self
    }

    #[must_use]
    pub fn primary_color(mut self, color: Hsla) -> Self {
        self.primary_color = color;
        self
    }

    #[must_use]
    pub fn others(mut self, others: Vec<OtherSeries>) -> Self {
        self.others = others;
        self
    }

    #[must_use]
    pub fn ma(mut self, ma: Option<Vec<f64>>) -> Self {
        self.ma = ma;
        self
    }

    #[must_use]
    pub fn ema(mut self, ema: Option<Vec<f64>>) -> Self {
        self.ema = ema;
        self
    }

    #[must_use]
    pub fn rsi(mut self, rsi: Option<Vec<f64>>) -> Self {
        self.rsi = rsi;
        self
    }

    /// Empty band slots after the last bar, so it is not glued to
    /// the price axis while following.
    #[must_use]
    pub fn margin_slots(mut self, slots: usize) -> Self {
        self.margin_slots = slots;
        self
    }

    fn layout(&self, size: (f32, f32)) -> Layout {
        let (width, height) = size;
        let plot_w = (width - self.gutter).max(40.);
        let panes_h = (height - TIME_AXIS_H).max(30.);
        let (price_share, volume_share) = if self.rsi.is_some() {
            (0.64, 0.18)
        } else {
            (0.82, 0.18)
        };
        let price = Pane {
            top: 0.,
            height: (panes_h * price_share).max(20.),
        };
        let volume = Pane {
            top: price.height,
            height: (panes_h * volume_share).max(10.),
        };
        let rsi = self.rsi.as_ref().map(|_| Pane {
            top: volume.top + volume.height,
            height: (panes_h - volume.top - volume.height).max(10.),
        });
        Layout {
            plot_w,
            price,
            volume,
            rsi,
            axis_top: panes_h,
        }
    }

    fn x_scale(&self, plot_w: f32) -> ScaleBand<usize> {
        ScaleBand::new(0..self.bars.len(), [0., plot_w])
            .band_count(self.bars.len() + self.margin_slots)
    }

    /// The price (or percent) domain over the visible bars,
    /// padded so the extremes do not touch the pane edges.
    fn extent(&self) -> (f64, f64) {
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        let mut take = |value: f64| {
            lo = lo.min(value);
            hi = hi.max(value);
        };
        if self.comparison {
            for series in &self.others {
                for value in &series.values {
                    take(*value);
                }
            }
            // The primary's own percent line is the last "other" the
            // panel passes in comparison mode; zero always shows.
            take(0.);
        } else {
            for bar in &self.bars {
                take(bar.low as f64);
                take(bar.high as f64);
            }
            for values in [&self.ma, &self.ema].into_iter().flatten() {
                for value in values.iter().copied() {
                    take(value);
                }
            }
        }
        if lo > hi {
            return (0., 1.);
        }
        if (hi - lo).abs() < f64::EPSILON {
            let pad = (hi.abs() * 0.01).max(0.5);
            return (lo - pad, hi + pad);
        }
        let pad = (hi - lo) * 0.06;
        (lo - pad, hi + pad)
    }

    fn axis_font(window: &Window) -> f32 {
        window.rem_size().as_f32() * (AXIS_FONT / 14.)
    }

    fn legend_font(window: &Window) -> f32 {
        window.rem_size().as_f32() * (LEGEND_FONT / 14.)
    }
}

impl Plot for PriceChart {
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    /// Measures the widest price label so the right gutter fits
    /// the axis, and shares it with the panel.
    fn prepaint(
        &mut self,
        _bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut App,
    ) -> Vec<AnyElement> {
        let (lo, hi) = self.extent();
        let (ticks, decimals) = nice_ticks(lo, hi, 6);
        let font = px(Self::axis_font(window));
        let widest = ticks
            .iter()
            .map(|tick| {
                let label = SharedString::from(if self.comparison {
                    format!("{tick:+.1}%")
                } else {
                    format_tick(*tick, decimals)
                });
                measure_text_width(&label, font, window)
            })
            .fold(0_f32, f32::max);
        self.gutter = (widest + 14.).clamp(44., 120.);
        self.gutter_out.set(self.gutter);
        Vec::new()
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _cx: &App,
    ) -> Option<TooltipState> {
        if self.bars.is_empty() {
            return None;
        }
        let layout = self.layout((bounds.size.width.as_f32(), bounds.size.height.as_f32()));
        let (x, y) = (position.x.as_f32(), position.y.as_f32());
        if x < 0. || x > layout.plot_w || y < 0. || y > layout.axis_top {
            return None;
        }
        let scale = self.x_scale(layout.plot_w);
        let step = scale.step().max(0.01);
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "x is inside the plot, checked above"
        )]
        let index = ((x / step) as usize).min(self.bars.len() - 1);
        let center = scale.tick(&index).unwrap_or(0.) + scale.band_width() / 2.;
        Some(TooltipState::new(
            index,
            point(px(center), position.y),
            Vec::new(),
        ))
    }

    fn hover(&mut self, hover: Option<&PlotHover>, _window: &mut Window, _cx: &mut App) {
        self.hovered = hover
            .filter(|hover| hover.is_hovered())
            .map(|hover| hover.state().clone());
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one paint pass over price, volume, RSI, axes, legend and crosshair, in z order"
    )]
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let theme = cx.theme();
        let origin = bounds.origin;
        let layout = self.layout((bounds.size.width.as_f32(), bounds.size.height.as_f32()));
        let plot_w = layout.plot_w;
        let x = self.x_scale(plot_w);
        let step = x.step();
        let center = |index: usize| x.tick(&index).unwrap_or(0.) + x.band_width() / 2.;
        let (lo, hi) = self.extent();
        let price_pane = layout.price;
        let y = ScaleLinear::new([lo, hi], [price_pane.height, 0.]);
        let price_bounds = price_pane.bounds(&bounds, plot_w);
        let axis_font = Self::axis_font(window);
        let mut labels: Vec<Text> = Vec::new();
        let quad = |window: &mut Window, qx: f32, qy: f32, qw: f32, qh: f32, color: Hsla| {
            if qw > 0. && qh > 0. {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + px(qx), origin.y + px(qy)),
                        size(px(qw), px(qh)),
                    ),
                    color,
                ));
            }
        };

        // Tag boxes on the axes, so labels under them can stand down
        // instead of colliding (coordinator screenshot review).
        let tag_h = axis_font + 4.;
        let last_tag_y = (!self.comparison)
            .then(|| {
                self.bars
                    .last()
                    .map(|bar| y.tick(&(bar.close as f64)).unwrap_or(0.))
            })
            .flatten();
        let cross_price_y = self
            .hovered
            .as_ref()
            .map(|state| state.cross_line.y.as_f32())
            .filter(|cursor_y| price_pane.contains(*cursor_y));
        let under_tag =
            |label_y: f32, tag_y: f32| (label_y - tag_y).abs() < f32::midpoint(axis_font, tag_h);

        // ---- price pane: grid, marks -----------------------------------
        let (ticks, decimals) = nice_ticks(lo, hi, 6);
        Grid::new()
            .y(ticks.iter().map(|tick| y.tick(tick).unwrap_or(0.)))
            .stroke(theme.chart_grid)
            .paint(&price_bounds, window);
        for tick in &ticks {
            let tick_y = y.tick(tick).unwrap_or(0.);
            if last_tag_y.is_some_and(|tag_y| under_tag(tick_y, tag_y))
                || cross_price_y.is_some_and(|tag_y| under_tag(tick_y + price_pane.top, tag_y))
            {
                continue;
            }
            labels.push(
                Text::new(
                    if self.comparison {
                        format!("{tick:+.1}%")
                    } else {
                        format_tick(*tick, decimals)
                    },
                    point(
                        px(plot_w + 6.),
                        px(price_pane.top + tick_y - axis_font / 2.),
                    ),
                    theme.muted_foreground,
                )
                .font_size(px(axis_font)),
            );
        }

        let line = |values: &[f64],
                    color: Hsla,
                    pane: Pane,
                    scale_y: &ScaleLinear<f64>,
                    window: &mut Window| {
            let points: Vec<(f32, f32)> = values
                .iter()
                .enumerate()
                .map(|(index, value)| (center(index), scale_y.tick(value).unwrap_or(0.)))
                .collect();
            Line::new()
                .data(points)
                .x(|point| Some(point.0))
                .y(|point| Some(point.1))
                .stroke(color)
                .stroke_width(px(1.5))
                .curve(Curve::Linear)
                .paint(&pane.bounds(&bounds, plot_w), window);
        };

        if self.comparison {
            // Zero line under the percent lines.
            let zero_y = y.tick(&0.).unwrap_or(0.);
            quad(
                window,
                0.,
                price_pane.top + zero_y - 0.5,
                plot_w,
                1.,
                theme.muted_foreground.opacity(0.6),
            );
            for series in &self.others {
                line(&series.values, series.color, price_pane, &y, window);
            }
        } else if self.candles {
            let body_w = (step - 1.5).clamp(1., MAX_BODY);
            for (index, bar) in self.bars.iter().enumerate() {
                let color = if bar.close >= bar.open {
                    theme.chart_bullish
                } else {
                    theme.chart_bearish
                };
                let cx_mid = center(index);
                let (y_open, y_close, y_high, y_low) = (
                    y.tick(&(bar.open as f64)).unwrap_or(0.),
                    y.tick(&(bar.close as f64)).unwrap_or(0.),
                    y.tick(&(bar.high as f64)).unwrap_or(0.),
                    y.tick(&(bar.low as f64)).unwrap_or(0.),
                );
                // Wick first, body over it. Under 3 px per bar the
                // candle becomes an OHLC tick: one 1 px stem.
                quad(
                    window,
                    cx_mid - 0.5,
                    price_pane.top + y_high,
                    1.,
                    (y_low - y_high).max(1.),
                    color,
                );
                if step >= 3. {
                    let (top, bottom) = (y_open.min(y_close), y_open.max(y_close));
                    quad(
                        window,
                        cx_mid - body_w / 2.,
                        price_pane.top + top,
                        body_w,
                        (bottom - top).max(1.),
                        color,
                    );
                } else if step >= 2. {
                    // Open tick left, close tick right.
                    quad(
                        window,
                        cx_mid - 1.,
                        price_pane.top + y_open - 0.5,
                        1.,
                        1.,
                        color,
                    );
                    quad(
                        window,
                        cx_mid,
                        price_pane.top + y_close - 0.5,
                        1.,
                        1.,
                        color,
                    );
                }
            }
        } else {
            let closes: Vec<f64> = self.bars.iter().map(|bar| bar.close as f64).collect();
            line(&closes, self.primary_color, price_pane, &y, window);
        }
        if !self.comparison {
            if let Some(ma) = &self.ma {
                line(ma, theme.foreground, price_pane, &y, window);
            }
            if let Some(ema) = &self.ema {
                line(ema, theme.muted_foreground, price_pane, &y, window);
            }
            // Last price: dashed line and its filled gutter tag.
            if let Some(last) = self.bars.last() {
                let prev = self.bars.get(self.bars.len().saturating_sub(2));
                let color = match prev {
                    Some(prev) if last.close < prev.close => theme.chart_bearish,
                    _ => theme.chart_bullish,
                };
                let last_y = last_tag_y.unwrap_or(0.);
                Grid::new()
                    .y([last_y])
                    .stroke(color)
                    .dash_array(&[px(4.), px(4.)])
                    .paint(&price_bounds, window);
                quad(
                    window,
                    plot_w + 1.,
                    price_pane.top + last_y - tag_h / 2.,
                    self.gutter - 2.,
                    tag_h,
                    color,
                );
                labels.push(
                    Text::new(
                        format_tick(last.close as f64, decimals),
                        point(
                            px(plot_w + self.gutter / 2.),
                            px(price_pane.top + last_y - axis_font / 2.),
                        ),
                        theme.foreground,
                    )
                    .font_size(px(axis_font))
                    .align(TextAlign::Center),
                );
            }
        }

        // ---- volume pane ------------------------------------------------
        let volume_pane = layout.volume;
        quad(
            window,
            0.,
            volume_pane.top - 0.5,
            plot_w + self.gutter,
            1.,
            theme.border,
        );
        let max_volume = self
            .bars
            .iter()
            .map(|bar| bar.volume)
            .fold(0_i128, i128::max);
        if max_volume > 0 {
            let volume_y = ScaleLinear::new([0_f64, max_volume as f64], [volume_pane.height, 0.]);
            let body_w = (step - 1.5).clamp(1., MAX_BODY);
            for (index, bar) in self.bars.iter().enumerate() {
                let base = if bar.close >= bar.open {
                    theme.chart_bullish
                } else {
                    theme.chart_bearish
                };
                let top = volume_y
                    .tick(&(bar.volume as f64))
                    .unwrap_or(volume_pane.height);
                quad(
                    window,
                    center(index) - body_w / 2.,
                    volume_pane.top + top,
                    body_w,
                    volume_pane.height - top,
                    base.opacity(0.45),
                );
            }
            let cross_in_volume = self
                .hovered
                .as_ref()
                .is_some_and(|state| volume_pane.contains(state.cross_line.y.as_f32()));
            if !cross_in_volume {
                labels.push(
                    Text::new(
                        format!("{max_volume}"),
                        point(px(plot_w + 6.), px(volume_pane.top + 1.)),
                        theme.muted_foreground,
                    )
                    .font_size(px(axis_font)),
                );
            }
        }

        // ---- RSI pane ---------------------------------------------------
        if let (Some(rsi), Some(rsi_pane)) = (&self.rsi, layout.rsi) {
            quad(
                window,
                0.,
                rsi_pane.top - 0.5,
                plot_w + self.gutter,
                1.,
                theme.border,
            );
            let rsi_y = ScaleLinear::new([0_f64, 100.], [rsi_pane.height, 0.]);
            for level in [30., 70.] {
                Grid::new()
                    .y([rsi_y.tick(&level).unwrap_or(0.)])
                    .stroke(theme.muted_foreground.opacity(0.5))
                    .dash_array(&[px(4.), px(4.)])
                    .paint(&rsi_pane.bounds(&bounds, plot_w), window);
                let level_y = rsi_y.tick(&level).unwrap_or(0.);
                let cross_in_rsi = self.hovered.as_ref().is_some_and(|state| {
                    let cursor_y = state.cross_line.y.as_f32();
                    rsi_pane.contains(cursor_y) && under_tag(level_y + rsi_pane.top, cursor_y)
                });
                if !cross_in_rsi {
                    labels.push(
                        Text::new(
                            format!("{level:.0}"),
                            point(
                                px(plot_w + 6.),
                                px(rsi_pane.top + rsi_y.tick(&level).unwrap_or(0.)
                                    - axis_font / 2.),
                            ),
                            theme.muted_foreground,
                        )
                        .font_size(px(axis_font)),
                    );
                }
            }
            line(rsi, self.primary_color, rsi_pane, &rsi_y, window);
        }

        // ---- time axis ----------------------------------------------------
        if let (Some(first), Some(last)) = (self.bars.first(), self.bars.last()) {
            let bar_ns = if self.bars.len() > 1 {
                (last.start_ns - first.start_ns) / (self.bars.len() - 1) as u64
            } else {
                bunting_trading_state::FINEST_BAR_NS
            };
            let sample = SharedString::from(run_clock_axis(last.start_ns));
            let label_w = measure_text_width(&sample, px(axis_font), window);
            let step_ns = pick_time_step(bar_ns.max(1), step, label_w);
            for (index, bar) in self.bars.iter().enumerate() {
                if bar.start_ns % step_ns != 0 {
                    continue;
                }
                let text = run_clock_axis(bar.start_ns);
                let half =
                    measure_text_width(&SharedString::from(text.clone()), px(axis_font), window)
                        / 2.;
                let x_at = center(index).clamp(half, (plot_w - half).max(half));
                labels.push(
                    Text::new(
                        text,
                        point(px(x_at), px(layout.axis_top + 3.)),
                        theme.muted_foreground,
                    )
                    .font_size(px(axis_font))
                    .align(TextAlign::Center),
                );
            }
        }

        // ---- legend -------------------------------------------------------
        let legend_font = Self::legend_font(window);
        let hovered = self
            .hovered
            .as_ref()
            .map(|state| state.index)
            .filter(|index| *index < self.bars.len())
            .or_else(|| self.bars.len().checked_sub(1));
        if self.comparison {
            for (row, series) in self.others.iter().enumerate() {
                let value = hovered
                    .and_then(|index| series.values.get(index))
                    .or_else(|| series.values.last());
                if let Some(value) = value {
                    labels.push(
                        Text::new(
                            format!("{} {value:+.2}%", series.name),
                            point(px(6.), px(4. + row as f32 * (legend_font + 3.))),
                            series.color,
                        )
                        .font_size(px(legend_font)),
                    );
                }
            }
        } else if let Some(index) = hovered {
            let bar = self.bars[index];
            let direction = if bar.close >= bar.open {
                theme.chart_bullish
            } else {
                theme.chart_bearish
            };
            let change = if index > 0 && self.bars[index - 1].close != 0 {
                (bar.close as f64 / self.bars[index - 1].close as f64 - 1.) * 100.
            } else {
                0.
            };
            let segments: [(String, Hsla); 4] = [
                (self.name.to_string(), theme.foreground),
                (
                    format!(
                        "O {}  H {}  L {}  C {}",
                        bar.open, bar.high, bar.low, bar.close
                    ),
                    direction,
                ),
                (format!("V {}", bar.volume), theme.muted_foreground),
                (format!("{change:+.2}%"), direction),
            ];
            let mut x_at = 6.;
            for (text, color) in segments {
                if text.is_empty() {
                    continue;
                }
                let shared = SharedString::from(text);
                let width = measure_text_width(&shared, px(legend_font), window);
                labels.push(
                    Text::new(shared, point(px(x_at), px(4.)), color).font_size(px(legend_font)),
                );
                x_at += width + 10.;
            }
            if let (Some(rsi), Some(rsi_pane)) = (&self.rsi, layout.rsi)
                && let Some(value) = rsi.get(index)
            {
                labels.push(
                    Text::new(
                        format!("RSI(14) {value:.1}"),
                        point(px(6.), px(rsi_pane.top + 3.)),
                        theme.foreground,
                    )
                    .font_size(px(legend_font)),
                );
            }
        }

        // ---- crosshair ------------------------------------------------------
        if let Some(state) = &self.hovered
            && state.index < self.bars.len()
        {
            let cx_mid = center(state.index);
            let cursor_y = state.cross_line.y.as_f32();
            let dashed = theme.muted_foreground;
            for pane in [Some(price_pane), Some(volume_pane), layout.rsi]
                .into_iter()
                .flatten()
            {
                Grid::new()
                    .x([cx_mid])
                    .stroke(dashed)
                    .dash_array(&[px(4.), px(4.)])
                    .paint(&pane.bounds(&bounds, plot_w), window);
            }
            // Horizontal line and its axis tag in the pane under the cursor.
            let invert = |pane: Pane, dlo: f64, dhi: f64| {
                dhi - f64::from(cursor_y - pane.top) / f64::from(pane.height) * (dhi - dlo)
            };
            let tag = |labels: &mut Vec<Text>, pane: Pane, text: String, window: &mut Window| {
                window.paint_quad(fill(
                    Bounds::new(
                        point(
                            origin.x + px(plot_w + 1.),
                            origin.y + px(cursor_y - tag_h / 2.),
                        ),
                        size(px(self.gutter - 2.), px(tag_h)),
                    ),
                    theme.popover,
                ));
                labels.push(
                    Text::new(
                        text,
                        point(px(plot_w + self.gutter / 2.), px(cursor_y - axis_font / 2.)),
                        theme.foreground,
                    )
                    .font_size(px(axis_font))
                    .align(TextAlign::Center),
                );
                let _ = pane;
            };
            if price_pane.contains(cursor_y) {
                Grid::new()
                    .y([cursor_y - price_pane.top])
                    .stroke(dashed)
                    .dash_array(&[px(4.), px(4.)])
                    .paint(&price_bounds, window);
                let value = invert(price_pane, lo, hi);
                tag(
                    &mut labels,
                    price_pane,
                    if self.comparison {
                        format!("{value:+.1}%")
                    } else {
                        format_tick(value, decimals)
                    },
                    window,
                );
            } else if volume_pane.contains(cursor_y) && max_volume > 0 {
                let value = invert(volume_pane, 0., max_volume as f64).max(0.);
                tag(&mut labels, volume_pane, format!("{value:.0}"), window);
            } else if let Some(rsi_pane) = layout.rsi
                && rsi_pane.contains(cursor_y)
            {
                let value = invert(rsi_pane, 0., 100.).clamp(0., 100.);
                tag(&mut labels, rsi_pane, format!("{value:.0}"), window);
            }
            // Time tag on the bottom axis.
            let bar = self.bars[state.index];
            let text = SharedString::from(run_clock_axis(bar.start_ns));
            let width = measure_text_width(&text, px(axis_font), window) + 8.;
            let left = (cx_mid - width / 2.).clamp(0., (plot_w - width).max(0.));
            quad(
                window,
                left,
                layout.axis_top + 1.,
                width,
                axis_font + 4.,
                theme.popover,
            );
            labels.push(
                Text::new(
                    text,
                    point(px(left + width / 2.), px(layout.axis_top + 3.)),
                    theme.foreground,
                )
                .font_size(px(axis_font))
                .align(TextAlign::Center),
            );
            // Hover dots on the lines.
            if self.comparison {
                for series in &self.others {
                    if let Some(value) = series.values.get(state.index) {
                        let dy = y.tick(value).unwrap_or(0.);
                        quad(
                            window,
                            cx_mid - 2.5,
                            price_pane.top + dy - 2.5,
                            5.,
                            5.,
                            series.color,
                        );
                    }
                }
            }
        }

        PlotLabel::new(labels).paint(&bounds, window, cx);
    }
}

impl IntoElement for PriceChart {
    type Element = PlotElement<Self>;

    fn into_element(self) -> Self::Element {
        PlotElement::new(self)
    }
}
