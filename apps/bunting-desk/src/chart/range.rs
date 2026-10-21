//! The pure geometry of the Charting window: which bars are
//! visible, where axis ticks fall, which bar size to start from,
//! and how comparison series are normalised. No GPUI in here:
//! every rule is unit-tested below.
#![allow(
    clippy::float_arithmetic,
    clippy::cast_precision_loss,
    reason = "chart coordinates and bar counts are f64 by the plot primitives' own API"
)]

use bunting_trading_state::{BAR_SIZES_NS, FINEST_BAR_NS};

/// Fewest bars a zoom may show.
pub const MIN_VISIBLE: usize = 10;
/// Most bars a zoom may show (a whole run at fine sizes).
pub const MAX_VISIBLE: usize = 2_000;
/// Bars shown when a chart first opens.
pub const DEFAULT_VISIBLE: usize = 80;
/// Index into [`BAR_SIZES_NS`] a chart starts from: 5 s. At 1 s a
/// typical bar holds about one trade and candles collapse to
/// dashes (owner review, 2026-10-20).
pub const DEFAULT_BAR: usize = 1;

/// Which bars of a series are on screen: a count and the end it
/// counts back from. `end: None` follows the latest bar — new bars
/// slide the window; any explicit end stays where the user put it
/// while new bars accumulate off-screen to the right.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewRange {
    count: usize,
    end: Option<usize>,
}

impl Default for ViewRange {
    fn default() -> Self {
        Self {
            count: DEFAULT_VISIBLE,
            end: None,
        }
    }
}

impl ViewRange {
    /// The `[start, end)` bar indices to draw out of `total` bars.
    #[must_use]
    pub fn resolve(&self, total: usize) -> (usize, usize) {
        let count = self.count.clamp(1, total.max(1)).min(total);
        let end = self
            .end
            .map_or(total, |end| end.clamp(count, total.max(count)));
        let end = end.min(total.max(count));
        (end.saturating_sub(count), end.min(total))
    }

    /// How many bars the range asks for (before clamping to data).
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Whether the window is pinned to the latest bar.
    #[must_use]
    pub fn is_following(&self, total: usize) -> bool {
        self.end.is_none() || self.resolve(total).1 == total
    }

    /// Follow the latest bar again, keeping the count.
    pub fn follow_latest(&mut self) {
        self.end = None;
    }

    /// Pins the window where it currently resolves, so later data
    /// growth no longer moves it. Used when a drag starts.
    fn pin(&mut self, total: usize) {
        let (_, end) = self.resolve(total);
        self.end = Some(end);
    }

    /// Zooms by `scale` (below 1 shows fewer bars) around the bar
    /// at `anchor` (0 at the left edge of the window, 1 at the
    /// right): that bar stays at the same fraction of the window.
    pub fn zoom(&mut self, total: usize, anchor: f64, scale: f64) {
        if total == 0 || !scale.is_finite() || scale <= 0. {
            return;
        }
        let (start, end) = self.resolve(total);
        let shown = (end - start).max(1);
        #[allow(
            clippy::cast_precision_loss,
            reason = "bar counts, far inside f64's exact integers"
        )]
        let anchor_bar = start as f64 + anchor.clamp(0., 1.) * shown as f64;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to the visible-count bounds right here"
        )]
        let count = (shown as f64 * scale).round().clamp(
            MIN_VISIBLE.min(total) as f64,
            MAX_VISIBLE.min(total.max(MIN_VISIBLE)) as f64,
        ) as usize;
        self.count = count;
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped into the data range right here"
        )]
        let new_end = (anchor_bar + (1. - anchor.clamp(0., 1.)) * count as f64)
            .round()
            .clamp(count.min(total) as f64, total.max(count.min(total)) as f64)
            as usize;
        self.end = if new_end >= total {
            None
        } else {
            Some(new_end)
        };
    }

    /// Pans by `delta` bars: positive moves toward later bars.
    /// Landing on the latest bar resumes following.
    pub fn pan(&mut self, total: usize, delta: isize) {
        if total == 0 || delta == 0 {
            return;
        }
        self.pin(total);
        let count = self.resolve(total).1 - self.resolve(total).0;
        let end = self.end.unwrap_or(total);
        let new_end = end
            .saturating_add_signed(delta)
            .clamp(count, total.max(count));
        self.end = if new_end >= total {
            None
        } else {
            Some(new_end)
        };
    }
}

