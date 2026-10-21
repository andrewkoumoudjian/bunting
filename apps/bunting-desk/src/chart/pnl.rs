//! The P&L baseline chart: total derived P&L against run time,
//! drawn around zero — a 1.5 px line coloured by sign, filled to
//! the zero line (never to the plot bottom, which is how
//! `AreaChart` misreads losses), with a right money axis and the
//! latest value tagged on it. One component, two homes: the
//! Portfolio window's P&L section and the P&L Chart window
//! (`PnlPanel`).
//!
//! Composed from Longbridge's plot primitives like
//! [`super::price_chart::PriceChart`]: `Plot`/`PlotElement`,
//! `ScaleLinear`, `Grid`, the `Line` and `Area` shapes (the
//! fill's `y0` sits at the zero line), `PlotLabel`, and the
//! component `Tooltip`/`CrossLine`/`Dot` overlay.
#![allow(
    clippy::float_arithmetic,
    clippy::cast_precision_loss,
    reason = "chart coordinates are f32/f64 by the plot primitives' own API"
)]

use super::range::{nice_ticks, run_clock_axis};
use crate::desk::Desk;
use crate::ui;
use bunting_trading_state::{Model, PnlSample, PnlTotals};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::plot::label::{Text, measure_text_width};
use gpui_kit::component::plot::scale::{Scale as _, ScaleLinear};
use gpui_kit::component::plot::shape::{Area, Line};
use gpui_kit::component::plot::tooltip::{CrossLine, Dot, Tooltip};
use gpui_kit::component::plot::{
    Curve, Grid, Plot, PlotElement, PlotHover, PlotLabel, TooltipState,
};
use gpui_kit::component::{IconName, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Bounds, Context, Div, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Point, Render, SharedString,
    Styled as _, Subscription, TestSupportExt as _, Window, div, fill, point, px, size,
};
use std::collections::VecDeque;

/// Samples drawn at most (the latest ones, one per run second).
const MAX_POINTS: usize = 2_000;
const DEFAULT_GUTTER: f32 = 52.;
const AXIS_FONT: f32 = 11.;

/// One run of points entirely on one side of zero, with the
/// interpolated crossing points at its ends: `(positive, points)`
/// where a point is `(index, value)` in data coordinates. Runs of
/// exact zeros belong to the run that follows them (or the last
/// run); a series that never leaves zero is one positive run.
#[must_use]
pub fn split_at_zero(points: &[(f64, f64)]) -> Vec<(bool, Vec<(f64, f64)>)> {
    let mut runs: Vec<(bool, Vec<(f64, f64)>)> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut sign: Option<bool> = None;
    let mut previous: Option<(f64, f64)> = None;
    for &point in points {
        let (_, value) = point;
        if value != 0. {
            let positive = value > 0.;
            match sign {
                None => sign = Some(positive),
                Some(held) if held != positive => {
                    // Close the old run at the crossing and open
                    // the new one from the same point.
                    if let Some((px, py)) = previous {
                        let cross = px + (point.0 - px) * (py / (py - value));
                        current.push((cross, 0.));
                        let done = std::mem::replace(&mut current, vec![(cross, 0.)]);
                        runs.push((held, done));
                    }
                    sign = Some(positive);
                }
                _ => {}
            }
        }
        current.push(point);
        previous = Some(point);
    }
    if !current.is_empty() {
        runs.push((sign.unwrap_or(true), current));
    }
    runs
}

/// Money in minor units, signed, for axis and stats.
fn money(value: f64) -> String {
    format!("{value:+.0}")
}

/// The stats row shared by both P&L homes: Realized, Unrealized
/// and Total in minor units, coloured by sign. `prefix` keeps the
/// element ids apart when both windows are open.
#[must_use]
pub fn stats_row(prefix: &'static str, totals: PnlTotals, cx: &App) -> Div {
    let theme = cx.theme();
    let signed = |value: i128| match value.cmp(&0) {
        std::cmp::Ordering::Greater => theme.chart_bullish,
        std::cmp::Ordering::Less => theme.chart_bearish,
        std::cmp::Ordering::Equal => theme.foreground,
    };
    let stat = |id: &'static str, label: &'static str, value: i128| {
        div()
            .id(SharedString::from(format!("{prefix}-{id}")))
            .test_support()
            .child(ui::stat_colored(
                label,
                format!("{value:+}"),
                signed(value),
                cx,
            ))
    };
    h_flex()
        .w_full()
        .gap_4()
        .child(stat("realized", "Realized", totals.realized))
        .child(stat("unrealized", "Unrealized", totals.unrealized))
        .child(stat("total", "Total", totals.total))
}

