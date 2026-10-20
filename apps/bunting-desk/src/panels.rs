//! The workspace's panels. Each one renders part of the shared [`Desk`]
//! and redraws when it changes; actions go straight to the desk, which
//! writes them to the venue at once.

use crate::desk::Desk;
use crate::flash::{self, Flash, Flashes};
use crate::format;
use crate::one_click::{self, Gesture, Intent, OneClick};
use crate::ui::{self, Col};
use bunting_trading_state::bunting_client::QueuedOrder;
use bunting_trading_state::bunting_client::bnp_wire::{Level, Listing, Side};
use bunting_trading_state::{
    Connection, FeedState, Instrument, NoticeLevel, OrderStatus, OrderTicket, TicketKind,
    TicketTif, TradePrint,
};
use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants as _};
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, ClickEvent, Context, Div, Entity, EventEmitter,
    FocusHandle, Focusable, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    WeakEntity, Window, div, px, uniform_list,
};
use std::collections::VecDeque;
use std::fmt::Write as _;
use std::time::Instant;

/// Book levels shown per side.
const BOOK_DEPTH: usize = 12;
/// Displayed orders shown per side in the order-by-order view.
const ORDER_DEPTH: usize = 16;
/// Venues shown side by side.
const MAX_VENUE_COLUMNS: usize = 6;
/// One side of an order-by-order book. Numbers right-align with their
/// headers; the broker is an identifier and stays left. As a card
/// narrows the broker drops first, then the quantity ahead (both
/// priority 0, the rightmost first), before anything clips.
pub const ORDER_COLUMNS: [Col; 5] = [
    Col::right("place", 44.),
    Col::right("ahead", 52.).priority(0),
    Col::right("qty", 52.).priority(2),
    Col::right("price", 60.).priority(2),
    Col::left("broker", 52.).priority(0),
];
/// One side of a price-level book.
pub const LEVEL_COLUMNS: [Col; 3] = [
    Col::center("mine", 48.).priority(0),
    Col::right("qty", 60.).priority(2),
    Col::right("price", 68.).priority(2),
];
/// A venue card's least width: both sides' essential columns (place,
/// qty, price / qty, price), the gap between them, padding and border.
/// Wider cards fit more columns back in; the books scroll sideways
/// below this.
const ORDER_CARD_WIDTH: f32 = 400.;
const LEVEL_CARD_WIDTH: f32 = 300.;

/// The boilerplate every dock panel shares.
macro_rules! dock_panel {
    ($panel:ident, $name:literal, $title:literal) => {
        dock_panel!($panel, $name, $title, false);
    };
    ($panel:ident, $name:literal, $title:literal, $follows_symbol:literal) => {
        impl BasePanel for $panel {
            fn panel_name(&self) -> &'static str {
                $name
            }
        }

        impl Panel for $panel {
            fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let symbol = if $follows_symbol {
                    self.desk
                        .read(cx)
                        .instrument()
                        .map(|instrument| instrument.symbol)
                } else {
                    None
                };
                $crate::theme::pane_title($title, symbol.as_deref())
            }
        }

        impl EventEmitter<PanelEvent> for $panel {}

        impl Focusable for $panel {
            fn focus_handle(&self, _: &App) -> FocusHandle {
                self.focus.clone()
            }
        }
    };
}

pub(crate) use dock_panel;

/// A panel that only reads the desk.
macro_rules! reader_panel {
    ($panel:ident) => {
        pub struct $panel {
            desk: Entity<Desk>,
            focus: FocusHandle,
            _observe: Subscription,
        }

        impl $panel {
            pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
                Self {
                    _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
                    desk,
                    focus: cx.focus_handle(),
                }
            }
        }
    };
}

pub(crate) use ui::{cell, colored, muted};

pub(crate) fn panel_body(
    id: &'static str,
    focus: &FocusHandle,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    ui::pane_body(id, focus)
}

pub(crate) fn side_color(side: Side, cx: &App) -> Hsla {
    match side {
        Side::Buy => cx.theme().chart_bullish,
        Side::Sell => cx.theme().chart_bearish,
    }
}

/// A table header at the columns' minimum widths, for layouts that
/// guarantee that much room (a book side in its minimum-width card, the
/// tape's three quote columns in its minimum-width pane). Pane-width
/// tables use [`ui::responsive_table`] instead.
pub(crate) fn aligned_header(cols: &[Col], cx: &App) -> Div {
    ui::table_head(&ui::fit_fixed(cols), cx)
}

/// One table row at the columns' minimum widths; see [`aligned_header`].
pub(crate) fn aligned_row(cols: &[Col], cells: Vec<AnyElement>, cx: &App) -> Div {
    ui::table_row(&ui::fit_fixed(cols), cells, cx)
}

// ---- books -----------------------------------------------------------------

/// How the books show depth.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Depth {
    /// Quantity per price.
    Levels,
    /// Every displayed order with its place in the queue, where the venue
    /// sends order-by-order data.
    Orders,
}

pub struct BooksPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    depth: Depth,
    /// RIT's lightning toggle: off until the trader turns it on.
    one_click: bool,
    volume: Entity<InputState>,
    offset: Entity<InputState>,
    flashes: Flashes,
    redraw_pending: bool,
    /// Why the last one-click order or cancel did not go.
    problem: Option<String>,
    /// Each venue card's content width last measured by its width
    /// probe, so each book side fits its columns to the card.
    card_widths: std::rc::Rc<
        std::cell::RefCell<std::collections::HashMap<Listing, std::rc::Rc<std::cell::Cell<f32>>>>,
    >,
    _observe: Subscription,
}

dock_panel!(BooksPanel, "books", "Book Trader", true);