/// Nice axis ticks inside `[min, max]`: steps of 1, 2 or 5 × 10ⁿ,
/// about `target` of them, plus the decimal places their labels
/// need. An empty span yields the single value.
#[must_use]
pub fn nice_ticks(min: f64, max: f64, target: usize) -> (Vec<f64>, usize) {
    if !min.is_finite() || !max.is_finite() || target == 0 {
        return (Vec::new(), 0);
    }
    let (min, max) = if min <= max { (min, max) } else { (max, min) };
    if max - min <= 0. {
        return (vec![min], decimals_for(1.));
    }
    let raw = (max - min) / target.max(1) as f64;
    let magnitude = 10_f64.powf(raw.log10().floor());
    let step = [1., 2., 5., 10.]
        .into_iter()
        .map(|factor| factor * magnitude)
        .find(|step| *step >= raw)
        .unwrap_or(10. * magnitude);
    let decimals = decimals_for(step);
    let first = (min / step).ceil() * step;
    let mut ticks = Vec::new();
    let mut value = first;
    // At most a screenful; the loop bound also guards float drift.
    while value <= max + step * 1e-9 && ticks.len() <= target * 3 + 2 {
        ticks.push(value);
        value += step;
    }
    (ticks, decimals)
}

fn decimals_for(step: f64) -> usize {
    if step >= 1. {
        0
    } else {
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "bounded by the clamp right here"
        )]
        {
            (-step.log10()).ceil().clamp(0., 6.) as usize
        }
    }
}

/// A tick label at `decimals` places, with `-0` read as `0`.
#[must_use]
pub fn format_tick(value: f64, decimals: usize) -> String {
    let text = format!("{value:.decimals$}");
    if text.starts_with('-') && text.trim_start_matches(['-', '0', '.']).is_empty() {
        text.trim_start_matches('-').to_owned()
    } else {
        text
    }
}