/// The derived note under the figures, with the contract-size
/// caveat while BNP sends no multiplier.
#[must_use]
pub fn derived_note(id: &'static str, totals: PnlTotals, cx: &App) -> impl IntoElement {
    let mut full = "Derived from your fills and the best bid/ask; \
                    the venue's own valuation is not yet on BNP"
        .to_owned();
    let mut short = "Derived from your fills and the best bid/ask".to_owned();
    if !totals.multipliers_known {
        full.push_str(" · assumes contract size 1");
        short.push_str(" · contract size assumed 1*");
    }
    ui::note(id, short, full, cx)
}

/// The chart element for `samples`, filling its container, or the
/// empty line when nothing has been sampled yet.
#[must_use]
pub fn chart_or_empty(id: &'static str, samples: &VecDeque<PnlSample>, cx: &App) -> AnyElement {
    if samples.is_empty() {
        return ui::muted("No fills yet — P&L appears after your first trade", cx)
            .into_any_element();
    }
    let points: Vec<(u64, f64)> = samples
        .iter()
        .skip(samples.len().saturating_sub(MAX_POINTS))
        .map(|sample| (sample.time_ns, sample.total as f64))
        .collect();
    PnlChart::new(SharedString::from(format!("{id}-plot")), points).into_any_element()
}

/// The baseline P&L plot. Points are `(run time ns, total P&L in
/// minor units)`, one per run second from the model.
pub struct PnlChart {
    id: ElementId,
    points: Vec<(u64, f64)>,
    gutter: f32,
    hovered: Option<TooltipState>,
}

impl PnlChart {
    pub fn new(id: impl Into<ElementId>, points: Vec<(u64, f64)>) -> Self {
        Self {
            id: id.into(),
            points,
            gutter: DEFAULT_GUTTER,
            hovered: None,
        }
    }

    fn extent(&self) -> (f64, f64) {
        let (mut lo, mut hi) = (0_f64, 0_f64);
        for (_, value) in &self.points {
            lo = lo.min(*value);
            hi = hi.max(*value);
        }
        if (hi - lo).abs() < f64::EPSILON {
            return (lo - 1., hi + 1.);
        }
        let pad = (hi - lo) * 0.1;
        (lo - pad, hi + pad)
    }

    fn x_at(&self, index: usize, plot_w: f32) -> f32 {
        if self.points.len() <= 1 {
            0.
        } else {
            index as f32 / (self.points.len() - 1) as f32 * plot_w
        }
    }

    fn axis_font(window: &Window) -> f32 {
        window.rem_size().as_f32() * (AXIS_FONT / 14.)
    }
}

impl Plot for PnlChart {
    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn prepaint(
        &mut self,
        _bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut App,
    ) -> Vec<AnyElement> {
        let (lo, hi) = self.extent();
        let (ticks, _) = nice_ticks(lo, hi, 4);
        let font = px(Self::axis_font(window));
        let widest = ticks
            .iter()
            .map(|tick| measure_text_width(&SharedString::from(money(*tick)), font, window))
            .fold(0_f32, f32::max);
        self.gutter = (widest + 14.).clamp(40., 110.);
        Vec::new()
    }