impl BooksPanel {
    pub fn new(desk: Entity<Desk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let defaults = OneClick::default();
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            depth: Depth::Orders,
            one_click: defaults.enabled,
            volume: cx
                .new(|cx| InputState::new(window, cx).default_value(defaults.volume.to_string())),
            offset: cx
                .new(|cx| InputState::new(window, cx).default_value(defaults.offset.to_string())),
            flashes: Flashes::default(),
            redraw_pending: false,
            problem: None,
            card_widths: std::rc::Rc::new(
                std::cell::RefCell::new(std::collections::HashMap::new()),
            ),
        }
    }

    fn depth_segment(&self, depth: Depth) -> Button {
        let (id, label, tip) = match depth {
            Depth::Levels => (
                "books-levels",
                "Levels",
                "Quantity per price. Click a price to put it on the ticket; alt+click cancels yours there. ● marks your resting quantity",
            ),
            Depth::Orders => (
                "books-orders",
                "Orders",
                "Each displayed order in queue order: its place at its price, the quantity ahead of it and its broker. Yours are highlighted. Click an order to put its price on the ticket; alt+click cancels yours",
            ),
        };
        ui::segment(id, label, self.depth == depth).tooltip(tip)
    }

    /// The one-click settings as entered; a field that does not parse
    /// counts as 0, which sends nothing.
    fn settings(&self, cx: &App) -> OneClick {
        let number = |state: &Entity<InputState>| {
            state
                .read(cx)
                .value()
                .trim()
                .parse::<i64>()
                .unwrap_or(0)
                .clamp(0, one_click::MAX_STEP)
        };
        OneClick {
            enabled: self.one_click,
            volume: number(&self.volume),
            offset: number(&self.offset),
        }
    }

    /// A click on a book row: an order, a cancel or a price for the ticket.
    fn clicked(&mut self, row: one_click::Row, gesture: Gesture, cx: &mut Context<Self>) {
        let Some(intent) = one_click::intent(self.settings(cx), row, gesture) else {
            return;
        };
        self.problem = None;
        match intent {
            Intent::Order(ticket) => {
                let result = self.desk.update(cx, |desk, cx| desk.place(&ticket, cx));
                self.problem = result.err().map(|error| error.to_string());
            }
            Intent::Cancel {
                listing,
                side,
                price,
            } => {
                let asked = self
                    .desk
                    .update(cx, |desk, cx| desk.cancel_at(listing, side, price, cx));
                if asked == 0 {
                    self.problem = Some(format!(
                        "No live {} of yours at {price} on {}.",
                        match side {
                            Side::Buy => "bids",
                            Side::Sell => "asks",
                        },
                        format::venue(listing)
                    ));
                }
            }
            Intent::Pick(price) => {
                self.desk
                    .update(cx, |desk, cx| desk.pick_price(row.listing, price, cx));
            }
        }
        cx.notify();
    }

    /// Lights what changed in each shown book and redraws when the lights
    /// go out.
    fn observe_books(&mut self, listings: &[Listing], cx: &mut Context<Self>) {
        let now = Instant::now();
        let desk = self.desk.read(cx);
        for listing in listings {
            if let Some(feed) = desk.model().feed(*listing) {
                self.flashes.observe(*listing, &feed.book, now);
            }
        }
        if self.redraw_pending {
            return;
        }
        if let Some(wait) = self.flashes.next_change(now) {
            self.redraw_pending = true;
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |this, cx| {
                    this.redraw_pending = false;
                    cx.notify();
                })
                .ok();
            })
            .detach();
        }
    }

    /// The measured-width cell for one venue card, kept across frames.
    /// Bounded like the flashes: past 64 cards the memory starts again.
    fn card_width_cell(&self, listing: Listing) -> std::rc::Rc<std::cell::Cell<f32>> {
        let mut widths = self.card_widths.borrow_mut();
        if widths.len() >= 64 && !widths.contains_key(&listing) {
            widths.clear();
        }
        widths.entry(listing).or_default().clone()
    }

    fn one_click_bar(&self, cx: &mut Context<Self>) -> Div {
        let mut bar = h_flex()
            .gap(px(ui::TOOLBAR_GAP))
            .items_center()
            .child(
                Button::new("books-one-click")
                    .label("One-click")
                    .xsmall()
                    .tooltip(
                        "Left-click a bid to buy V at its price + O (an ask: sell at − O), right-click for market, shift+right-click to sweep, alt+click to cancel yours",
                    )
                    .when_toggled(self.one_click)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.one_click = !this.one_click;
                        cx.notify();
                    })),
            );
        if self.one_click {
            bar = bar
                .child(muted("V", cx))
                .child(
                    div()
                        .id("books-volume")
                        .test_support()
                        .w(ui::rem(ui::NUMBER_INPUT_MIN_WIDTH))
                        .child(NumberInput::new(&self.volume).xsmall()),
                )
                .child(muted("O", cx))
                .child(
                    div()
                        .id("books-offset")
                        .test_support()
                        .w(ui::rem(ui::NUMBER_INPUT_MIN_WIDTH))
                        .child(NumberInput::new(&self.offset).xsmall()),
                );
        }
        bar
    }
}

impl Render for BooksPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let instruments = desk.instruments();
        let instrument = desk.instrument();
        let chosen = desk.listing();
        let mut body = panel_body("books", &self.focus);
        if instruments.is_empty() {
            return body.child(ui::empty_state(
                "books-empty",
                IconName::BookOpen,
                "No listings yet",
                None,
                cx,
            ));
        }
        let mut tabs = h_flex().gap_1().items_center();
        for each in &instruments {
            let id = each.instrument_id;
            let selected = instrument
                .as_ref()
                .is_some_and(|shown| shown.instrument_id == id);
            tabs = tabs.child(
                Button::new(SharedString::from(format!("instrument-{id}")))
                    .label(each.symbol.clone())
                    .xsmall()
                    .tooltip("Each venue's book as its own feed reports it; books are never merged")
                    .when_toggled(selected)
                    .on_click(window.listener_for(&self.desk, move |desk, _, _, cx| {
                        desk.select_instrument(id, cx);
                    })),
            );
        }
        body = body.child(
            ui::toolbar(cx)
                .w_full()
                .child(tabs)
                .child(div().flex_1())
                .child(self.one_click_bar(cx))
                .child(
                    ButtonGroup::new("books-depth")
                        .xsmall()
                        .child(self.depth_segment(Depth::Orders))
                        .child(self.depth_segment(Depth::Levels))
                        .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                            this.depth = if clicks.contains(&0) {
                                Depth::Orders
                            } else {
                                Depth::Levels
                            };
                            cx.notify();
                        })),
                ),
        );
        let Some(instrument) = instrument else {
            return body;
        };
        if let Some(halted) = halted_notice(&self.desk, &instrument, cx) {
            body = body.child(halted);
        }
        let listings: Vec<Listing> = instrument
            .listings
            .iter()
            .take(MAX_VENUE_COLUMNS)
            .map(|info| info.listing)
            .collect();
        self.observe_books(&listings, cx);
        let mut columns = h_flex().w_full().gap_2().items_start();
        for listing in &listings {
            columns = columns.child(self.book_column(*listing, chosen, window, cx));
        }
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .id("books-problem")
                    .test_support()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        body.child(
            div()
                .id("books-scroll")
                .flex_1()
                .min_h_0()
                .overflow_scroll()
                .child(columns),
        )
    }
}

/// A line saying `instrument` is halted, while it is.
fn halted_notice(desk: &Entity<Desk>, instrument: &Instrument, cx: &App) -> Option<AnyElement> {
    desk.read(cx)
        .model()
        .is_halted(instrument.instrument_id)
        .then(|| {
            div()
                .id("books-halted")
                .test_support()
                .text_size(ui::rem(ui::LABEL_SIZE))
                .text_color(cx.theme().warning)
                .child(format!(
                    "{} is halted on every venue: orders wait for the reopening",
                    instrument.symbol
                ))
                .into_any_element()
        })
}

/// Displayed quantity on `side` from the best price through `price`.
fn through(levels: &[Level], side: Side, price: i64) -> i64 {
    levels
        .iter()
        .filter(|level| match side {
            Side::Buy => level.price >= price,
            Side::Sell => level.price <= price,
        })
        .map(|level| level.quantity)
        .sum()
}

/// A row's background: lit by a flash for 250 ms (bullish for new,
/// bearish for traded, both at low alpha), else the own-order highlight.
fn row_background(flash: Option<Flash>, own: bool, cx: &App) -> Option<Hsla> {
    let theme = cx.theme();
    match flash {
        Some(Flash::New) => Some(theme.chart_bullish.opacity(0.35)),
        Some(Flash::Traded) => Some(theme.chart_bearish.opacity(0.35)),
        None => own.then_some(theme.list_active),
    }
}

