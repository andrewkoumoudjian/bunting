//! The workspace's panels. Each one renders part of the shared [`Desk`]
//! and redraws when it changes; actions go straight to the desk, which
//! writes them to the venue at once.

use crate::desk::Desk;
use crate::flash::{self, Flash, Flashes};
use crate::format;
use crate::one_click::{self, Gesture, Intent, OneClick};
use crate::theme;
use bunting_trading_state::bunting_client::QueuedOrder;
use bunting_trading_state::bunting_client::bnp_wire::{Level, Listing, Side};
use bunting_trading_state::{
    Connection, FeedState, Instrument, NoticeLevel, OrderStatus, OrderTicket, TicketKind,
    TicketTif, TradePrint,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
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
/// One side of an order-by-order book.
const ORDER_COLUMNS: [(&str, f32); 5] = [
    ("place", 36.),
    ("ahead", 48.),
    ("qty", 48.),
    ("price", 56.),
    ("broker", 48.),
];
/// The cells of [`ORDER_COLUMNS`] and the gaps between them.
const ORDER_SIDE_WIDTH: f32 = 268.;
/// One side of a price-level book.
const LEVEL_COLUMNS: [(&str, f32); 3] = [("mine", 44.), ("qty", 56.), ("price", 64.)];
/// The cells of [`LEVEL_COLUMNS`] and the gaps between them.
const LEVEL_SIDE_WIDTH: f32 = 180.;
/// A venue card's least width: both sides, the gap between them, padding
/// and border (2 × side + 30).
const ORDER_CARD_WIDTH: f32 = 566.;
const LEVEL_CARD_WIDTH: f32 = 390.;
/// Height of one row in the scrolling lists.
pub(crate) const ROW_HEIGHT: f32 = 22.;

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

pub(crate) fn panel_body(
    id: &'static str,
    focus: &FocusHandle,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    v_flex()
        .id(id)
        .test_support()
        .track_focus(focus)
        .size_full()
        .p_2()
        .gap_2()
        .text_sm()
        .font_features(theme::tabular())
}

pub(crate) fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub(crate) fn side_color(side: Side, cx: &App) -> Hsla {
    match side {
        Side::Buy => cx.theme().chart_bullish,
        Side::Sell => cx.theme().chart_bearish,
    }
}

/// One table row of fixed-width cells.
pub(crate) fn row(cells: Vec<(AnyElement, f32)>) -> Div {
    let mut row = h_flex().w_full().h(px(ROW_HEIGHT)).gap_2();
    for (cell, width) in cells {
        row = row.child(
            div()
                .w(px(width))
                .flex_shrink_0()
                .overflow_hidden()
                .child(cell),
        );
    }
    row
}

/// [`row`], for code where a book row's `row` shadows it.
fn row_cells(cells: Vec<(AnyElement, f32)>) -> Div {
    row(cells)
}

pub(crate) fn cell(text: impl Into<SharedString>) -> AnyElement {
    div().child(text.into()).into_any_element()
}

pub(crate) fn colored(text: impl Into<SharedString>, color: Hsla) -> AnyElement {
    div()
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

pub(crate) fn header(cells: &[(&'static str, f32)], cx: &App) -> Div {
    row(cells
        .iter()
        .map(|(label, width)| {
            (
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(*label)
                    .into_any_element(),
                *width,
            )
        })
        .collect())
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
        }
    }

    fn depth_button(&self, depth: Depth, cx: &mut Context<Self>) -> Button {
        let (id, label) = match depth {
            Depth::Levels => ("books-levels", "Levels"),
            Depth::Orders => ("books-orders", "Orders"),
        };
        Button::new(id)
            .label(label)
            .xsmall()
            .when_toggled(self.depth == depth)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.depth = depth;
                cx.notify();
            }))
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

    fn one_click_bar(&self, cx: &mut Context<Self>) -> Div {
        let mut bar = h_flex().gap_2().items_center().child(
            Button::new("books-one-click")
                .label("One-click")
                .xsmall()
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
                        .w(px(92.))
                        .child(NumberInput::new(&self.volume).xsmall()),
                )
                .child(muted("O", cx))
                .child(
                    div()
                        .id("books-offset")
                        .test_support()
                        .w(px(80.))
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
            return body
                .child("No listings yet")
                .child(muted(
                    "Each venue's book appears here as that venue's own feed reports it. Books are never merged.",
                    cx,
                ));
        }
        let mut tabs = h_flex().gap_1();
        for each in &instruments {
            let id = each.instrument_id;
            let selected = instrument
                .as_ref()
                .is_some_and(|shown| shown.instrument_id == id);
            tabs = tabs.child(
                Button::new(SharedString::from(format!("instrument-{id}")))
                    .label(each.symbol.clone())
                    .small()
                    .when_toggled(selected)
                    .on_click(window.listener_for(&self.desk, move |desk, _, _, cx| {
                        desk.select_instrument(id, cx);
                    })),
            );
        }
        body = body.child(
            h_flex()
                .justify_between()
                .flex_wrap()
                .gap_2()
                .child(tabs)
                .child(self.one_click_bar(cx))
                .child(
                    h_flex()
                        .gap_1()
                        .child(self.depth_button(Depth::Orders, cx))
                        .child(self.depth_button(Depth::Levels, cx)),
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
        let hint = if self.one_click {
            "One-click: left-click a bid to buy V at its price + O (an ask: sell at − O), right-click the asks to buy V at market (the bids: sell), shift+right-click to sweep through a price, alt+click to cancel yours."
        } else {
            match self.depth {
                Depth::Levels => {
                    "Click a venue to send there, or a price to put it on the ticket. Alt+click cancels yours at a price. ● marks your resting quantity."
                }
                Depth::Orders => {
                    "Each displayed order in queue order: its place at its price, the quantity ahead of it and its broker. Yours are highlighted. Click an order to put its price on the ticket; alt+click cancels yours."
                }
            }
        };
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .id("books-problem")
                    .test_support()
                    .text_xs()
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
        .child(muted(hint, cx))
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
                .text_xs()
                .text_color(cx.theme().warning)
                .child(format!(
                    "{} is halted on every venue: orders wait for the reopening.",
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

/// A row's background: lit by a flash, else marked when it is yours.
fn row_background(flash: Option<Flash>, own: bool, cx: &App) -> Option<Hsla> {
    let theme = cx.theme();
    match flash {
        Some(Flash::New) => Some(theme.chart_bullish.opacity(0.35)),
        Some(Flash::Traded) => Some(theme.chart_bearish.opacity(0.35)),
        None => own.then(|| theme.primary.opacity(0.22)),
    }
}

impl BooksPanel {
    #[expect(
        clippy::too_many_lines,
        reason = "one venue's column: header, status lines and both sides"
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
        let state = match feed.map(|feed| &feed.state) {
            Some(FeedState::Live) => "live".to_owned(),
            Some(FeedState::Requested) => "subscribing".to_owned(),
            Some(FeedState::Refused(reason)) => format!("refused: {reason}"),
            None => "no feed".to_owned(),
        };
        let age = feed.and_then(|feed| feed.last_message_us).map(|at| {
            format!(
                "last update {} ago",
                format::duration(now.saturating_sub(at))
            )
        });
        let gaps = feed.map_or(0, |feed| feed.gaps);
        let order_trip = model
            .venue_round_trips()
            .get(&listing.venue_id)
            .map(|trip| format!("order round trip {}", format::duration(trip.last_us)));
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
        let theme = cx.theme();
        let border = if selected { theme.ring } else { theme.border };
        // Room for both sides, the gap between them and the padding, so a
        // narrow pane scrolls sideways instead of overlapping the sides.
        let card_width = match self.depth {
            Depth::Orders => ORDER_CARD_WIDTH,
            Depth::Levels => LEVEL_CARD_WIDTH,
        };
        let mut column = v_flex()
            .id(SharedString::from(format!("venue-{venue_id}")))
            .test_support()
            .flex_1()
            .flex_shrink_0()
            .min_w(px(card_width))
            .max_w(px(640.))
            .p_2()
            .gap_1()
            .border_1()
            .border_color(border)
            .rounded_md()
            .cursor_pointer()
            .on_click(window.listener_for(&self.desk, move |desk, _, _, cx| {
                desk.select_listing(listing, cx);
            }))
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .child(format::venue(listing)),
                    )
                    .child(muted(state, cx)),
            );
        if let Some(age) = age {
            column = column.child(muted(age, cx));
        }
        if let Some(trip) = order_trip {
            column = column.child(muted(trip, cx));
        }
        if gaps > 0 {
            column = column.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(format!("{gaps} feed gaps, resubscribed")),
            );
        }
        let spread = muted(
            spread.map_or_else(
                || "one-sided".to_owned(),
                |spread| format!("spread {spread}"),
            ),
            cx,
        );
        let clock = Instant::now();
        column = column.child(spread);
        // Bids on the left and asks on the right, best price at the top of
        // each, as in RIT's Book Trader.
        // Each side is wide enough for its cells, so the two never overlap;
        // the books scroll sideways instead.
        let sides = |bids: Vec<AnyElement>,
                     asks: Vec<AnyElement>,
                     head: &[(&'static str, f32)],
                     width: f32| {
            let list = |rows: Vec<AnyElement>| {
                v_flex()
                    .flex_1()
                    .min_w(px(width))
                    .child(header(head, cx))
                    .children(rows)
            };
            h_flex()
                .w_full()
                .gap_3()
                .items_start()
                .child(list(bids))
                .child(list(asks))
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
                    Self::order_row(row, order, index, flash, mine, cx)
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
                    &ORDER_COLUMNS,
                    ORDER_SIDE_WIDTH,
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
            Self::level_row(row, *level, own(level.price, side), index, flash, cx)
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
            &LEVEL_COLUMNS,
            LEVEL_SIDE_WIDTH,
        ))
    }

    /// A book row that answers clicks: one-click orders, alt+click cancels,
    /// or a price for the ticket.
    fn clickable(
        id: String,
        row: one_click::Row,
        background: Option<Hsla>,
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
            cx,
        )
        .child(row_cells(
            [
                cell(format!("#{}", order.position_at_price)),
                cell(order.quantity_ahead.to_string()),
                colored(order.quantity.to_string(), color),
                colored(order.price.to_string(), color),
                cell(broker),
            ]
            .into_iter()
            .zip(ORDER_COLUMNS.map(|(_, width)| width))
            .collect(),
        ))
        .into_any_element()
    }

    fn level_row(
        row: one_click::Row,
        level: Level,
        mine: i64,
        index: usize,
        flash: Option<Flash>,
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
            cx,
        )
        .child(row_cells(
            [
                cell(mark),
                colored(level.quantity.to_string(), color),
                colored(level.price.to_string(), color),
            ]
            .into_iter()
            .zip(LEVEL_COLUMNS.map(|(_, width)| width))
            .collect(),
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
            return body.child("No listings yet").child(muted(
                "Side, quantity, price, time in force and the venue for each order.",
                cx,
            ));
        };
        body = body.child(
            div()
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .child(instrument.symbol.clone()),
        );
        // The venue: no default, the participant chooses (ADR 0050).
        let mut venues = h_flex().gap_1().flex_wrap();
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
                .when_toggled(chosen == Some(listing))
                .on_click(window.listener_for(
                    &self.desk,
                    move |desk, _, _, cx| {
                        desk.select_listing(listing, cx);
                    },
                )),
            );
        }
        body = body.child(muted("Venue", cx)).child(venues);
        if chosen.is_none() {
            body = body.child(muted("Choose the venue for this order.", cx));
        }
        let side = self.side;
        body = body.child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("ticket-buy")
                        .label("Buy")
                        .small()
                        .when_toggled(side == Side::Buy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.side = Side::Buy;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("ticket-sell")
                        .label("Sell")
                        .small()
                        .when_toggled(side == Side::Sell)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.side = Side::Sell;
                            cx.notify();
                        })),
                )
                .child(div().w(px(12.)))
                .child(
                    Button::new("ticket-limit")
                        .label("Limit")
                        .small()
                        .when_toggled(self.kind == TicketKind::Limit)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.kind = TicketKind::Limit;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("ticket-market")
                        .label("Market")
                        .small()
                        .when_toggled(self.kind == TicketKind::Market)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.kind = TicketKind::Market;
                            cx.notify();
                        })),
                ),
        );
        body = body.child(
            h_flex()
                .gap_2()
                .child(
                    v_flex()
                        .flex_1()
                        .child(muted("Quantity (lots)", cx))
                        .child(NumberInput::new(&self.quantity).small()),
                )
                .child(
                    v_flex().flex_1().child(muted("Price (ticks)", cx)).child(
                        NumberInput::new(&self.price)
                            .small()
                            .disabled(self.kind == TicketKind::Market),
                    ),
                ),
        );
        body = body.child(
            v_flex()
                .id("ticket-display")
                .test_support()
                .child(muted("Show at a time (iceberg; blank shows all)", cx))
                .child(
                    NumberInput::new(&self.display)
                        .small()
                        .disabled(self.kind == TicketKind::Market),
                ),
        );
        let mut tifs = h_flex().gap_1();
        for tif in TicketTif::ALL {
            tifs = tifs.child(
                Button::new(SharedString::from(format!("ticket-tif-{}", tif.label())))
                    .label(tif.label())
                    .xsmall()
                    .when_toggled(self.tif == tif)
                    .disabled(self.kind == TicketKind::Market)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.tif = tif;
                        cx.notify();
                    })),
            );
        }
        body = body.child(tifs).child(
            h_flex()
                .gap_3()
                .child(
                    Checkbox::new("ticket-post-only")
                        .label("Post only")
                        .checked(self.post_only)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.post_only = *checked;
                            cx.notify();
                        })),
                )
                .child(
                    Checkbox::new("ticket-anonymous")
                        .label("Anonymous")
                        .checked(self.anonymous)
                        .on_click(cx.listener(|this, checked: &bool, _, cx| {
                            this.anonymous = *checked;
                            cx.notify();
                        })),
                ),
        );
        let send_label = match (self.side, chosen) {
            (Side::Buy, Some(listing)) => format!("Buy on {}", format::venue(listing)),
            (Side::Sell, Some(listing)) => format!("Sell on {}", format::venue(listing)),
            (_, None) => "Choose a venue".to_owned(),
        };
        let send = Button::new("ticket-send")
            .label(send_label)
            .disabled(!connected || killed || chosen.is_none())
            .on_click(cx.listener(|this, _, _, cx| this.send(cx)));
        let send = match self.side {
            Side::Buy => send.success(),
            Side::Sell => send.danger(),
        };
        body = body.child(send);
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .id("ticket-problem")
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        if !connected {
            body = body.child(muted("Not connected: orders cannot be sent.", cx));
        }
        if killed {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child("Kill switch active: the venue refuses new orders until you reset it."),
            );
        }
        body.child(
            h_flex()
                .gap_1()
                .pt_2()
                .child(
                    Button::new("cancel-all")
                        .label("Cancel all")
                        .small()
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

    fn source_button(&self, source: TradeSource, cx: &mut Context<Self>) -> Button {
        let (id, label) = match source {
            TradeSource::Consolidated => ("tape-consolidated", "Consolidated"),
            TradeSource::Direct => ("tape-direct", "Direct feeds"),
        };
        Button::new(id)
            .label(label)
            .xsmall()
            .when_toggled(self.source == source)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.source = source;
                cx.notify();
            }))
    }

    /// Each venue's best bid and offer as the tape last showed them; the
    /// best across venues is bold.
    fn quotes(&self, cx: &App) -> Div {
        let desk = self.desk.read(cx);
        let Some(instrument) = desk.instrument() else {
            return div();
        };
        let tape = desk.model().tape(instrument.instrument_id);
        let state = match tape.map(|tape| &tape.state) {
            Some(FeedState::Live) => "live".to_owned(),
            Some(FeedState::Requested) => "subscribing".to_owned(),
            Some(FeedState::Refused(reason)) => format!("refused: {reason}"),
            None => "not subscribed".to_owned(),
        };
        let mut quotes = v_flex().child(muted(
            format!("{} from the hub · {state}", instrument.symbol),
            cx,
        ));
        let Some(tape) = tape else {
            return quotes;
        };
        let best_bid = tape.book.best(Side::Buy).map(|(venue, _)| venue);
        let best_ask = tape.book.best(Side::Sell).map(|(venue, _)| venue);
        let quote = |venue_id: u128, side: Side, best: Option<u128>| -> AnyElement {
            let Some(level) = tape.book.quote(venue_id, side) else {
                return cell("—");
            };
            let text = format!("{} × {}", level.quantity, level.price);
            let element = div().text_color(side_color(side, cx)).child(text);
            if best == Some(venue_id) {
                element
                    .font_weight(gpui_kit::FontWeight::BOLD)
                    .into_any_element()
            } else {
                element.into_any_element()
            }
        };
        quotes = quotes.child(header(
            &[("venue", 64.), ("best bid", 96.), ("best ask", 96.)],
            cx,
        ));
        for info in &instrument.listings {
            let venue_id = info.listing.venue_id;
            quotes = quotes.child(row(vec![
                (cell(format::venue(info.listing)), 64.),
                (quote(venue_id, Side::Buy, best_bid), 96.),
                (quote(venue_id, Side::Sell, best_ask), 96.),
            ]));
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
        let columns = [
            ("received", 72.),
            ("venue", 64.),
            ("symbol", 64.),
            ("price", 72.),
            ("qty", 56.),
        ];
        let mut body = panel_body("tape", &self.focus).child(
            h_flex()
                .gap_1()
                .child(self.source_button(TradeSource::Consolidated, cx))
                .child(self.source_button(TradeSource::Direct, cx)),
        );
        body = match source {
            TradeSource::Consolidated => body.child(self.quotes(cx)).child(muted(
                "The tape is published at the hub, so near a venue it arrives after that venue's direct feed.",
                cx,
            )),
            TradeSource::Direct => body.child(muted(
                "Trades from each venue's direct feed, newest first, as they reached you; each venue's arrive after its own distance.",
                cx,
            )),
        };
        body.child(header(&columns, cx)).child(
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
                        row(vec![
                            (cell(format::clock(print.received_us)), 72.),
                            (cell(format::venue(print.listing)), 64.),
                            (
                                cell(model.symbol(print.listing).unwrap_or("?").to_owned()),
                                64.,
                            ),
                            (cell(print.price.to_string()), 72.),
                            (cell(print.quantity.to_string()), 56.),
                        ])
                    })
                    .collect()
            })
            .flex_1(),
        )
    }
}

