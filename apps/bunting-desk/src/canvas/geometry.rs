//! Pure window geometry for the canvas (ADR 0051 decision 2): snapping,
//! resizing, clamping, edge hit-testing, tiling and cascading. No GPUI
//! types appear here; everything is plain `f32` logical pixels, unit-tested
//! without a window.

use serde::{Deserialize, Serialize};

/// The gap kept between windows and inside the canvas edges. Windows have
/// a 6 px radius and a 1 px border, so touching windows would double the
/// border and notch at the corners; snapping, tiling, cascading,
/// maximising and templates all keep this gutter instead.
pub const GUTTER: f32 = 6.;

/// A rectangle in logical pixels, in canvas coordinates.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A point in canvas coordinates.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

/// An edge or corner of a window being dragged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Edge {
    #[must_use]
    pub const fn has_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    #[must_use]
    pub const fn has_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    #[must_use]
    pub const fn has_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    #[must_use]
    pub const fn has_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

/// A guide line a drag snapped to: a vertical line at `x`, or a horizontal
/// line at `y`, in canvas coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Guide {
    Vertical(f32),
    Horizontal(f32),
}

/// The result of snapping: the adjusted rect and the guides it snapped to.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapped {
    pub rect: Rect,
    pub guides: Vec<Guide>,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    #[must_use]
    pub const fn right(self) -> f32 {
        self.x + self.w
    }

    #[must_use]
    pub const fn bottom(self) -> f32 {
        self.y + self.h
    }

    #[must_use]
    pub fn contains(self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    /// Whether the two rects overlap with positive area. Touching edges do
    /// not overlap.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    #[must_use]
    pub const fn area(self) -> f32 {
        self.w * self.h
    }

    /// This rect shrunk by `by` on every side (never below zero size).
    #[must_use]
    pub fn inset(self, by: f32) -> Self {
        Self::new(
            self.x + by,
            self.y + by,
            (self.w - by * 2.).max(0.),
            (self.h - by * 2.).max(0.),
        )
    }
}

/// Snap targets for the near (left/top) and far (right/bottom) edges on
/// one axis. Aligned snaps meet exactly; adjacent snaps keep [`GUTTER`];
/// canvas edges are inset by [`GUTTER`]. `others` are `(near, far)` pairs.
fn axis_targets(canvas_near: f32, canvas_far: f32, others: &[(f32, f32)]) -> (Vec<f32>, Vec<f32>) {
    let mut near = vec![canvas_near + GUTTER];
    let mut far = vec![canvas_far - GUTTER];
    for &(other_near, other_far) in others {
        near.push(other_near);
        near.push(other_far + GUTTER);
        far.push(other_far);
        far.push(other_near - GUTTER);
    }
    (near, far)
}

/// The best snap offset for an edge against its targets, within
/// `threshold`. Returns the offset and the coordinate snapped to.
fn best_snap(edge: f32, targets: &[f32], threshold: f32) -> Option<(f32, f32)> {
    let mut best: Option<(f32, f32)> = None;
    for &target in targets {
        let offset = target - edge;
        if offset.abs() <= threshold && best.is_none_or(|(held, _)| offset.abs() < held.abs()) {
            best = Some((offset, target));
        }
    }
    best
}