    fn tooltip_state(
        &self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _cx: &App,
    ) -> Option<TooltipState> {
        if self.points.is_empty() {
            return None;
        }
        let plot_w = (bounds.size.width.as_f32() - self.gutter).max(1.);
        let x = position.x.as_f32();
        if !(0. ..=plot_w).contains(&x) {
            return None;
        }
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "x is inside the plot, checked above"
        )]
        let index = if self.points.len() <= 1 {
            0
        } else {
            ((x / plot_w) * (self.points.len() - 1) as f32).round() as usize
        };
        let height = bounds.size.height.as_f32();
        let (lo, hi) = self.extent();
        let y = ScaleLinear::new([lo, hi], [height, 0.]);
        let value = self.points[index].1;
        let point_at = point(
            px(self.x_at(index, plot_w)),
            px(y.tick(&value).unwrap_or(0.)),
        );
        Some(TooltipState::new(index, point_at, vec![point_at]))
    }

    fn hover(&mut self, hover: Option<&PlotHover>, _window: &mut Window, _cx: &mut App) {
        self.hovered = hover
            .filter(|hover| hover.is_hovered())
            .map(|hover| hover.state().clone());
    }

    fn tooltip(
        &self,
        state: &TooltipState,
        cursor: Point<Pixels>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let (time_ns, value) = *self.points.get(state.index)?;
        let color = if value < 0. {
            cx.theme().chart_bearish
        } else {
            cx.theme().chart_bullish
        };
        let dot = *state.dots.first()?;
        Some(
            Tooltip::new(cursor, bounds.size)
                .gap(px(8.))
                .title(format!("{} run time", run_clock_axis(time_ns)))
                .row(color, "Total P&L", money(value))
                .cross_line(CrossLine::new(state.cross_line).both())
                .dots(Some(
                    Dot::new(dot)
                        .size(px(6.))
                        .stroke(cx.theme().background)
                        .fill(color),
                ))
                .into_any_element(),
        )
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one paint pass: grid, split fills and lines, axis, latest tag"
    )]
    #[allow(
        clippy::cast_possible_truncation,
        reason = "sample indices become pixel coordinates, far inside f32's range here"
    )]
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let theme = cx.theme();
        let origin = bounds.origin;
        let height = bounds.size.height.as_f32();
        let plot_w = (bounds.size.width.as_f32() - self.gutter).max(1.);
        let plot = Bounds::new(origin, size(px(plot_w), px(height)));
        let (lo, hi) = self.extent();
        let y = ScaleLinear::new([lo, hi], [height, 0.]);
        let axis_font = Self::axis_font(window);
        let mut labels: Vec<Text> = Vec::new();

        let last_tag_y = self
            .points
            .last()
            .map(|(_, last)| y.tick(last).unwrap_or(0.));
        let (ticks, _) = nice_ticks(lo, hi, 4);
        Grid::new()
            .y(ticks.iter().map(|tick| y.tick(tick).unwrap_or(0.)))
            .stroke(theme.chart_grid)
            .paint(&plot, window);
        for tick in &ticks {
            // The latest-value tag wins its slot on the axis.
            if last_tag_y
                .is_some_and(|tag_y| (y.tick(tick).unwrap_or(0.) - tag_y).abs() < axis_font + 2.)
            {
                continue;
            }
            labels.push(
                Text::new(
                    money(*tick),
                    point(
                        px(plot_w + 6.),
                        px(y.tick(tick).unwrap_or(0.) - axis_font / 2.),
                    ),
                    theme.muted_foreground,
                )
                .font_size(px(axis_font)),
            );
        }

        // Zero line, then the split fills and lines around it.
        let zero_y = y.tick(&0.).unwrap_or(height);
        window.paint_quad(fill(
            Bounds::new(
                point(origin.x, origin.y + px(zero_y - 0.5)),
                size(px(plot_w), px(1.)),
            ),
            theme.muted_foreground.opacity(0.7),
        ));
        let indexed: Vec<(f64, f64)> = self
            .points
            .iter()
            .enumerate()
            .map(|(index, (_, value))| (index as f64, *value))
            .collect();
        for (positive, run) in split_at_zero(&indexed) {
            if run.is_empty() {
                continue;
            }
            let color = if positive {
                theme.chart_bullish
            } else {
                theme.chart_bearish
            };
            let pixels: Vec<(f32, f32)> = run
                .iter()
                .map(|(index, value)| {
                    let x = if self.points.len() <= 1 {
                        0.
                    } else {
                        (*index as f32 / (self.points.len() - 1) as f32) * plot_w
                    };
                    (x, y.tick(value).unwrap_or(zero_y))
                })
                .collect();
            Area::new()
                .data(pixels.clone())
                .x(|point| Some(point.0))
                .y0(zero_y)
                .y1(|point| Some(point.1))
                .fill(color.opacity(0.2))
                .stroke(gpui_kit::transparent_black())
                .curve(Curve::Linear)
                .paint(&plot, window);
            Line::new()
                .data(pixels)
                .x(|point| Some(point.0))
                .y(|point| Some(point.1))
                .stroke(color)
                .stroke_width(px(1.5))
                .curve(Curve::Linear)
                .paint(&plot, window);
        }

        // Latest value tag on the axis.
        if let Some((_, last)) = self.points.last() {
            let color = if *last < 0. {
                theme.chart_bearish
            } else {
                theme.chart_bullish
            };
            let last_y = last_tag_y.unwrap_or(zero_y);
            let tag_h = axis_font + 4.;
            window.paint_quad(fill(
                Bounds::new(
                    point(
                        origin.x + px(plot_w + 1.),
                        origin.y + px(last_y - tag_h / 2.),
                    ),
                    size(px(self.gutter - 2.), px(tag_h)),
                ),
                color,
            ));
            labels.push(
                Text::new(
                    money(*last),
                    point(px(plot_w + self.gutter / 2.), px(last_y - axis_font / 2.)),
                    theme.foreground,
                )
                .font_size(px(axis_font))
                .align(gpui_kit::TextAlign::Center),
            );
        }

        PlotLabel::new(labels).paint(&bounds, window, cx);
    }
}

