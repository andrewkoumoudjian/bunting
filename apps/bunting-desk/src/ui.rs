//! Shared pane components (ADR 0051 decision 7, slice U2b): the pane
//! body, toolbar, form field, stat, empty state, section label and table
//! helpers every pane is built from, styled with theme tokens only.
//!
//! The sizes below are the visual spec's scale in one place. U2a's
//! `metrics(cx)` replaces these constants at merge; pane code must not
//! repeat the literals.

use crate::theme;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Selectable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, Div, FocusHandle, FontWeight, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, Styled as _, TestSupportExt as _, Window, div, px,
};

/// Inner padding of every pane body.
pub const PANE_PADDING: f32 = 8.;
/// Vertical gap between a pane body's sections.
pub const PANE_GAP: f32 = 8.;
/// Height of a pane toolbar row.
pub const TOOLBAR_HEIGHT: f32 = 28.;
/// Gap between controls on one toolbar.
pub const TOOLBAR_GAP: f32 = 6.;
/// Height of a venue card's header row in the Book Trader.
pub const CARD_HEADER_HEIGHT: f32 = 28.;
/// Height of a table header row.
pub const TABLE_HEADER_HEIGHT: f32 = 24.;
/// Height of one table or book row.
pub const TABLE_ROW_HEIGHT: f32 = 24.;
/// Horizontal padding inside one table cell.
pub const CELL_PADDING: f32 = 8.;
/// Label text: field labels, stat labels, section labels, table heads.
pub const LABEL_SIZE: f32 = 11.;
/// Body and stat value text.
pub const VALUE_SIZE: f32 = 13.;
/// Gap between a field's label and its control.
pub const FIELD_GAP: f32 = 4.;
/// Icon size in an empty state.
pub const EMPTY_ICON_SIZE: f32 = 16.;

/// A pane's body: 8 px padding, an 8 px section gap, tabular figures.
/// The canvas's `WindowFrame` draws the title; panes never draw their own.
#[must_use]
pub fn pane_body(
    id: &'static str,
    focus: &FocusHandle,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    v_flex()
        .id(id)
        .test_support()
        .track_focus(focus)
        .size_full()
        .p(px(PANE_PADDING))
        .gap(px(PANE_GAP))
        .text_size(px(VALUE_SIZE))
        .font_features(theme::tabular())
}

/// One 28 px row of xsmall buttons, toggles and segmented controls,
/// vertically centred on one line.
#[must_use]
pub fn toolbar() -> Div {
    h_flex()
        .h(px(TOOLBAR_HEIGHT))
        .items_center()
        .gap(px(TOOLBAR_GAP))
}

/// A label above its control: an 11 px muted label, a 4 px gap, then the
/// control, aligned in a column with its sibling fields.
#[must_use]
pub fn field(label: impl Into<SharedString>, control: AnyElement, cx: &App) -> Div {
    v_flex()
        .gap(px(FIELD_GAP))
        .child(
            div()
                .text_size(px(LABEL_SIZE))
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(control)
}

/// An 11 px muted label above a 13 px tabular value.
#[must_use]
pub fn stat(label: impl Into<SharedString>, value: impl Into<SharedString>, cx: &App) -> Div {
    stat_colored(label, value, cx.theme().foreground, cx)
}

/// [`stat`] with the value in `color` (P&L by sign, a side's open size).
#[must_use]
pub fn stat_colored(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    color: Hsla,
    cx: &App,
) -> Div {
    v_flex()
        .gap(px(FIELD_GAP))
        .child(
            div()
                .text_size(px(LABEL_SIZE))
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .text_size(px(VALUE_SIZE))
                .text_color(color)
                .child(value.into()),
        )
}

/// An 11 px semibold muted section label.
#[must_use]
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(LABEL_SIZE))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// Muted secondary text at the label size.
#[must_use]
pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(LABEL_SIZE))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// An empty pane's whole story: a centred 16 px muted icon, one 13 px
/// muted line, and the action that fills it where one exists. Never a
/// large blank area.
#[must_use]
pub fn empty_state(
    id: &'static str,
    icon: IconName,
    text: impl Into<SharedString>,
    action: Option<Button>,
    cx: &App,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    let muted = cx.theme().muted_foreground;
    let mut state = v_flex()
        .id(id)
        .test_support()
        .flex_1()
        .w_full()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            div()
                .text_color(muted)
                .child(Icon::from(icon).size(px(EMPTY_ICON_SIZE))),
        )
        .child(
            div()
                .text_size(px(VALUE_SIZE))
                .text_color(muted)
                .child(text.into()),
        );
    if let Some(action) = action {
        state = state.child(action);
    }
    state
}

/// A small round status dot (a feed live or stale), coloured by token.
#[must_use]
pub fn status_dot(color: Hsla) -> Div {
    div().size(px(8.)).rounded_full().bg(color).flex_shrink_0()
}