impl BooksPanel {
    #[expect(
        clippy::too_many_lines,
        reason = "one venue's column: header, status lines and both sides"
    )]
    #[allow(
        clippy::float_arithmetic,
        reason = "halving the measured f32 card width for the two sides"
    )]
    fn book_column(
        &self,
        listing: Listing,
        chosen: Option<Listing>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let now = desk.now_us();
        let feed = model.feed(listing);
        let theme = cx.theme();
        let (state, state_color) = match feed.map(|feed| &feed.state) {
            Some(FeedState::Live) => ("live".to_owned(), theme.success),
            Some(FeedState::Requested) => ("subscribing".to_owned(), theme.warning),
            Some(FeedState::Refused(reason)) => (format!("refused: {reason}"), theme.danger),
            None => ("no feed".to_owned(), theme.warning),
        };
        let age = feed
            .and_then(|feed| feed.last_message_us)
            .map(|at| format!("{} ago", format::duration(now.saturating_sub(at))));
        let gaps = feed.map_or(0, |feed| feed.gaps);
        let order_trip = model
            .venue_round_trips()
            .get(&listing.venue_id)
            .map(|trip| format!("order trip {}", format::duration(trip.last_us)));
        let own = |price: i64, side: Side| -> i64 {
            model
                .live_orders()
                .filter(|order| {
                    order.listing == listing && order.side == side && order.price == Some(price)
                })
                .map(|order| order.remaining)
                .sum()
        };
        let (all_asks, all_bids) = feed.map_or_else(
            || (Vec::new(), Vec::new()),
            |feed| (feed.book.asks(), feed.book.bids()),
        );
        let spread = match (all_bids.first(), all_asks.first()) {
            (Some(bid), Some(ask)) => Some(ask.price - bid.price),
            _ => None,
        };
        let selected = chosen == Some(listing);
        let venue_id = listing.venue_id;
        let border = if selected { theme.ring } else { theme.border };
        // Room for both sides, the gap between them and the padding, so a
        // narrow pane scrolls sideways instead of overlapping the sides.
        let card_width = match self.depth {
            Depth::Orders => ORDER_CARD_WIDTH,
            Depth::Levels => LEVEL_CARD_WIDTH,
        };
        // The card's 28 px header: venue, feed state as a dot and word,
        // the measured ages in muted tabular text, and the spread.
        let mut card_header = h_flex()
            .w_full()
            .h(ui::rem_px(
                crate::appearance::metrics(cx).title_strip_height,
            ))
            .items_center()
            .gap(px(ui::TOOLBAR_GAP))
            .child(
                div()
                    .flex_shrink_0()
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .child(format::venue(listing)),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(ui::status_dot(state_color))
                    .child(muted(state, cx)),
            )
            .child(div().flex_1());
        // The ages truncate first; the spread never does.
        if let Some(age) = age {
            card_header = card_header.child(muted(age, cx).min_w_0().truncate());
        }
        if let Some(trip) = order_trip {
            card_header = card_header.child(muted(trip, cx).min_w_0().truncate());
        }
        card_header = card_header.child(
            muted(
                spread.map_or_else(
                    || "one-sided".to_owned(),
                    |spread| format!("spread {spread}"),
                ),
                cx,
            )
            .flex_shrink_0(),
        );
        let mut column = v_flex()
            .id(SharedString::from(format!("venue-{venue_id}")))
            .test_support()
            .flex_1()
            .flex_shrink_0()
            .min_w(px(card_width))
            .max_w(px(640.))
            .p(px(ui::PANE_PADDING))
            .gap_1()
            .border_1()
            .border_color(border)
            .rounded_md()
            .cursor_pointer()
            .on_click(window.listener_for(&self.desk, move |desk, _, _, cx| {
                desk.select_listing(listing, cx);
            }))
            .child(ui::width_probe(
                cx.entity().downgrade(),
                self.card_width_cell(listing),
            ))
            .child(card_header);
        if gaps > 0 {
            column = column.child(
                div()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().warning)
                    .child(format!("{gaps} feed gaps, resubscribed")),
            );
        }
        let clock = Instant::now();
        // Each side fits its columns to half the card's measured content
        // width (minus the gap between the sides): lower-priority
        // columns drop whole instead of clipping. Unmeasured (0) keeps
        // every column at its minimum for the first frame.
        let gap = f32::from(ui::rem_size(cx)) * 0.75;
        let measured = self.card_width_cell(listing).get();
        let side_available = if measured > 0. {
            (measured - gap) / 2.
        } else {
            0.
        };
        let order_fit = ui::fit(&ORDER_COLUMNS, side_available);
        let level_fit = ui::fit(&LEVEL_COLUMNS, side_available);
        // Bids on the left and asks on the right, best price at the top of
        // each, as in RIT's Book Trader.
        let sides = |bids: Vec<AnyElement>, asks: Vec<AnyElement>, fitted: &[ui::Fit]| {
            let list = |side_name: &'static str, empty: &'static str, rows: Vec<AnyElement>| {
                let mut list = v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(ui::table_head(fitted, cx));
                if rows.is_empty() {
                    list = list.child(
                        div()
                            .id(SharedString::from(format!("no-{side_name}-{venue_id}")))
                            .test_support()
                            .w_full()
                            .py_2()
                            .text_center()
                            .text_size(ui::rem(ui::LABEL_SIZE))
                            .text_color(cx.theme().muted_foreground)
                            .child(empty),
                    );
                } else {
                    list = list.children(rows);
                }
                list
            };
            h_flex()
                .w_full()
                .gap_3()
                .items_start()
                .child(list("bids", "No bids", bids))
                .child(list("asks", "No asks", asks))
        };
        if self.depth == Depth::Orders {
            if let Some(feed) = feed.filter(|feed| feed.order_by_order) {
                let order_row = |order: &QueuedOrder, index: usize, levels: &[Level]| {
                    let row = one_click::Row {
                        listing,
                        side: order.side,
                        price: order.price,
                        through: through(levels, order.side, order.price),
                    };
                    let flash =
                        self.flashes
                            .get(listing, flash::Key::Order(order.reference), clock);
                    let mine = model.own_order(listing, order.reference).is_some();
                    Self::order_row(row, order, index, flash, mine, &order_fit, cx)
                };
                let asks = feed.book.orders(Side::Sell);
                let bids = feed.book.orders(Side::Buy);
                let hidden =
                    asks.len().saturating_sub(ORDER_DEPTH) + bids.len().saturating_sub(ORDER_DEPTH);
                column = column.child(sides(
                    bids.iter()
                        .take(ORDER_DEPTH)
                        .enumerate()
                        .map(|(index, order)| order_row(order, index, &all_bids))
                        .collect(),
                    asks.iter()
                        .take(ORDER_DEPTH)
                        .enumerate()
                        .map(|(index, order)| order_row(order, index, &all_asks))
                        .collect(),
                    &order_fit,
                ));
                if hidden > 0 {
                    column = column.child(muted(format!("{hidden} more orders deeper"), cx));
                }
                return column;
            }
            if feed.is_some_and(|feed| matches!(feed.state, FeedState::Live)) {
                column = column.child(muted("This venue sends price levels only.", cx));
            }
        }
        let level_row = |level: &Level, side: Side, index: usize, levels: &[Level]| {
            let row = one_click::Row {
                listing,
                side,
                price: level.price,
                through: through(levels, side, level.price),
            };
            let flash = self
                .flashes
                .get(listing, flash::Key::Level(side, level.price), clock);
            Self::level_row(
                row,
                *level,
                own(level.price, side),
                index,
                flash,
                &level_fit,
                cx,
            )
        };
        column.child(sides(
            all_bids
                .iter()
                .take(BOOK_DEPTH)
                .enumerate()
                .map(|(index, level)| level_row(level, Side::Buy, index, &all_bids))
                .collect(),
            all_asks
                .iter()
                .take(BOOK_DEPTH)
                .enumerate()
                .map(|(index, level)| level_row(level, Side::Sell, index, &all_asks))
                .collect(),
            &level_fit,
        ))
    }

    /// A book row that answers clicks: one-click orders, alt+click cancels,
    /// or a price for the ticket. Your own rows carry the highlight's
    /// left edge as well as its background.
    fn clickable(
        id: String,
        row: one_click::Row,
        background: Option<Hsla>,
        own: bool,
        cx: &Context<Self>,
    ) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
        let mut element = div()
            .id(SharedString::from(id))
            .test_support()
            .cursor_pointer()
            .rounded_sm()
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                let modifiers = event.modifiers();
                cx.stop_propagation();
                this.clicked(
                    row,
                    Gesture::from_click(false, modifiers.shift, modifiers.alt),
                    cx,
                );
            }))
            .on_aux_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.is_right_click() {
                    let modifiers = event.modifiers();
                    cx.stop_propagation();
                    this.clicked(
                        row,
                        Gesture::from_click(true, modifiers.shift, modifiers.alt),
                        cx,
                    );
                }
            }));
        if let Some(background) = background {
            element = element.bg(background);
        }
        if own {
            element = element
                .border_l_1()
                .border_color(cx.theme().list_active_border);
        }
        element
    }

    /// One displayed order: its place at its price, the displayed quantity
    /// ahead of it, its size, price and broker. Yours are highlighted.
    fn order_row(
        row: one_click::Row,
        order: &QueuedOrder,
        index: usize,
        flash: Option<Flash>,
        mine: bool,
        fitted: &[ui::Fit],
        cx: &Context<Self>,
    ) -> AnyElement {
        let side_name = match order.side {
            Side::Buy => "bid",
            Side::Sell => "ask",
        };
        let color = side_color(order.side, cx);
        let broker = if mine {
            "you".to_owned()
        } else {
            order
                .broker
                .map(|broker| broker.to_string())
                .unwrap_or_default()
        };
        Self::clickable(
            format!("{side_name}-order-{}-{index}", row.listing.venue_id),
            row,
            row_background(flash, mine, cx),
            mine,
            cx,
        )
        .child(ui::table_row(
            fitted,
            vec![
                cell(format!("#{}", order.position_at_price)),
                cell(order.quantity_ahead.to_string()),
                cell(order.quantity.to_string()),
                colored(order.price.to_string(), color),
                cell(broker),
            ],
            cx,
        ))
        .into_any_element()
    }

    fn level_row(
        row: one_click::Row,
        level: Level,
        mine: i64,
        index: usize,
        flash: Option<Flash>,
        fitted: &[ui::Fit],
        cx: &Context<Self>,
    ) -> AnyElement {
        let mark = if mine > 0 {
            format!("● {mine}")
        } else {
            String::new()
        };
        let color = side_color(row.side, cx);
        let side_name = match row.side {
            Side::Buy => "bid",
            Side::Sell => "ask",
        };
        Self::clickable(
            format!("{side_name}-{}-{index}", row.listing.venue_id),
            row,
            row_background(flash, mine > 0, cx),
            mine > 0,
            cx,
        )
        .child(ui::table_row(
            fitted,
            vec![
                cell(mark),
                cell(level.quantity.to_string()),
                colored(level.price.to_string(), color),
            ],
            cx,
        ))
        .into_any_element()
    }
}