// ---- orders --------------------------------------------------------------------

reader_panel!(OrdersPanel);
dock_panel!(OrdersPanel, "orders", "Blotter");

impl Render for OrdersPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.desk.read(cx).model().orders().len();
        let desk = self.desk.clone();
        let cancel = window.listener_for(&self.desk, |desk, id: &u64, _, cx| desk.cancel(*id, cx));
        let cancel = std::rc::Rc::new(cancel);
        let columns = [
            ("venue", 64.),
            ("symbol", 64.),
            ("side", 40.),
            ("qty", 56.),
            ("price", 72.),
            ("left", 56.),
            ("status", 150.),
            ("queue", 96.),
            ("answer", 72.),
            ("", 64.),
        ];
        panel_body("orders", &self.focus)
            .child(header(&columns, cx))
            .child(
                uniform_list("order-rows", count, move |range, _, cx| {
                    let desk = desk.read(cx);
                    let model = desk.model();
                    let orders = model.orders();
                    range
                        .filter_map(|index| orders.get(orders.len().checked_sub(index + 1)?))
                        .map(|order| {
                            let color = side_color(order.side, cx);
                            let id = order.client_order_id;
                            let action =
                                if matches!(order.status, OrderStatus::New | OrderStatus::Resting)
                                    && id != 0
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
                            row(vec![
                                (cell(format::venue(order.listing)), 64.),
                                (
                                    cell(model.symbol(order.listing).unwrap_or("?").to_owned()),
                                    64.,
                                ),
                                (colored(format::side(order.side), color), 40.),
                                (cell(order.quantity.to_string()), 56.),
                                (cell(format::price(order.price)), 72.),
                                (cell(order.remaining.to_string()), 56.),
                                (cell(order.status.label()), 150.),
                                (
                                    cell(model.queue_place(order).map_or_else(
                                        String::new,
                                        |place| {
                                            format!(
                                                "#{} · {} ahead",
                                                place.position_at_price, place.quantity_ahead
                                            )
                                        },
                                    )),
                                    96.,
                                ),
                                (
                                    cell(order.answer_us.map(format::duration).unwrap_or_default()),
                                    72.,
                                ),
                                (action, 64.),
                            ])
                        })
                        .collect()
                })
                .flex_1(),
            )
    }
}