/// One segment of a `ButtonGroup` segmented control: the group's size
/// applies, the selected segment takes the primary treatment, and
/// segments share the group's width equally.
#[must_use]
pub fn segment(
    id: impl Into<gpui_kit::ElementId>,
    label: impl Into<SharedString>,
    selected: bool,
) -> Button {
    let button = Button::new(id).label(label).selected(selected).flex_1();
    if selected { button.primary() } else { button }
}

// ---- tables ------------------------------------------------------------------

/// How a column's header and cells line up. Numbers are right-aligned in
/// both; prose and identifiers are left-aligned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// One table column: its header label, least width, share of any extra
/// width, keeping priority and alignment.
///
/// Columns are laid out for the width actually available ([`fit`]): each
/// starts at its `min_width` (wide enough for its header, never clipped
/// mid-word), extra space is shared by `weight`, and when even the
/// minimums do not fit, the lowest-`priority` columns are dropped whole
/// (ties: the rightmost first) until the rest fit. A dropped column
/// disappears from header and rows together.
#[derive(Clone, Copy, Debug)]
pub struct Col {
    pub label: &'static str,
    pub min_width: f32,
    pub weight: f32,
    /// 0 drops first, 1 is ordinary, 2 is kept longest.
    pub priority: u8,
    pub align: Align,
}

impl Col {
    const fn new(label: &'static str, min_width: f32, weight: f32, align: Align) -> Self {
        Self {
            label,
            min_width,
            weight,
            priority: 1,
            align,
        }
    }

    /// Prose and identifiers: left-aligned, and they take the extra width.
    #[must_use]
    pub const fn left(label: &'static str, min_width: f32) -> Self {
        Self::new(label, min_width, 1., Align::Left)
    }

    /// Numbers: right-aligned in header and cells, held at their minimum.
    #[must_use]
    pub const fn right(label: &'static str, min_width: f32) -> Self {
        Self::new(label, min_width, 0., Align::Right)
    }

    #[must_use]
    pub const fn center(label: &'static str, min_width: f32) -> Self {
        Self::new(label, min_width, 0., Align::Center)
    }

    /// This column's share of any width beyond the minimums.
    #[must_use]
    pub const fn weight(mut self, weight: f32) -> Self {
        self.weight = weight;
        self
    }

    /// How long this column is kept as the pane narrows (0 first to go).
    #[must_use]
    pub const fn priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }
}

/// One column that fits: which column it is (its index in the full set,
/// so rows can pick their cells) and the width it gets.
#[derive(Clone, Copy, Debug)]
pub struct Fit {
    pub index: usize,
    pub col: Col,
    pub width: f32,
}

/// Lays `cols` out in `available` px: drop the lowest-priority columns
/// (rightmost on a tie) until the minimums fit, then share what is left
/// by weight. A width of 0 (nothing measured yet) keeps every column at
/// its minimum. At least one column always remains.
#[must_use]
#[allow(
    clippy::float_arithmetic,
    reason = "column layout is f32 logical pixels throughout"
)]
pub fn fit(cols: &[Col], available: f32) -> Vec<Fit> {
    let mut kept: Vec<usize> = (0..cols.len()).collect();
    if available > 0. {
        while kept.len() > 1
            && kept.iter().map(|&index| cols[index].min_width).sum::<f32>() > available
        {
            let lowest = kept
                .iter()
                .map(|&index| cols[index].priority)
                .min()
                .unwrap_or(0);
            if let Some(position) = kept
                .iter()
                .rposition(|&index| cols[index].priority == lowest)
            {
                kept.remove(position);
            }
        }
    }
    let minimum: f32 = kept.iter().map(|&index| cols[index].min_width).sum();
    let extra = (available - minimum).max(0.);
    let weights: f32 = kept.iter().map(|&index| cols[index].weight).sum();
    kept.into_iter()
        .map(|index| {
            let col = cols[index];
            let share = if weights > 0. {
                extra * col.weight / weights
            } else {
                0.
            };
            Fit {
                index,
                col,
                width: col.min_width + share,
            }
        })
        .collect()
}

/// Every column at its minimum, for layouts that guarantee that much
/// room themselves (a book side inside its minimum-width card).
#[must_use]
pub fn fit_fixed(cols: &[Col]) -> Vec<Fit> {
    fit(cols, 0.)
}

/// Measures the width on offer and builds a table for it: `build` gets
/// the fitted columns, so header and rows always agree. The region
/// fills its parent, as a table area should.
pub fn responsive_table<E>(
    cols: &'static [Col],
    build: impl FnOnce(&[Fit], &mut Window, &mut App) -> E + 'static,
) -> gpui_kit::ContainerQuery
where
    E: IntoElement,
{
    gpui_kit::container_query(move |size, window, cx| {
        let fitted = fit(cols, f32::from(size.width));
        build(&fitted, window, cx)
    })
    .w_full()
    .h_auto()
    .flex_1()
    .min_h_0()
}