// ---- order ticket ------------------------------------------------------------

pub struct TicketPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    quantity: Entity<InputState>,
    price: Entity<InputState>,
    /// Quantity shown at a time (an iceberg); blank shows it all.
    display: Entity<InputState>,
    side: Side,
    kind: TicketKind,
    tif: TicketTif,
    post_only: bool,
    anonymous: bool,
    seen_hint: u64,
    /// Why the last order was not sent.
    problem: Option<String>,
    /// The pane content width last measured by the body's width probe,
    /// so paired fields can stack when the pane is narrow.
    width: std::rc::Rc<std::cell::Cell<f32>>,
    _observe: Subscription,
}

dock_panel!(TicketPanel, "ticket", "Order Entry", true);

impl TicketPanel {
    pub fn new(desk: Entity<Desk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe_in(&desk, window, |this, desk, window, cx| {
            let (number, price) = desk.read(cx).price_hint();
            if number != this.seen_hint {
                this.seen_hint = number;
                if let Some(price) = price {
                    this.kind = TicketKind::Limit;
                    this.price.update(cx, |state, cx| {
                        state.set_value(price.to_string(), window, cx);
                    });
                }
            }
            cx.notify();
        });
        let seen_hint = desk.read(cx).price_hint().0;
        Self {
            quantity: cx.new(|cx| InputState::new(window, cx).default_value("1")),
            price: cx.new(|cx| InputState::new(window, cx)),
            display: cx.new(|cx| InputState::new(window, cx).placeholder("all")),
            desk,
            focus: cx.focus_handle(),
            side: Side::Buy,
            kind: TicketKind::Limit,
            tif: TicketTif::Gtc,
            post_only: false,
            anonymous: false,
            seen_hint,
            problem: None,
            width: std::rc::Rc::new(std::cell::Cell::new(0.)),
            _observe: observation,
        }
    }

    /// The ticket as entered; numbers that do not parse become 0, which the
    /// ticket refuses with its own reason.
    fn ticket(&self, cx: &App) -> OrderTicket {
        let number =
            |state: &Entity<InputState>| state.read(cx).value().trim().parse::<i64>().unwrap_or(0);
        OrderTicket {
            listing: self.desk.read(cx).listing(),
            side: self.side,
            quantity: number(&self.quantity),
            kind: self.kind,
            price: number(&self.price),
            tif: self.tif,
            post_only: self.post_only,
            anonymous: self.anonymous,
            display_quantity: {
                let text = self.display.read(cx).value();
                let text = text.trim();
                // Blank shows the whole order; anything else the ticket checks.
                (!text.is_empty()).then(|| text.parse::<i64>().unwrap_or(0))
            },
        }
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let ticket = self.ticket(cx);
        let result = self.desk.update(cx, |desk, cx| desk.place(&ticket, cx));
        self.problem = result.err().map(|error| error.to_string());
        cx.notify();
    }

    /// The submit button's label: the action it sends, venue included, or
    /// what is missing before anything can be sent.
    fn send_label(&self, symbol: &str, chosen: Option<Listing>, cx: &App) -> String {
        let Some(listing) = chosen else {
            return "Choose a venue".to_owned();
        };
        let ticket = self.ticket(cx);
        let side = format::side(self.side);
        let venue = format::venue(listing);
        match self.kind {
            TicketKind::Market => {
                format!("{side} {} {symbol} at market on {venue}", ticket.quantity)
            }
            TicketKind::Limit if self.price.read(cx).value().trim().is_empty() => {
                format!("{side} {} {symbol} on {venue}", ticket.quantity)
            }
            TicketKind::Limit => {
                format!(
                    "{side} {} {symbol} @ {} on {venue}",
                    ticket.quantity, ticket.price
                )
            }
        }
    }
}