// ---- fills ----------------------------------------------------------------------

reader_panel!(FillsPanel);
dock_panel!(FillsPanel, "fills", "Fills");

impl Render for FillsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.desk.read(cx).model().fills().len();
        let desk = self.desk.clone();
        let columns = [
            ("run time", 72.),
            ("venue", 64.),
            ("symbol", 64.),
            ("side", 40.),
            ("qty", 56.),
            ("price", 72.),
            ("fee", 64.),
            ("", 56.),
        ];
        panel_body("fills", &self.focus)
            .child(header(&columns, cx))
            .child(
                uniform_list("fill-rows", count, move |range, _, cx| {
                    let desk = desk.read(cx);
                    let model = desk.model();
                    let fills = model.fills();
                    range
                        .filter_map(|index| fills.get(fills.len().checked_sub(index + 1)?))
                        .map(|fill| {
                            row(vec![
                                (cell(format::logical(fill.logical_time_ns)), 72.),
                                (cell(format::venue(fill.listing)), 64.),
                                (
                                    cell(model.symbol(fill.listing).unwrap_or("?").to_owned()),
                                    64.,
                                ),
                                (
                                    colored(format::side(fill.side), side_color(fill.side, cx)),
                                    40.,
                                ),
                                (cell(fill.quantity.to_string()), 56.),
                                (cell(fill.price.to_string()), 72.),
                                (cell(fill.fee.to_string()), 64.),
                                (cell(format::liquidity(fill.liquidity)), 56.),
                            ])
                        })
                        .collect()
                })
                .flex_1(),
            )
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
            return body.child(muted("No account answer from the venue yet.", cx));
        };
        let instruments = model.instruments();
        body = body
            .child(muted(
                format!(
                    "As of committed sequence {}, received {} ago",
                    account.run_sequence,
                    format::duration(desk.now_us().saturating_sub(account.received_us))
                ),
                cx,
            ))
            .child(header(
                &[
                    ("symbol", 72.),
                    ("position", 72.),
                    ("open buy", 72.),
                    ("open sell", 72.),
                    ("fills since", 80.),
                ],
                cx,
            ));
        for position in &account.positions {
            let symbol = instruments
                .iter()
                .find(|instrument| instrument.instrument_id == position.instrument_id)
                .map_or_else(
                    || format!("#{}", position.instrument_id),
                    |instrument| instrument.symbol.clone(),
                );
            let pending = model.unreported_position(position.instrument_id);
            body = body.child(row(vec![
                (cell(symbol), 72.),
                (cell(position.position.to_string()), 72.),
                (
                    colored(position.open_buy.to_string(), cx.theme().chart_bullish),
                    72.,
                ),
                (
                    colored(position.open_sell.to_string(), cx.theme().chart_bearish),
                    72.,
                ),
                (
                    colored(
                        if pending == 0 {
                            String::new()
                        } else {
                            format!("{pending:+}")
                        },
                        cx.theme().warning,
                    ),
                    80.,
                ),
            ]));
        }
        body
    }
}