/// An invisible probe that records the width on offer into `width` and
/// redraws `view` when it changes. For tables inside scrolling content,
/// where a container query cannot size the content.
#[allow(
    clippy::float_arithmetic,
    reason = "comparing measured f32 logical pixels"
)]
pub fn width_probe<V: 'static>(
    view: gpui_kit::WeakEntity<V>,
    width: std::rc::Rc<std::cell::Cell<f32>>,
) -> gpui_kit::Canvas<()> {
    gpui_kit::canvas(
        move |bounds, _, cx| {
            let measured = f32::from(bounds.size.width);
            if (width.get() - measured).abs() > 0.5 {
                width.set(measured);
                view.update(cx, |_, cx| cx.notify()).ok();
            }
        },
        |_, (), _, _| {},
    )
    .w_full()
    .h(px(0.))
}

fn aligned_cell(width: f32, align: Align, content: AnyElement) -> Div {
    let cell = h_flex()
        .w(px(width))
        .flex_shrink_0()
        .px(px(CELL_PADDING))
        .items_center()
        .overflow_hidden();
    let cell = match align {
        Align::Left => cell,
        Align::Right => cell.justify_end(),
        Align::Center => cell.justify_center(),
    };
    cell.child(content)
}

/// A table's header: 24 px, 11 px semibold labels in the table-head
/// foreground, each aligned with its column. Rendered outside the
/// scrolling list, so it stays put while rows scroll. Takes the fitted
/// columns, so a dropped column has no header either.
#[must_use]
pub fn table_head(fitted: &[Fit], cx: &App) -> Div {
    let color = cx.theme().table_head_foreground;
    let mut head = h_flex().w_full().h(px(TABLE_HEADER_HEIGHT)).items_center();
    for fit in fitted {
        head = head.child(aligned_cell(
            fit.width,
            fit.col.align,
            div()
                .text_size(px(LABEL_SIZE))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color)
                .child(fit.col.label)
                .into_any_element(),
        ));
    }
    head
}

/// One 24 px table row. `cells` holds one element per column of the
/// full set, in column order; only the fitted ones are shown.
#[must_use]
pub fn table_row(fitted: &[Fit], cells: Vec<AnyElement>) -> Div {
    let mut cells: Vec<Option<AnyElement>> = cells.into_iter().map(Some).collect();
    let mut row = h_flex().w_full().h(px(TABLE_ROW_HEIGHT)).items_center();
    for fit in fitted {
        if let Some(cell) = cells.get_mut(fit.index).and_then(Option::take) {
            row = row.child(aligned_cell(fit.width, fit.col.align, cell));
        }
    }
    row
}

/// A plain text cell.
#[must_use]
pub fn cell(text: impl Into<SharedString>) -> AnyElement {
    div().child(text.into()).into_any_element()
}

/// A text cell in `color` (a price coloured by side, a warning).
#[must_use]
pub fn colored(text: impl Into<SharedString>, color: Hsla) -> AnyElement {
    div()
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

#[cfg(test)]
#[allow(
    clippy::float_arithmetic,
    reason = "asserting on f32 logical-pixel widths"
)]
mod tests {
    use super::*;

    const COLS: [Col; 4] = [
        Col::left("symbol", 64.).priority(2),
        Col::right("qty", 56.).priority(2),
        Col::right("price", 72.),
        Col::left("queue", 110.).priority(0),
    ];

    fn indices(fitted: &[Fit]) -> Vec<usize> {
        fitted.iter().map(|fit| fit.index).collect()
    }

    #[test]
    fn unmeasured_width_keeps_every_column_at_its_minimum() {
        let fitted = fit(&COLS, 0.);
        assert_eq!(indices(&fitted), [0, 1, 2, 3]);
        assert!(
            fitted
                .iter()
                .all(|f| (f.width - f.col.min_width).abs() < 0.01)
        );
    }

    #[test]
    fn extra_width_goes_to_weighted_columns_only() {
        // Minimums sum to 302; 398 px more, shared by the two left
        // columns (weight 1 each): +199 each.
        let fitted = fit(&COLS, 700.);
        assert_eq!(indices(&fitted), [0, 1, 2, 3]);
        assert!((fitted[0].width - 263.).abs() < 0.01, "{}", fitted[0].width);
        assert!((fitted[1].width - 56.).abs() < 0.01);
        assert!((fitted[2].width - 72.).abs() < 0.01);
        assert!((fitted[3].width - 309.).abs() < 0.01);
    }

    #[test]
    fn lowest_priority_columns_drop_whole_until_the_rest_fit() {
        // 150 px: queue (priority 0) goes first, then price (1);
        // symbol and qty (2) stay, never below their minimums.
        let fitted = fit(&COLS, 150.);
        assert_eq!(indices(&fitted), [0, 1]);
        assert!(fitted.iter().map(|f| f.width).sum::<f32>() <= 150.01);
        assert!(fitted.iter().all(|f| f.width >= f.col.min_width - 0.01));
    }

    #[test]
    fn ties_drop_the_rightmost_first_and_one_column_always_remains() {
        let fitted = fit(&COLS, 260.);
        assert_eq!(indices(&fitted), [0, 1, 2], "only queue drops at 260");
        let fitted = fit(&COLS, 10.);
        assert_eq!(fitted.len(), 1);
        assert_eq!(fitted[0].index, 0, "the leftmost essential remains");
    }
}