impl Render for TicketPanel {
    #[expect(
        clippy::too_many_lines,
        reason = "the whole ticket form, field by field"
    )]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let instrument = desk.instrument();
        let chosen = desk.listing();
        let connected = desk.model().is_connected();
        let killed = desk.model().kill_switch_active();
        let trips = desk.model().venue_round_trips().clone();
        let mut body = panel_body("ticket", &self.focus);
        let Some(instrument) = instrument else {
            return body.child(ui::empty_state(
                "ticket-empty",
                IconName::SquareTerminal,
                "No listings yet",
                None,
                cx,
            ));
        };
        // The fields scroll; the footer below them never does, so the
        // submit button and the kill switch stay reachable however
        // short the window is. The probe measures the content width so
        // paired fields can stack in a narrow pane (360 px: the probe
        // sits inside the pane padding, hence 344).
        let wide = self.width.get() >= 344.;
        let mut fields = v_flex()
            .id("ticket-fields")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(ui::PANE_GAP));
        // The venue: no default, the participant chooses (ADR 0050).
        let mut venues = h_flex().gap_1().flex_wrap().items_center();
        for info in &instrument.listings {
            let listing = info.listing;
            let label = trips.get(&listing.venue_id).map_or_else(
                || format::venue(listing),
                |trip| {
                    format!(
                        "{} · {}",
                        format::venue(listing),
                        format::duration(trip.last_us)
                    )
                },
            );
            venues = venues.child(
                Button::new(SharedString::from(format!(
                    "ticket-venue-{}",
                    listing.venue_id
                )))
                .label(label)
                .small()
                .tooltip("Every order names its venue; there is no default router")
                .when_toggled(chosen == Some(listing))
                .on_click(window.listener_for(
                    &self.desk,
                    move |desk, _, _, cx| {
                        desk.select_listing(listing, cx);
                    },
                )),
            );
        }
        fields = fields.child(ui::field("Venue", venues.into_any_element(), cx));
        // Side and type are segmented controls. The selected side takes
        // its market colour: bullish for Buy, bearish for Sell.
        let side_segment = |id: &'static str, label: &'static str, selected: bool, buy: bool| {
            let button = Button::new(id).label(label).selected(selected).flex_1();
            match (selected, buy) {
                (true, true) => button.success(),
                (true, false) => button.danger(),
                (false, _) => button,
            }
        };
        fields = fields.child(
            h_flex()
                .gap_2()
                .items_start()
                .child(
                    ui::field(
                        "Side",
                        ButtonGroup::new("ticket-side")
                            .small()
                            .child(side_segment(
                                "ticket-buy",
                                "Buy",
                                self.side == Side::Buy,
                                true,
                            ))
                            .child(side_segment(
                                "ticket-sell",
                                "Sell",
                                self.side == Side::Sell,
                                false,
                            ))
                            .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                                this.side = if clicks.contains(&0) {
                                    Side::Buy
                                } else {
                                    Side::Sell
                                };
                                cx.notify();
                            }))
                            .into_any_element(),
                        cx,
                    )
                    .flex_1(),
                )
                .child(
                    ui::field(
                        "Type",
                        ButtonGroup::new("ticket-kind")
                            .small()
                            .child(ui::segment(
                                "ticket-limit",
                                "Limit",
                                self.kind == TicketKind::Limit,
                            ))
                            .child(ui::segment(
                                "ticket-market",
                                "Market",
                                self.kind == TicketKind::Market,
                            ))
                            .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                                this.kind = if clicks.contains(&0) {
                                    TicketKind::Limit
                                } else {
                                    TicketKind::Market
                                };
                                cx.notify();
                            }))
                            .into_any_element(),
                        cx,
                    )
                    .flex_1(),
                ),
        );
        // Quantity and price: two fields of equal width on one row.
        fields = fields.child(
            h_flex()
                .gap_2()
                .items_start()
                .child(
                    ui::field(
                        "Quantity (lots)",
                        NumberInput::new(&self.quantity).small().into_any_element(),
                        cx,
                    )
                    .flex_1(),
                )
                .child(
                    ui::field(
                        "Price (ticks)",
                        NumberInput::new(&self.price)
                            .small()
                            .disabled(self.kind == TicketKind::Market)
                            .into_any_element(),
                        cx,
                    )
                    .flex_1(),
                ),
        );
        fields = fields.child(ui::field_tipped(
            "ticket-display-label",
            "Display qty",
            "Iceberg: the quantity shown in the book at a time. Blank shows the whole order.",
            div()
                .id("ticket-display")
                .test_support()
                .child(
                    NumberInput::new(&self.display)
                        .small()
                        .disabled(self.kind == TicketKind::Market),
                )
                .into_any_element(),
            cx,
        ));
        let market = self.kind == TicketKind::Market;
        let mut tifs = ButtonGroup::new("ticket-tifs").small();
        for tif in TicketTif::ALL {
            tifs = tifs.child(
                ui::segment(
                    SharedString::from(format!("ticket-tif-{}", tif.label())),
                    tif.label(),
                    self.tif == tif,
                )
                .disabled(market),
            );
        }
        let tifs = tifs.on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
            if let Some(index) = clicks.first() {
                this.tif = TicketTif::ALL[*index];
            }
            cx.notify();
        }));
        let tif_field = ui::field("Time in force", tifs.into_any_element(), cx);
        let flags_field = ui::field(
            "Flags",
            ButtonGroup::new("ticket-flags")
                .small()
                .multiple(true)
                .child(ui::segment("ticket-post-only", "Post only", self.post_only))
                .child(ui::segment("ticket-anonymous", "Anonymous", self.anonymous))
                .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                    this.post_only = clicks.contains(&0);
                    this.anonymous = clicks.contains(&1);
                    cx.notify();
                }))
                .into_any_element(),
            cx,
        );
        // One row when the pane is wide enough for both, stacked else.
        fields = if wide {
            fields.child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(tif_field.flex_1())
                    .child(flags_field.flex_1()),
            )
        } else {
            fields.child(tif_field).child(flags_field)
        };
        // Full width, coloured by side, and labelled with the order it
        // sends; with no venue it says so and stays disabled.
        let send = Button::new("ticket-send")
            .label(self.send_label(&instrument.symbol, chosen, cx))
            .small()
            .w_full()
            .disabled(!connected || killed || chosen.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.send(cx)));
        let send = match self.side {
            Side::Buy => send.success(),
            Side::Sell => send.danger(),
        };
        // The sticky footer: outside the scrolling fields, always
        // visible. A trader must always be able to send and kill.
        let mut footer = v_flex()
            .gap_2()
            .pt_2()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(send);
        if let Some(problem) = &self.problem {
            footer = footer.child(
                div()
                    .id("ticket-problem")
                    .test_support()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        if !connected {
            footer = footer.child(muted("Not connected: orders cannot be sent", cx));
        }
        if killed {
            footer = footer.child(
                div()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().danger)
                    .child("Kill switch active: the venue refuses new orders until you reset it"),
            );
        }
        body = body
            .child(ui::width_probe(cx.entity().downgrade(), self.width.clone()))
            .child(fields);
        body.child(
            footer.child(
                h_flex()
                    .h(ui::rem_px(
                        crate::appearance::metrics(cx).title_strip_height,
                    ))
                    .items_center()
                    .gap(px(ui::TOOLBAR_GAP))
                    .child(
                        Button::new("cancel-all")
                            .label("Cancel all")
                            .small()
                            .outline()
                            .on_click(window.listener_for(&self.desk, |desk, _, _, cx| {
                                desk.cancel_all(cx);
                            })),
                    )
                    .child(if killed {
                        Button::new("kill-switch-reset")
                            .label("Reset kill switch")
                            .small()
                            .warning()
                            .on_click(window.listener_for(&self.desk, |desk, _, _, cx| {
                                desk.reset_kill_switch(cx);
                            }))
                    } else {
                        Button::new("kill-switch")
                            .label("Kill switch")
                            .small()
                            .danger()
                            .on_click(cx.listener(|this, _, window, cx| {
                                confirm_kill_switch(this.desk.clone(), window, cx);
                            }))
                    }),
            ),
        )
    }
}