// ---- account ---------------------------------------------------------------------------

reader_panel!(AccountPanel);
dock_panel!(AccountPanel, "account", "Trader Info");

impl Render for AccountPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let mut body = panel_body("account", &self.focus).child(
            h_flex().gap_2().child(
                Button::new("refresh")
                    .label("Reload orders and account")
                    .xsmall()
                    .on_click(window.listener_for(&self.desk, |desk, _, _, cx| desk.refresh(cx))),
            ),
        );
        let Some(account) = model.account() else {
            return body.child(muted("No account answer from the venue yet.", cx));
        };
        body = body
            .child(muted(
                format!("As of committed sequence {}", account.run_sequence),
                cx,
            ))
            .child(header(
                &[("currency", 96.), ("balance", 120.), ("reserved", 120.)],
                cx,
            ));
        for balance in &account.cash {
            body = body.child(row(vec![
                (cell(balance.currency_id.to_string()), 96.),
                (cell(balance.balance.to_string()), 120.),
                (cell(balance.reserved.to_string()), 120.),
            ]));
        }
        body
    }
}

// ---- notices -----------------------------------------------------------------------------

reader_panel!(NoticesPanel);
dock_panel!(NoticesPanel, "notices", "Notices");

impl Render for NoticesPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.desk.read(cx).model().notices().len();
        let desk = self.desk.clone();
        panel_body("notices", &self.focus).child(
            uniform_list("notice-rows", count, move |range, _, cx| {
                let desk = desk.read(cx);
                let notices = desk.model().notices();
                range
                    .filter_map(|index| notices.get(notices.len().checked_sub(index + 1)?))
                    .map(|notice| {
                        let color = match notice.level {
                            NoticeLevel::Info => cx.theme().muted_foreground,
                            NoticeLevel::Warning => cx.theme().warning,
                            NoticeLevel::Error => cx.theme().danger,
                        };
                        row(vec![
                            (cell(format::clock(notice.at_us)), 72.),
                            (colored(notice.text.clone(), color), 640.),
                        ])
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