impl IntoElement for PnlChart {
    type Element = PlotElement<Self>;

    fn into_element(self) -> Self::Element {
        PlotElement::new(self)
    }
}

/// The P&L Chart window: the stats row, the baseline chart at
/// full height, and the derived note.
pub struct PnlPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    _observe: Subscription,
}

impl PnlPanel {
    pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
        }
    }
}

impl Render for PnlPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let model: &Model = desk.model();
        let totals = model.pnl_totals();
        let samples = model.pnl_series();
        let empty = samples.is_empty();
        let mut body = ui::pane_body("pnl", &self.focus)
            .child(stats_row("pnl", totals, cx))
            .child(derived_note("pnl-note", totals, cx));
        body = if empty {
            body.child(ui::empty_state(
                "pnl-empty",
                IconName::Frame,
                "No fills yet — P&L appears after your first trade",
                None,
                cx,
            ))
        } else {
            body.child(
                div()
                    .id("pnl-chart")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(chart_or_empty("pnl", samples, cx)),
            )
        };
        body
    }
}

impl BasePanel for PnlPanel {
    fn panel_name(&self) -> &'static str {
        "pnl"
    }
}

impl Panel for PnlPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::theme::pane_title("P&L Chart", None)
    }
}

impl EventEmitter<PanelEvent> for PnlPanel {}

impl Focusable for PnlPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// Chart height for a Portfolio P&L section offered
/// `region_h` pixels (stats and gaps already accounted): three
/// fifths of what remains, at most the section's 140 px design
/// height.
#[must_use]
pub fn portfolio_chart_h(region_h: f32) -> f32 {
    ((region_h - 59.).max(0.) * 0.6).min(140.)
}

/// The Portfolio window's P&L section content, for
/// `panels::PositionsPanel` to place under its collapsible
/// header. `chart_h` is the height the window can spare for the
/// chart (the panel measures its region): below 24 px the stats
/// stand alone rather than crush the holdings table.
#[must_use]
pub fn portfolio_section(model: &Model, chart_h: f32, cx: &App) -> Div {
    let mut section =
        v_flex()
            .w_full()
            .gap(px(ui::PANE_GAP))
            .child(stats_row("pf-pnl", model.pnl_totals(), cx));
    if chart_h >= 24. {
        section = section.child(
            div()
                .id("pf-pnl-chart")
                .test_support()
                .w_full()
                .h(px(chart_h))
                .flex_shrink(0.)
                .child(chart_or_empty("pf-pnl", model.pnl_series(), cx)),
        );
    }
    section
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixtures use integers and exact halves, so equality is the assertion"
)]
mod tests {
    use super::*;

    #[test]
    fn a_series_that_never_crosses_is_one_run() {
        let points = [(0., 1.), (1., 3.), (2., 2.)];
        assert_eq!(split_at_zero(&points), vec![(true, points.to_vec())]);
        let points = [(0., -1.), (1., -3.)];
        assert_eq!(split_at_zero(&points), vec![(false, points.to_vec())]);
        assert!(split_at_zero(&[]).is_empty());
    }

    #[test]
    fn crossings_are_interpolated_and_shared_by_both_runs() {
        // +2 at 0 to -2 at 2 crosses zero at 1; back up to +2 at 4
        // crosses at 3.
        let points = [(0., 2.), (2., -2.), (4., 2.)];
        assert_eq!(
            split_at_zero(&points),
            vec![
                (true, vec![(0., 2.), (1., 0.)]),
                (false, vec![(1., 0.), (2., -2.), (3., 0.)]),
                (true, vec![(3., 0.), (4., 2.)]),
            ]
        );
    }

    #[test]
    fn a_point_on_zero_stays_with_its_run() {
        let points = [(0., 1.), (1., 0.), (2., -1.)];
        let runs = split_at_zero(&points);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0], (true, vec![(0., 1.), (1., 0.), (1., 0.)]));
        assert_eq!(runs[1], (false, vec![(1., 0.), (2., -1.)]));
    }

    #[test]
    fn leading_zeros_take_the_following_sign_and_flat_is_positive() {
        let points = [(0., 0.), (1., 0.), (2., -4.)];
        assert_eq!(split_at_zero(&points), vec![(false, points.to_vec())]);
        let flat = [(0., 0.), (1., 0.)];
        assert_eq!(split_at_zero(&flat), vec![(true, flat.to_vec())]);
    }
}