/// Asks before activating the kill switch.
pub fn confirm_kill_switch(desk: Entity<Desk>, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, _| {
        let desk = desk.clone();
        alert
            .confirm()
            .title("Activate the kill switch?")
            .description(
                "The venue cancels every live order of yours, FIX orders included, and refuses new ones until you reset it.",
            )
            .ok_text("Kill switch")
            .on_ok(move |_, _, cx| {
                desk.update(cx, Desk::kill_switch);
                true
            })
    });
}

pub(crate) trait WhenToggled {
    fn when_toggled(self, toggled: bool) -> Self;
}

impl WhenToggled for Button {
    fn when_toggled(self, toggled: bool) -> Self {
        if toggled {
            self.toggled(true).primary()
        } else {
            self
        }
    }
}

const QUOTE_COLUMNS: [Col; 3] = [
    Col::left("venue", 72.).priority(2),
    Col::right("best bid", 104.).priority(2),
    Col::right("best ask", 104.).priority(2),
];

const TAPE_COLUMNS: [Col; 5] = [
    Col::right("received", 80.).priority(0),
    Col::left("venue", 72.),
    Col::left("symbol", 64.).priority(2),
    Col::right("price", 72.).priority(2),
    Col::right("qty", 56.).priority(2),
];

const BLOTTER_COLUMNS: [Col; 10] = [
    Col::left("venue", 72.),
    Col::left("symbol", 64.).priority(2),
    Col::left("side", 48.).priority(2),
    Col::right("qty", 56.).priority(2),
    Col::right("price", 72.).priority(2),
    Col::right("left", 56.),
    Col::left("status", 130.).priority(2),
    Col::left("queue", 110.).priority(0),
    Col::right("answer", 72.).priority(0),
    Col::center("", 72.).priority(2),
];

const FILL_COLUMNS: [Col; 8] = [
    Col::right("run time", 80.),
    Col::left("venue", 72.),
    Col::left("symbol", 64.).priority(2),
    Col::left("side", 48.).priority(2),
    Col::right("qty", 56.).priority(2),
    Col::right("price", 72.).priority(2),
    Col::right("fee", 64.).priority(0),
    Col::left("liquidity", 72.).priority(0),
];

const POSITION_COLUMNS: [Col; 5] = [
    Col::left("symbol", 72.).priority(2),
    Col::right("position", 80.).priority(2),
    Col::right("open buy", 80.),
    Col::right("open sell", 80.),
    Col::right("fills since", 88.).priority(0),
];

// ---- trades ------------------------------------------------------------------

/// Which trades the tape panel lists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TradeSource {
    /// The consolidated tape from the hub: every venue's best quotes and
    /// trades.
    Consolidated,
    /// Each venue's own direct feed.
    Direct,
}

pub struct TradesPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    source: TradeSource,
    _observe: Subscription,
}

dock_panel!(TradesPanel, "tape", "Tape", true);

impl TradesPanel {
    pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            source: TradeSource::Consolidated,
        }
    }

    fn source_segment(&self, source: TradeSource) -> Button {
        let (id, label, tip) = match source {
            TradeSource::Consolidated => (
                "tape-consolidated",
                "Consolidated",
                "Published at the hub, so near a venue it arrives after that venue's direct feed",
            ),
            TradeSource::Direct => (
                "tape-direct",
                "Direct feeds",
                "Each venue's own feed, newest first, as it reached you after its own distance",
            ),
        };
        ui::segment(id, label, self.source == source).tooltip(tip)
    }

    /// Each venue's best bid and offer as the tape last showed them; the
    /// best across venues is bold.
    fn quotes(&self, cx: &App) -> gpui_kit::Stateful<Div> {
        let desk = self.desk.read(cx);
        let Some(instrument) = desk.instrument() else {
            return v_flex().id("tape-quotes");
        };
        let tape = desk.model().tape(instrument.instrument_id);
        let state = match tape.map(|tape| &tape.state) {
            Some(FeedState::Live) => "live".to_owned(),
            Some(FeedState::Requested) => "subscribing".to_owned(),
            Some(FeedState::Refused(reason)) => format!("refused: {reason}"),
            None => "not subscribed".to_owned(),
        };
        // The quotes scroll inside their own region when an instrument
        // lists on more venues than the pane is tall.
        let mut quotes = v_flex()
            .id("tape-quotes")
            .min_h_0()
            .overflow_y_scroll()
            .child(muted(
                format!("{} from the hub · {state}", instrument.symbol),
                cx,
            ));
        let Some(tape) = tape else {
            return quotes;
        };
        let best_bid = tape.book.best(Side::Buy).map(|(venue, _)| venue);
        let best_ask = tape.book.best(Side::Sell).map(|(venue, _)| venue);
        // Prices are coloured by side; quantities are not. The best
        // across venues is bold.
        let quote = |venue_id: u128, side: Side, best: Option<u128>| -> AnyElement {
            let Some(level) = tape.book.quote(venue_id, side) else {
                return cell("—");
            };
            let element = h_flex()
                .gap_1()
                .items_center()
                .child(format!("{} ×", level.quantity))
                .child(
                    div()
                        .text_color(side_color(side, cx))
                        .child(level.price.to_string()),
                );
            if best == Some(venue_id) {
                element
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .into_any_element()
            } else {
                element.into_any_element()
            }
        };
        quotes = quotes.child(aligned_header(&QUOTE_COLUMNS, cx));
        for info in &instrument.listings {
            let venue_id = info.listing.venue_id;
            quotes = quotes.child(aligned_row(
                &QUOTE_COLUMNS,
                vec![
                    cell(format::venue(info.listing)),
                    quote(venue_id, Side::Buy, best_bid),
                    quote(venue_id, Side::Sell, best_ask),
                ],
                cx,
            ));
        }
        quotes
    }
}

/// Indices of the prints to list for an instrument, newest first.
fn print_indices(prints: &VecDeque<TradePrint>, instrument_id: Option<u128>) -> Vec<usize> {
    (0..prints.len())
        .rev()
        .filter(|&index| instrument_id.is_none_or(|id| prints[index].listing.instrument_id == id))
        .collect()
}

