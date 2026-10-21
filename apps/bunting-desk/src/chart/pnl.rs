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
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "chart coordinates are f32/f64 by the plot primitives' own API; run times are non-negative seconds"
)]

use super::range::{nice_ticks, run_clock_axis, time_axis_labels};
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
/// Height reserved under the plot for run-time labels.
const TIME_AXIS_H: f32 = 18.;

/// The stepped polyline of `points` (data coordinates), split
/// into runs of one sign: P&L only changes at events, so each
/// value holds until the next sample (step-after), and a sign
/// change happens at that step — the vertical jump passes
/// through zero at the step's x, never along a diagonal. Each
/// run is `(positive, polyline)`; consecutive points in a run
/// always share an x or a y. Runs of exact zeros belong to the
/// run that follows them; a flat series is one positive run.
#[must_use]
pub fn stepped_runs(points: &[(f64, f64)]) -> Vec<(bool, Vec<(f64, f64)>)> {
    let mut runs: Vec<(bool, Vec<(f64, f64)>)> = Vec::new();
    let mut current: Vec<(f64, f64)> = Vec::new();
    let mut sign: Option<bool> = None;
    let mut previous: Option<(f64, f64)> = None;
    for &(x, value) in points {
        if let Some((_, prev_value)) = previous {
            // Hold the previous value up to this sample's x.
            current.push((x, prev_value));
            if value != 0. {
                let positive = value > 0.;
                match sign {
                    None => {
                        if !positive {
                            // Zeros so far are a flat run of their
                            // own, not the start of the loss.
                            let done = std::mem::replace(&mut current, vec![(x, 0.)]);
                            runs.push((true, done));
                        }
                        sign = Some(positive);
                    }
                    Some(held) if held != positive => {
                        // The jump at this x crosses zero: both
                        // runs own the (x, 0) corner.
                        current.push((x, 0.));
                        let done = std::mem::replace(&mut current, vec![(x, 0.)]);
                        runs.push((held, done));
                        sign = Some(positive);
                    }
                    _ => {}
                }
            }
        } else if value != 0. {
            sign = Some(value > 0.);
        }
        current.push((x, value));
        previous = Some((x, value));
    }
    if !current.is_empty() {
        runs.push((sign.unwrap_or(true), current));
    }
    for (_, run) in &mut runs {
        run.dedup();
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
/// minor units)`, one per run second from the model, drawn on a
/// true time scale (a 36-second gap takes 36 seconds of width)
/// as steps: P&L holds its value between events.
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

    /// Y domain: the samples' range with zero in it, padded.
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

    /// X domain in run-time seconds: first to last sample, with a
    /// breath past the last one.
    fn time_domain(&self) -> (f64, f64) {
        let seconds = |ns: u64| ns as f64 / 1_000_000_000.;
        let Some(first) = self.points.first() else {
            return (0., 60.);
        };
        let last = self.points.last().unwrap_or(first);
        let (start, end) = (seconds(first.0), seconds(last.0));
        if end - start < 1. {
            (start, start + 60.)
        } else {
            (start, end + (end - start) * 0.02 + 1.)
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
        let (ticks, _) = nice_ticks(lo.floor(), hi.ceil(), 4);
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
        let height = (bounds.size.height.as_f32() - TIME_AXIS_H).max(1.);
        let x = position.x.as_f32();
        if !(0.0..=plot_w).contains(&x) {
            return None;
        }
        let (start, end) = self.time_domain();
        let at = start + f64::from(x / plot_w) * (end - start);
        let seconds: Vec<f64> = self
            .points
            .iter()
            .map(|(ns, _)| *ns as f64 / 1_000_000_000.)
            .collect();
        let index = match seconds.binary_search_by(|each| each.total_cmp(&at)) {
            Ok(index) => index,
            Err(0) => 0,
            Err(insert) if insert >= seconds.len() => seconds.len() - 1,
            Err(insert) => {
                if at - seconds[insert - 1] <= seconds[insert] - at {
                    insert - 1
                } else {
                    insert
                }
            }
        };
        let (lo, hi) = self.extent();
        let y = ScaleLinear::new([lo, hi], [height, 0.]);
        let value = self.points[index].1;
        let x_at = ((seconds[index] - start) / (end - start)) as f32 * plot_w;
        let point_at = point(px(x_at), px(y.tick(&value).unwrap_or(0.)));
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
                .cross_line(
                    CrossLine::new(state.cross_line)
                        .both()
                        .height((bounds.size.height.as_f32() - TIME_AXIS_H).max(1.)),
                )
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
        reason = "one paint pass: grid, stepped fills and lines, axes, latest tag"
    )]
    fn paint(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut App) {
        let theme = cx.theme();
        let origin = bounds.origin;
        let height = (bounds.size.height.as_f32() - TIME_AXIS_H).max(1.);
        let plot_w = (bounds.size.width.as_f32() - self.gutter).max(1.);
        let plot = Bounds::new(origin, size(px(plot_w), px(height)));
        let (lo, hi) = self.extent();
        let y = ScaleLinear::new([lo, hi], [height, 0.]);
        let (start, end) = self.time_domain();
        let x_at = |seconds: f64| ((seconds - start) / (end - start)) as f32 * plot_w;
        let axis_font = Self::axis_font(window);
        let mut labels: Vec<Text> = Vec::new();

        let last_tag_y = self
            .points
            .last()
            .map(|(_, last)| y.tick(last).unwrap_or(0.));
        let (ticks, _) = nice_ticks(lo.floor(), hi.ceil(), 4);
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

        // Run-time labels along the bottom, placed so no two can
        // overlap whatever the span (range::time_axis_labels).
        let sample = SharedString::from(run_clock_axis((end as u64) * 1_000_000_000));
        let label_w = measure_text_width(&sample, px(axis_font), window);
        for (at, centered) in time_axis_labels(start, end, plot_w, label_w) {
            labels.push(
                Text::new(
                    run_clock_axis((at as u64) * 1_000_000_000),
                    point(px(centered), px(height + 3.)),
                    theme.muted_foreground,
                )
                .font_size(px(axis_font))
                .align(gpui_kit::TextAlign::Center),
            );
        }

        // Zero line, then the stepped fills and lines around it.
        let zero_y = y.tick(&0.).unwrap_or(height);
        window.paint_quad(fill(
            Bounds::new(
                point(origin.x, origin.y + px(zero_y - 0.5)),
                size(px(plot_w), px(1.)),
            ),
            theme.muted_foreground.opacity(0.7),
        ));
        let timed: Vec<(f64, f64)> = self
            .points
            .iter()
            .map(|(ns, value)| (*ns as f64 / 1_000_000_000., *value))
            .collect();
        for (positive, run) in stepped_runs(&timed) {
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
                .map(|(seconds, value)| (x_at(*seconds), y.tick(value).unwrap_or(zero_y)))
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

        // Latest value tag on the axis, text in the fill's own
        // contrasting foreground token.
        if let Some((_, last)) = self.points.last() {
            let (color, text_color) = if *last < 0. {
                (theme.chart_bearish, theme.danger_foreground)
            } else {
                (theme.chart_bullish, theme.success_foreground)
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
                    text_color,
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
    reason = "fixtures use whole seconds and values, so equality is the assertion"
)]
mod tests {
    use super::*;

    #[test]
    fn a_series_that_never_crosses_is_one_stepped_run() {
        let points = [(0., 1.), (1., 3.), (2., 2.)];
        assert_eq!(
            stepped_runs(&points),
            vec![(true, vec![(0., 1.), (1., 1.), (1., 3.), (2., 3.), (2., 2.)])]
        );
        assert!(stepped_runs(&[]).is_empty());
    }

    #[test]
    fn sign_changes_happen_at_the_step_never_on_a_diagonal() {
        // The review case: flat at zero, then the first fill at
        // 36 s. The line holds 0 to 36, then jumps — no ramp.
        let points = [(0., 0.), (36., -25.), (60., -25.)];
        let runs = stepped_runs(&points);
        assert_eq!(
            runs,
            vec![
                (true, vec![(0., 0.), (36., 0.)]),
                (false, vec![(36., 0.), (36., -25.), (60., -25.)]),
            ]
        );
        for (_, run) in &runs {
            for pair in run.windows(2) {
                assert!(
                    pair[0].0 == pair[1].0 || pair[0].1 == pair[1].1,
                    "every segment is horizontal or vertical: {pair:?}"
                );
            }
        }
    }

    #[test]
    fn crossings_share_the_zero_corner_at_the_step() {
        let points = [(0., 10.), (10., -5.), (20., 5.)];
        assert_eq!(
            stepped_runs(&points),
            vec![
                (true, vec![(0., 10.), (10., 10.), (10., 0.)]),
                (false, vec![(10., 0.), (10., -5.), (20., -5.), (20., 0.)]),
                (true, vec![(20., 0.), (20., 5.)]),
            ]
        );
    }

    #[test]
    fn leading_zeros_are_a_flat_run_before_the_first_step() {
        let points = [(0., 0.), (1., 0.), (2., -4.)];
        assert_eq!(
            stepped_runs(&points),
            vec![
                (true, vec![(0., 0.), (1., 0.), (2., 0.)]),
                (false, vec![(2., 0.), (2., -4.)]),
            ]
        );
        let flat = [(0., 0.), (1., 0.)];
        assert_eq!(stepped_runs(&flat), vec![(true, vec![(0., 0.), (1., 0.)])]);
    }
}