/// The better of the near and far edges' snaps on one axis.
fn snap_axis(
    near: f32,
    far: f32,
    targets: &(Vec<f32>, Vec<f32>),
    threshold: f32,
) -> Option<(f32, f32)> {
    match (
        best_snap(near, &targets.0, threshold),
        best_snap(far, &targets.1, threshold),
    ) {
        (Some(a), Some(b)) => Some(if a.0.abs() <= b.0.abs() { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

/// Snaps a moving window to the canvas edges (inset by [`GUTTER`]) and to
/// other windows' edges — aligned exactly, or adjacent with exactly
/// [`GUTTER`] between — within `threshold`.
#[must_use]
pub fn snap_move(moving: Rect, others: &[Rect], canvas: Rect, threshold: f32) -> Snapped {
    let verticals = axis_targets(
        canvas.x,
        canvas.right(),
        &others
            .iter()
            .map(|other| (other.x, other.right()))
            .collect::<Vec<_>>(),
    );
    let horizontals = axis_targets(
        canvas.y,
        canvas.bottom(),
        &others
            .iter()
            .map(|other| (other.y, other.bottom()))
            .collect::<Vec<_>>(),
    );
    let mut rect = moving;
    let mut guides = Vec::new();
    if let Some((dx, line)) = snap_axis(moving.x, moving.right(), &verticals, threshold) {
        rect.x += dx;
        guides.push(Guide::Vertical(line));
    }
    if let Some((dy, line)) = snap_axis(moving.y, moving.bottom(), &horizontals, threshold) {
        rect.y += dy;
        guides.push(Guide::Horizontal(line));
    }
    Snapped { rect, guides }
}

/// The best snap target for one dragged coordinate.
fn snap_one(edge: f32, targets: &[f32], threshold: f32) -> Option<f32> {
    best_snap(edge, targets, threshold).map(|(offset, _)| edge + offset)
}

/// Snaps the edges being dragged (named by `edge`) to the canvas and to
/// other windows, like [`snap_move`]. The undragged edges stay put; the
/// caller enforces minimum sizes before and after.
#[must_use]
pub fn snap_resize(
    rect: Rect,
    edge: Edge,
    others: &[Rect],
    canvas: Rect,
    threshold: f32,
) -> Snapped {
    let verticals = axis_targets(
        canvas.x,
        canvas.right(),
        &others
            .iter()
            .map(|other| (other.x, other.right()))
            .collect::<Vec<_>>(),
    );
    let horizontals = axis_targets(
        canvas.y,
        canvas.bottom(),
        &others
            .iter()
            .map(|other| (other.y, other.bottom()))
            .collect::<Vec<_>>(),
    );
    let mut out = rect;
    let mut guides = Vec::new();
    if edge.has_left()
        && let Some(x) = snap_one(rect.x, &verticals.0, threshold)
    {
        out.w = rect.right() - x;
        out.x = x;
        guides.push(Guide::Vertical(x));
    }
    if edge.has_right()
        && let Some(right) = snap_one(rect.right(), &verticals.1, threshold)
    {
        out.w = right - rect.x;
        guides.push(Guide::Vertical(right));
    }
    if edge.has_top()
        && let Some(y) = snap_one(rect.y, &horizontals.0, threshold)
    {
        out.h = rect.bottom() - y;
        out.y = y;
        guides.push(Guide::Horizontal(y));
    }
    if edge.has_bottom()
        && let Some(bottom) = snap_one(rect.bottom(), &horizontals.1, threshold)
    {
        out.h = bottom - rect.y;
        guides.push(Guide::Horizontal(bottom));
    }
    Snapped { rect: out, guides }
}

/// Moves `start`'s dragged edges by `dx`/`dy`, keeping at least the minimum
/// size (`min_w`, `min_h`). The undragged edges stay put.
#[must_use]
pub fn resize_rect(start: Rect, edge: Edge, dx: f32, dy: f32, min: (f32, f32)) -> Rect {
    let mut rect = start;
    if edge.has_left() {
        let right = start.right();
        rect.x = (start.x + dx).min(right - min.0);
        rect.w = right - rect.x;
    }
    if edge.has_right() {
        rect.w = (start.w + dx).max(min.0);
    }
    if edge.has_top() {
        let bottom = start.bottom();
        rect.y = (start.y + dy).min(bottom - min.1);
        rect.h = bottom - rect.y;
    }
    if edge.has_bottom() {
        rect.h = (start.h + dy).max(min.1);
    }
    rect
}

/// Keeps at least `min` of size, growing right and down from the corner.
#[must_use]
pub fn enforce_min(rect: Rect, min: (f32, f32)) -> Rect {
    Rect {
        w: rect.w.max(min.0),
        h: rect.h.max(min.1),
        ..rect
    }
}

/// Moves `rect` so its title strip stays reachable inside `canvas`: the top
/// is never above the canvas and never below `min_visible_title` from the
/// canvas bottom, and at least 64 px of the title's width stays inside.
/// The size is unchanged.
#[must_use]
pub fn clamp_to_canvas(rect: Rect, canvas: Rect, min_visible_title: f32) -> Rect {
    const MIN_TITLE_WIDTH: f32 = 64.;
    let visible_w = MIN_TITLE_WIDTH.min(rect.w.max(0.));
    let min_x = canvas.x - (rect.w - visible_w).max(0.);
    let max_x = (canvas.right() - visible_w).max(min_x);
    let max_y = (canvas.bottom() - min_visible_title.min(canvas.h.max(0.))).max(canvas.y);
    Rect {
        x: rect.x.clamp(min_x, max_x),
        y: rect.y.clamp(canvas.y, max_y),
        ..rect
    }
}

/// Which edge or corner of `rect` is under `point`, within `grip` px of the
/// border. Corners win over edges; inside or far outside gives `None`.
#[must_use]
pub fn hit_edge(rect: Rect, point: Point, grip: f32) -> Option<Edge> {
    let outside = Rect::new(
        rect.x - grip,
        rect.y - grip,
        rect.w + grip * 2.,
        rect.h + grip * 2.,
    );
    if !outside.contains(point) {
        return None;
    }
    let left = (point.x - rect.x).abs() <= grip;
    let right = (point.x - rect.right()).abs() <= grip;
    let top = (point.y - rect.y).abs() <= grip;
    let bottom = (point.y - rect.bottom()).abs() <= grip;
    match (left, right, top, bottom) {
        (true, _, true, _) => Some(Edge::TopLeft),
        (_, true, true, _) => Some(Edge::TopRight),
        (true, _, _, true) => Some(Edge::BottomLeft),
        (_, true, _, true) => Some(Edge::BottomRight),
        (true, _, _, _) => Some(Edge::Left),
        (_, true, _, _) => Some(Edge::Right),
        (_, _, true, _) => Some(Edge::Top),
        (_, _, _, true) => Some(Edge::Bottom),
        _ => None,
    }
}

/// A near-square grid of `n` rects filling the canvas inset by [`GUTTER`],
/// with `gap` px between cells. Rows are filled as evenly as possible, so
/// the cells cover the inset canvas exactly and never overlap.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "window counts are tiny; f32 holds them exactly"
)]
pub fn tile(n: usize, canvas: Rect, gap: f32) -> Vec<Rect> {
    let canvas = canvas.inset(GUTTER);
    if n == 0 || canvas.w <= 0. || canvas.h <= 0. {
        return Vec::new();
    }
    // Pick the column count whose grid is nearest to square, penalising
    // empty cells; ties go to more columns on a wide canvas.
    let mut best: Option<(f32, usize, usize)> = None;
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let waste = (cols * rows - n) as f32;
        let skew = ((rows as f32) / (cols as f32)).ln().abs();
        let score = skew + waste * 0.5;
        let better = best.is_none_or(|(held, held_cols, _)| {
            score < held - f32::EPSILON
                || ((score - held).abs() <= f32::EPSILON && cols > held_cols)
        });
        if better {
            best = Some((score, cols, rows));
        }
    }
    let Some((_, _, rows)) = best else {
        return Vec::new();
    };
    let row_h = (canvas.h - gap * (rows as f32 - 1.)) / rows as f32;
    let mut rects = Vec::with_capacity(n);
    let mut y = canvas.y;
    let mut placed = 0usize;
    for row in 0..rows {
        // Earlier rows take the remainder, so counts differ by at most one.
        let count = n / rows + usize::from(row < n % rows);
        let cell_w = (canvas.w - gap * (count as f32 - 1.)) / count as f32;
        let mut x = canvas.x;
        for _ in 0..count {
            if placed < n {
                rects.push(Rect::new(x, y, cell_w, row_h));
                placed += 1;
            }
            x += cell_w + gap;
        }
        y += row_h + gap;
    }
    rects
}

/// `n` same-sized rects fanned out from the top-left of the canvas inset
/// by [`GUTTER`], by `step` px each, wrapping to the start when the next
/// step would leave the inset canvas. Each window keeps at least half the
/// inset canvas in each dimension.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "window counts are tiny; f32 holds them exactly"
)]
pub fn cascade(n: usize, canvas: Rect, step: f32) -> Vec<Rect> {
    let canvas = canvas.inset(GUTTER);
    if n == 0 || canvas.w <= 0. || canvas.h <= 0. {
        return Vec::new();
    }
    let spread = step * (n - 1) as f32;
    let w = (canvas.w - spread).clamp(canvas.w * 0.5, canvas.w);
    let h = (canvas.h - spread).clamp(canvas.h * 0.5, canvas.h);
    let steps_x = if step > 0. {
        ((canvas.w - w) / step).floor().max(0.) as usize + 1
    } else {
        1
    };
    let steps_y = if step > 0. {
        ((canvas.h - h) / step).floor().max(0.) as usize + 1
    } else {
        1
    };
    let cycle = steps_x.min(steps_y).max(1);
    (0..n)
        .map(|index| {
            let at = (index % cycle) as f32 * step;
            Rect::new(canvas.x + at, canvas.y + at, w, h)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANVAS: Rect = Rect::new(0., 0., 1200., 800.);

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.01
    }

    #[test]
    fn snap_move_aligns_to_canvas_edges_inset_by_the_gutter() {
        let moving = Rect::new(5., 795. - 100., 200., 100.);
        let snapped = snap_move(moving, &[], CANVAS, 8.);
        assert!(close(snapped.rect.x, GUTTER));
        assert!(close(snapped.rect.bottom(), 800. - GUTTER));
        assert_eq!(
            snapped.guides,
            vec![Guide::Vertical(GUTTER), Guide::Horizontal(800. - GUTTER)]
        );
    }

    #[test]
    fn snap_move_adjacent_to_another_window_keeps_the_gutter() {
        let other = Rect::new(100., 100., 200., 150.);
        let moving = Rect::new(305., 104., 180., 120.);
        let snapped = snap_move(moving, &[other], CANVAS, 8.);
        assert!(
            close(snapped.rect.x, other.right() + GUTTER),
            "exactly the gutter after it"
        );
        assert!(close(snapped.rect.y, other.y), "tops aligned");
        assert!(snapped.guides.contains(&Guide::Vertical(306.)));
    }

    #[test]
    fn snap_move_leaves_a_far_window_alone() {
        let moving = Rect::new(50., 60., 200., 100.);
        let snapped = snap_move(moving, &[Rect::new(600., 400., 100., 100.)], CANVAS, 8.);
        assert_eq!(snapped.rect, moving);
        assert!(snapped.guides.is_empty());
    }

    #[test]
    fn snap_resize_moves_only_the_dragged_edge() {
        let other = Rect::new(500., 100., 200., 150.);
        let rect = Rect::new(100., 100., 395., 150.);
        // Dragging right towards the other window stops the gutter short
        // of its left edge.
        let snapped = snap_resize(rect, Edge::Right, &[other], CANVAS, 8.);
        assert!(close(snapped.rect.right(), 500. - GUTTER));
        assert!(close(snapped.rect.x, 100.));
        assert_eq!(snapped.guides, vec![Guide::Vertical(500. - GUTTER)]);
        // The left edge is out of range and the top was not dragged.
        let snapped = snap_resize(rect, Edge::Left, &[other], CANVAS, 8.);
        assert_eq!(snapped.rect, rect);
    }

    #[test]
    fn resize_rect_honours_the_minimum_from_every_side() {
        let start = Rect::new(100., 100., 200., 150.);
        let min = (120., 90.);
        let shrunk = resize_rect(start, Edge::Right, -500., 0., min);
        assert!(close(shrunk.w, 120.) && close(shrunk.x, 100.));
        let shrunk = resize_rect(start, Edge::BottomLeft, 500., -500., min);
        assert!(close(shrunk.w, 120.) && close(shrunk.right(), start.right()));
        assert!(close(shrunk.h, 90.) && close(shrunk.y, 100.));
        let grown = resize_rect(start, Edge::TopLeft, -20., -30., min);
        assert_eq!(grown, Rect::new(80., 70., 220., 180.));
    }

    #[test]
    fn clamp_keeps_the_title_reachable() {
        let off_bottom = clamp_to_canvas(Rect::new(40., 900., 300., 200.), CANVAS, 28.);
        assert!(close(off_bottom.y, 800. - 28.));
        let off_top = clamp_to_canvas(Rect::new(40., -50., 300., 200.), CANVAS, 28.);
        assert!(close(off_top.y, 0.));
        let off_right = clamp_to_canvas(Rect::new(2000., 40., 300., 200.), CANVAS, 28.);
        assert!(close(off_right.x, 1200. - 64.));
        let off_left = clamp_to_canvas(Rect::new(-1000., 40., 300., 200.), CANVAS, 28.);
        assert!(close(off_left.right(), 64.));
        let inside = Rect::new(40., 40., 300., 200.);
        assert_eq!(clamp_to_canvas(inside, CANVAS, 28.), inside);
    }

    #[test]
    fn hit_edge_prefers_corners_and_misses_the_middle() {
        let rect = Rect::new(100., 100., 200., 150.);
        let at = |x, y| hit_edge(rect, Point { x, y }, 6.);
        assert_eq!(at(300., 250.), Some(Edge::BottomRight));
        assert_eq!(at(102., 175.), Some(Edge::Left));
        assert_eq!(at(200., 98.), Some(Edge::Top));
        assert_eq!(at(200., 175.), None);
        assert_eq!(at(500., 500.), None);
    }

    #[test]
    fn tile_fills_the_inset_canvas_without_overlap() {
        let inner = CANVAS.inset(GUTTER);
        for n in [1usize, 2, 3, 4, 5, 7] {
            let rects = tile(n, CANVAS, 0.);
            assert_eq!(rects.len(), n);
            let total: f32 = rects.iter().map(|rect| rect.area()).sum();
            assert!(close(total, inner.area()), "n={n} covers the inset canvas");
            for (index, rect) in rects.iter().enumerate() {
                assert!(rect.x >= inner.x && rect.y >= inner.y);
                assert!(rect.right() <= inner.right() + 0.01);
                assert!(rect.bottom() <= inner.bottom() + 0.01);
                for other in &rects[index + 1..] {
                    assert!(!rect.overlaps(*other), "n={n} has no overlap");
                }
            }
        }
        assert_eq!(tile(1, CANVAS, 0.), vec![inner]);
        let two = tile(2, CANVAS, GUTTER);
        assert!(close(two[0].x, GUTTER), "the gutter is around");
        assert!(close(two[0].right() + GUTTER, two[1].x), "and between");
        assert!(tile(0, CANVAS, 0.).is_empty());
    }

    #[test]
    fn cascade_steps_each_window_and_stays_inside() {
        let inner = CANVAS.inset(GUTTER);
        let rects = cascade(4, CANVAS, 28.);
        assert_eq!(rects.len(), 4);
        assert!(close(rects[0].x, inner.x) && close(rects[0].y, inner.y));
        for pair in rects.windows(2) {
            assert!(close(pair[1].x - pair[0].x, 28.));
            assert!(close(pair[1].y - pair[0].y, 28.));
            assert!(close(pair[0].w, pair[1].w) && close(pair[0].h, pair[1].h));
        }
        let last = rects[3];
        assert!(last.right() <= inner.right() + 0.01);
        assert!(last.bottom() <= inner.bottom() + 0.01);
        // A crowd wraps instead of marching off the canvas.
        for rect in cascade(30, CANVAS, 28.) {
            assert!(rect.right() <= inner.right() + 0.01);
            assert!(rect.bottom() <= inner.bottom() + 0.01);
        }
        assert!(cascade(0, CANVAS, 28.).is_empty());
    }

    #[test]
    fn rect_round_trips_through_json() {
        let rect = Rect::new(12.5, 30., 400., 300.);
        let text = serde_json::to_string(&rect).unwrap_or_default();
        assert_eq!(serde_json::from_str::<Rect>(&text).ok(), Some(rect));
    }
}