impl Render for TradesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let source = self.source;
        let desk = self.desk.read(cx);
        let instrument_id = desk.instrument().map(|instrument| instrument.instrument_id);
        let prints = match source {
            TradeSource::Consolidated => desk.model().tape_prints(),
            TradeSource::Direct => desk.model().prints(),
        };
        let count = print_indices(prints, instrument_id).len();
        let desk = self.desk.clone();
        let mut body = panel_body("tape", &self.focus).child(
            ui::toolbar(cx).child(
                ButtonGroup::new("tape-source")
                    .xsmall()
                    .child(self.source_segment(TradeSource::Consolidated))
                    .child(self.source_segment(TradeSource::Direct))
                    .on_click(cx.listener(|this, clicks: &Vec<usize>, _, cx| {
                        this.source = if clicks.contains(&0) {
                            TradeSource::Consolidated
                        } else {
                            TradeSource::Direct
                        };
                        cx.notify();
                    })),
            ),
        );
        if source == TradeSource::Consolidated {
            body = body.child(self.quotes(cx));
        }
        if count == 0 {
            return body.child(ui::empty_state(
                "tape-empty",
                IconName::Globe,
                "No trades yet",
                None,
                cx,
            ));
        }
        body.child(ui::responsive_table(&TAPE_COLUMNS, move |fitted, _, cx| {
            let fitted: Vec<ui::Fit> = fitted.to_vec();
            v_flex()
                .size_full()
                .child(ui::table_head(&fitted, cx))
                .child(
                    uniform_list("tape-rows", count, move |range, _, cx| {
                        let desk = desk.read(cx);
                        let model = desk.model();
                        let prints = match source {
                            TradeSource::Consolidated => model.tape_prints(),
                            TradeSource::Direct => model.prints(),
                        };
                        let indices = print_indices(prints, instrument_id);
                        range
                            .filter_map(|index| prints.get(*indices.get(index)?))
                            .map(|print| {
                                ui::table_row(
                                    &fitted,
                                    vec![
                                        cell(format::clock(print.received_us)),
                                        cell(format::venue(print.listing)),
                                        cell(model.symbol(print.listing).unwrap_or("?").to_owned()),
                                        cell(print.price.to_string()),
                                        cell(print.quantity.to_string()),
                                    ],
                                    cx,
                                )
                            })
                            .collect()
                    })
                    .flex_1(),
                )
        }))
    }
}

// ---- orders --------------------------------------------------------------------

reader_panel!(OrdersPanel);
dock_panel!(OrdersPanel, "orders", "Blotter");

impl Render for OrdersPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.desk.read(cx).model().orders().len();
        if count == 0 {
            return panel_body("orders", &self.focus).child(ui::empty_state(
                "orders-empty",
                IconName::Inbox,
                "No open orders",
                None,
                cx,
            ));
        }
        let desk = self.desk.clone();
        let cancel = window.listener_for(&self.desk, |desk, id: &u64, _, cx| desk.cancel(*id, cx));
        let cancel = std::rc::Rc::new(cancel);
        panel_body("orders", &self.focus).child(ui::responsive_table(
            &BLOTTER_COLUMNS,
            move |fitted, _, cx| {
                let fitted: Vec<ui::Fit> = fitted.to_vec();
                v_flex()
                    .size_full()
                    .child(ui::table_head(&fitted, cx))
                    .child(
                        uniform_list("order-rows", count, move |range, _, cx| {
                            let desk = desk.read(cx);
                            let model = desk.model();
                            let orders = model.orders();
                            range
                                .filter_map(|index| {
                                    orders.get(orders.len().checked_sub(index + 1)?)
                                })
                                .map(|order| {
                                    let color = side_color(order.side, cx);
                                    let id = order.client_order_id;
                                    let action = if matches!(
                                        order.status,
                                        OrderStatus::New | OrderStatus::Resting
                                    ) && id != 0
                                    {
                                        let cancel = cancel.clone();
                                        Button::new(SharedString::from(format!("cancel-{id}")))
                                            .label("Cancel")
                                            .xsmall()
                                            .on_click(move |_, window, cx| cancel(&id, window, cx))
                                            .into_any_element()
                                    } else {
                                        cell("")
                                    };
                                    ui::table_row(
                                        &fitted,
                                        vec![
                                            cell(format::venue(order.listing)),
                                            cell(
                                                model
                                                    .symbol(order.listing)
                                                    .unwrap_or("?")
                                                    .to_owned(),
                                            ),
                                            colored(format::side(order.side), color),
                                            cell(order.quantity.to_string()),
                                            cell(format::price(order.price)),
                                            cell(order.remaining.to_string()),
                                            cell(order.status.label()),
                                            cell(model.queue_place(order).map_or_else(
                                                String::new,
                                                |place| {
                                                    format!(
                                                        "#{} · {} ahead",
                                                        place.position_at_price,
                                                        place.quantity_ahead
                                                    )
                                                },
                                            )),
                                            cell(
                                                order
                                                    .answer_us
                                                    .map(format::duration)
                                                    .unwrap_or_default(),
                                            ),
                                            action,
                                        ],
                                        cx,
                                    )
                                })
                                .collect()
                        })
                        .flex_1(),
                    )
            },
        ))
    }
}

// ---- fills ----------------------------------------------------------------------

reader_panel!(FillsPanel);
dock_panel!(FillsPanel, "fills", "Fills");

impl Render for FillsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.desk.read(cx).model().fills().len();
        if count == 0 {
            return panel_body("fills", &self.focus).child(ui::empty_state(
                "fills-empty",
                IconName::CircleCheck,
                "No fills yet",
                None,
                cx,
            ));
        }
        let desk = self.desk.clone();
        panel_body("fills", &self.focus).child(ui::responsive_table(
            &FILL_COLUMNS,
            move |fitted, _, cx| {
                let fitted: Vec<ui::Fit> = fitted.to_vec();
                v_flex()
                    .size_full()
                    .child(ui::table_head(&fitted, cx))
                    .child(
                        uniform_list("fill-rows", count, move |range, _, cx| {
                            let desk = desk.read(cx);
                            let model = desk.model();
                            let fills = model.fills();
                            range
                                .filter_map(|index| fills.get(fills.len().checked_sub(index + 1)?))
                                .map(|fill| {
                                    ui::table_row(
                                        &fitted,
                                        vec![
                                            cell(format::logical(fill.logical_time_ns)),
                                            cell(format::venue(fill.listing)),
                                            cell(
                                                model
                                                    .symbol(fill.listing)
                                                    .unwrap_or("?")
                                                    .to_owned(),
                                            ),
                                            colored(
                                                format::side(fill.side),
                                                side_color(fill.side, cx),
                                            ),
                                            cell(fill.quantity.to_string()),
                                            cell(fill.price.to_string()),
                                            cell(fill.fee.to_string()),
                                            cell(format::liquidity(fill.liquidity)),
                                        ],
                                        cx,
                                    )
                                })
                                .collect()
                        })
                        .flex_1(),
                    )
            },
        ))
    }
}

// ---- positions ---------------------------------------------------------------------

reader_panel!(PositionsPanel);
dock_panel!(PositionsPanel, "positions", "Portfolio");