/// Candidate run-time steps for the bottom axis, in seconds.
const TIME_STEPS_S: [u64; 13] = [1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

/// The smallest time step (ns) whose labels — `label_px` wide on
/// bars `band_px` apart — keep at least 8 px between neighbours.
/// Steps are whole multiples of the bar size so labels land on bar
/// starts. Measuring happens in the caller; this only picks.
#[must_use]
pub fn pick_time_step(bar_ns: u64, band_px: f32, label_px: f32) -> u64 {
    let bar_seconds = (bar_ns / 1_000_000_000).max(1);
    for step_s in TIME_STEPS_S {
        if step_s % bar_seconds != 0 {
            continue;
        }
        #[allow(clippy::cast_precision_loss, reason = "small whole-second ratios")]
        let spacing = band_px * (step_s / bar_seconds) as f32;
        if spacing >= label_px + 8. {
            return step_s * 1_000_000_000;
        }
    }
    let step_s = TIME_STEPS_S[TIME_STEPS_S.len() - 1].max(bar_seconds);
    step_s.div_ceil(bar_seconds) * bar_ns.max(FINEST_BAR_NS)
}

/// A run-clock instant for the axis: `m:ss`, or `h:mm:ss` once the
/// run passes an hour.
#[must_use]
pub fn run_clock_axis(nanos: u64) -> String {
    let seconds = nanos / 1_000_000_000;
    if seconds >= 3600 {
        format!(
            "{}:{:02}:{:02}",
            seconds / 3600,
            (seconds % 3600) / 60,
            seconds % 60
        )
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// The smallest bar size in [`BAR_SIZES_NS`] whose bars average at
/// least ~2 *active seconds* each, from `active` one-second bars
/// spread over `span_ns`. Finest bars do not retain a trade count,
/// so seconds that traded stand in for trades. Too little data
/// takes the 5 s default.
#[must_use]
pub fn pick_bar_size(active: usize, span_ns: u64) -> usize {
    let span_seconds = span_ns / FINEST_BAR_NS;
    if active < 8 || span_seconds == 0 {
        return DEFAULT_BAR;
    }
    // Bar seconds needed so that (active / (span / bar_s)) >= 2.
    let needed_s = (2 * span_seconds).div_ceil(active as u64);
    BAR_SIZES_NS
        .iter()
        .position(|&size| size / FINEST_BAR_NS >= needed_s)
        .unwrap_or(BAR_SIZES_NS.len() - 1)
}

/// Percent change of each close from `base` (the first visible
/// bar's close). A zero base yields zeros rather than infinities.
#[must_use]
pub fn percent_changes(closes: &[f64], base: f64) -> Vec<f64> {
    if base == 0. {
        return vec![0.; closes.len()];
    }
    closes
        .iter()
        .map(|close| (close / base - 1.) * 100.)
        .collect()
}

#[cfg(test)]
#[allow(
    clippy::float_cmp,
    reason = "fixtures use integers and exact halves, so equality is the assertion"
)]
mod tests {
    use super::*;

    const SEC: u64 = FINEST_BAR_NS;

    #[test]
    fn the_default_range_follows_the_latest_eighty() {
        let range = ViewRange::default();
        assert_eq!(range.resolve(500), (420, 500));
        assert_eq!(range.resolve(40), (0, 40));
        assert!(range.is_following(500));
    }

    #[test]
    fn zoom_keeps_the_anchored_bar_at_its_fraction() {
        let mut range = ViewRange::default();
        // 200 bars shown 120..200 after panning to a 80-wide window? Start exact:
        assert_eq!(range.resolve(200), (120, 200));
        // Anchor at the left edge: bar 120 stays the left edge.
        let mut left = range;
        left.zoom(200, 0., 0.5);
        assert_eq!(left.resolve(200), (120, 160));
        // Anchor mid-window (bar 160): halving keeps it centred.
        range.zoom(200, 0.5, 0.5);
        let (start, end) = range.resolve(200);
        assert_eq!(end - start, 40);
        assert!((160 - start).abs_diff(end - 160) <= 1, "{start}..{end}");
        // Zooming out past the data clamps and follows again.
        range.zoom(200, 0.5, 100.);
        assert_eq!(range.resolve(200), (0, 200));
        assert!(range.is_following(200));
    }

    #[test]
    fn zoom_never_shows_fewer_than_ten_or_more_than_the_data() {
        let mut range = ViewRange::default();
        range.zoom(500, 1., 0.0001);
        let (start, end) = range.resolve(500);
        assert_eq!(end - start, MIN_VISIBLE);
        assert_eq!(end, 500, "anchored at the right edge stays latest");
        let mut few = ViewRange::default();
        few.zoom(6, 0.5, 0.1);
        assert_eq!(few.resolve(6), (0, 6));
    }

    #[test]
    fn pan_moves_by_bars_and_following_resumes_at_the_end() {
        let mut range = ViewRange::default();
        range.pan(500, -30);
        assert_eq!(range.resolve(500), (390, 470));
        assert!(!range.is_following(500));
        // New bars do not move a panned window.
        assert_eq!(range.resolve(510), (390, 470));
        range.pan(510, 10);
        assert_eq!(range.resolve(510), (400, 480));
        range.pan(510, 10_000);
        assert!(range.is_following(510));
        range.pan(510, -10_000);
        assert_eq!(range.resolve(510), (0, 80));
        range.follow_latest();
        assert_eq!(range.resolve(510), (430, 510));
    }

    #[test]
    fn nice_ticks_step_one_two_or_five_times_a_power_of_ten() {
        let (ticks, decimals) = nice_ticks(0., 100., 5);
        assert_eq!(ticks, vec![0., 20., 40., 60., 80., 100.]);
        assert_eq!(decimals, 0);
        let (ticks, decimals) = nice_ticks(98.2, 101.7, 5);
        assert_eq!(ticks, vec![99., 100., 101.]);
        assert_eq!(decimals, 0);
        let (ticks, decimals) = nice_ticks(-3., 7., 5);
        assert_eq!(ticks, vec![-2., 0., 2., 4., 6.]);
        assert_eq!(decimals, 0);
        let (ticks, decimals) = nice_ticks(0., 0.9, 4);
        assert_eq!(ticks, vec![0., 0.5]);
        assert_eq!(decimals, 1);
        assert_eq!(nice_ticks(5., 5., 5).0, vec![5.]);
        assert_eq!(format_tick(-0.0, 0), "0");
        assert_eq!(format_tick(2.5, 1), "2.5");
    }

    #[test]
    fn time_steps_are_bar_multiples_with_room_between_labels() {
        // 5 s bars 6 px apart, 28 px labels: 30 s gives 36 px.
        assert_eq!(pick_time_step(5 * SEC, 6., 28.), 30 * SEC);
        // Roomy 1 s bars: every 5 s is enough for 30 px labels.
        assert_eq!(pick_time_step(SEC, 8., 30.), 5 * SEC);
        // A 1 min bar never labels inside itself.
        assert_eq!(pick_time_step(60 * SEC, 10., 10.), 120 * SEC);
    }

    #[test]
    fn the_axis_clock_gains_hours_after_an_hour() {
        assert_eq!(run_clock_axis(0), "0:00");
        assert_eq!(run_clock_axis(599 * SEC), "9:59");
        assert_eq!(run_clock_axis(3600 * SEC), "1:00:00");
        assert_eq!(run_clock_axis(3723 * SEC), "1:02:03");
    }

    #[test]
    fn bar_size_picking_targets_two_active_seconds_per_bar() {
        // Too little data: the 5 s default.
        assert_eq!(pick_bar_size(3, 60 * SEC), DEFAULT_BAR);
        // Every second traded for a minute: 1 s bars average 1
        // active second, so 5 s is the first size reaching 2.
        assert_eq!(pick_bar_size(60, 60 * SEC), 1);
        // A trade every ~10 s over ten minutes: needs 20 s -> 1 min? No:
        // sizes are 1/5/15/60/300 s; 15 s gives 1.5, so 60 s.
        assert_eq!(pick_bar_size(60, 600 * SEC), 3);
        // Busy tape, two active seconds needed: dense 1 s data with
        // span equal to active still needs 2 s -> 5 s.
        assert_eq!(BAR_SIZES_NS[pick_bar_size(600, 600 * SEC)], 5 * SEC);
    }

    #[test]
    fn percent_changes_measure_from_the_base_close() {
        assert_eq!(
            percent_changes(&[100., 125., 75.], 100.),
            vec![0., 25., -25.]
        );
        assert_eq!(percent_changes(&[5., 6.], 0.), vec![0., 0.]);
    }
}