impl Render for PositionsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let mut body = panel_body("positions", &self.focus);
        let Some(account) = model.account() else {
            return body.child(ui::empty_state(
                "positions-empty",
                IconName::ChartPie,
                "No account answer from the venue yet",
                None,
                cx,
            ));
        };
        let instruments = model.instruments();
        body = body.child(muted(
            format!(
                "As of committed sequence {}, received {} ago",
                account.run_sequence,
                format::duration(desk.now_us().saturating_sub(account.received_us))
            ),
            cx,
        ));
        if account.positions.is_empty() {
            return body.child(ui::empty_state(
                "positions-none",
                IconName::ChartPie,
                "No positions",
                None,
                cx,
            ));
        }
        let mut rows: Vec<Vec<AnyElement>> = Vec::new();
        for position in &account.positions {
            let symbol = instruments
                .iter()
                .find(|instrument| instrument.instrument_id == position.instrument_id)
                .map_or_else(
                    || format!("#{}", position.instrument_id),
                    |instrument| instrument.symbol.clone(),
                );
            let pending = model.unreported_position(position.instrument_id);
            let theme = cx.theme();
            let position_color = match position.position.cmp(&0) {
                std::cmp::Ordering::Greater => theme.chart_bullish,
                std::cmp::Ordering::Less => theme.chart_bearish,
                std::cmp::Ordering::Equal => theme.foreground,
            };
            rows.push(vec![
                cell(symbol),
                colored(position.position.to_string(), position_color),
                cell(position.open_buy.to_string()),
                cell(position.open_sell.to_string()),
                colored(
                    if pending == 0 {
                        String::new()
                    } else {
                        format!("{pending:+}")
                    },
                    theme.warning,
                ),
            ]);
        }
        body.child(ui::responsive_table(
            &POSITION_COLUMNS,
            move |fitted, _, cx| {
                let mut table = v_flex().size_full().child(ui::table_head(fitted, cx));
                let mut list = v_flex()
                    .id("positions-rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll();
                for cells in rows {
                    list = list.child(ui::table_row(fitted, cells, cx));
                }
                table = table.child(list);
                table
            },
        ))
    }
}

// ---- account ---------------------------------------------------------------------------

reader_panel!(AccountPanel);
dock_panel!(AccountPanel, "account", "Trader Info");

impl Render for AccountPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let model = desk.model();
        // The toolbar: the as-of line on the left in muted text, a quiet
        // ghost refresh icon on the right.
        let mut toolbar = ui::toolbar(cx).w_full();
        if let Some(account) = model.account() {
            toolbar = toolbar.child(muted(
                format!("As of committed sequence {}", account.run_sequence),
                cx,
            ));
        }
        let body = panel_body("account", &self.focus).child(
            toolbar.child(div().flex_1()).child(
                Button::new("refresh")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip("Reload orders and account")
                    .on_click(window.listener_for(&self.desk, |desk, _, _, cx| desk.refresh(cx))),
            ),
        );
        let Some(account) = model.account() else {
            return body.child(ui::empty_state(
                "account-empty",
                IconName::CircleUser,
                "No account answer from the venue yet",
                None,
                cx,
            ));
        };
        // Balances as stats, coloured by sign (the model carries no
        // separate P&L figure), in a region that scrolls when there are
        // more currencies than the window is tall.
        let mut stats = v_flex()
            .id("account-stats")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .gap(px(ui::PANE_GAP));
        for balance in &account.cash {
            let theme = cx.theme();
            let balance_color = match balance.balance.cmp(&0) {
                std::cmp::Ordering::Greater => theme.chart_bullish,
                std::cmp::Ordering::Less => theme.chart_bearish,
                std::cmp::Ordering::Equal => theme.foreground,
            };
            stats = stats
                .child(ui::section_label(
                    format!("Currency {}", balance.currency_id),
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_4()
                        .items_start()
                        .child(ui::stat_colored(
                            "Balance",
                            balance.balance.to_string(),
                            balance_color,
                            cx,
                        ))
                        .child(ui::stat("Reserved", balance.reserved.to_string(), cx)),
                );
        }
        body.child(stats)
    }
}

// ---- notices -----------------------------------------------------------------------------

reader_panel!(NoticesPanel);
dock_panel!(NoticesPanel, "notices", "Notices");

impl Render for NoticesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.desk.read(cx).model().notices().len();
        if count == 0 {
            return panel_body("notices", &self.focus).child(ui::empty_state(
                "notices-empty",
                IconName::Bell,
                "No notices yet",
                None,
                cx,
            ));
        }
        let desk = self.desk.clone();
        panel_body("notices", &self.focus).child(
            uniform_list("notice-rows", count, move |range, _, cx| {
                let desk = desk.read(cx);
                let notices = desk.model().notices();
                range
                    .filter_map(|index| {
                        notices
                            .get(notices.len().checked_sub(index + 1)?)
                            .map(|notice| (index, notice))
                    })
                    .map(|(index, notice)| {
                        let color = match notice.level {
                            NoticeLevel::Info => cx.theme().muted_foreground,
                            NoticeLevel::Warning => cx.theme().warning,
                            NoticeLevel::Error => cx.theme().danger,
                        };
                        // Rows are uniform for the virtual list: the text
                        // truncates to one line, the tooltip has it all.
                        let full = notice.text.clone();
                        h_flex()
                            .w_full()
                            .h(ui::rem_px(crate::appearance::metrics(cx).row_height))
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(72.))
                                    .flex_shrink_0()
                                    .child(format::clock(notice.at_us)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("notice-text-{index}")))
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(color)
                                    .tooltip(move |window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new(full.clone())
                                            .build(window, cx)
                                    })
                                    .child(notice.text.clone()),
                            )
                    })
                    .collect()
            })
            .flex_1(),
        )
    }
}

/// The status bar's connection text: state, measured round trips and the
/// last committed sequence. Nothing is smoothed or estimated (ADR 0035).
#[must_use]
pub fn connection_label(desk: &Desk) -> String {
    let model = desk.model();
    let now = desk.now_us();
    let mut text = match model.connection() {
        Connection::Idle => "Not connected".to_owned(),
        Connection::Connecting { attempt } => {
            format!("Connecting to {} (attempt {attempt})", desk.label())
        }
        Connection::Connected { welcome, since_us } => format!(
            "Connected to {} · participant {} · {}",
            desk.label(),
            welcome.participant_id,
            format::duration(now.saturating_sub(*since_us))
        ),
        Connection::Disconnected { reason, .. } => format!("Disconnected: {reason}"),
    };
    let trips = model.round_trips();
    if let (Some(last), Some(min), Some(mean)) = (trips.last_us, trips.min_us, trips.mean_us()) {
        let _ = write!(
            text,
            " · ping {} (min {}, mean {})",
            format::duration(last),
            format::duration(min),
            format::duration(mean)
        );
    }
    // What the venue measured for this connection: published, never used
    // to order anything (ADR 0035).
    if let Some(one_way) = model.access_delay().and_then(|delay| delay.one_way_us) {
        let _ = write!(
            text,
            " · venue measures {} one way",
            format::duration(one_way)
        );
    }
    if model.last_sequence() > 0 {
        let _ = write!(text, " · sequence {}", model.last_sequence());
    }
    if model.kill_switch_active() {
        text.push_str(" · KILL SWITCH ACTIVE");
    }
    text
}
