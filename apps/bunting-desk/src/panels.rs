//! The workspace's panels. Each one renders part of the shared [`Desk`]
//! and redraws when it changes; actions go straight to the desk, which
//! writes them to the venue at once.

use crate::desk::Desk;
use crate::format;
use bunting_trading_state::bunting_client::QueuedOrder;
use bunting_trading_state::bunting_client::bnp_wire::{Level, Listing, Side};
use bunting_trading_state::{
    Connection, FeedState, NoticeLevel, OrderStatus, OrderTicket, TicketKind, TicketTif, TradePrint,
};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Div, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, Window, div,
    px, uniform_list,
};
use std::collections::VecDeque;
use std::fmt::Write as _;

/// Book levels shown per side.
const BOOK_DEPTH: usize = 12;
/// Displayed orders shown per side in the order-by-order view.
const ORDER_DEPTH: usize = 16;
/// Venues shown side by side.
const MAX_VENUE_COLUMNS: usize = 6;
/// Height of one row in the scrolling lists.
const ROW_HEIGHT: f32 = 22.;

/// The boilerplate every dock panel shares.
macro_rules! dock_panel {
    ($panel:ident, $name:literal, $title:literal) => {
        impl BasePanel for $panel {
            fn panel_name(&self) -> &'static str {
                $name
            }
        }

        impl Panel for $panel {
            fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                $title
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

fn panel_body(
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
}

fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

fn side_color(side: Side, cx: &App) -> Hsla {
    match side {
        Side::Buy => cx.theme().chart_bullish,
        Side::Sell => cx.theme().chart_bearish,
    }
}

/// One table row of fixed-width cells.
fn row(cells: Vec<(AnyElement, f32)>) -> Div {
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

fn cell(text: impl Into<SharedString>) -> AnyElement {
    div().child(text.into()).into_any_element()
}

fn colored(text: impl Into<SharedString>, color: Hsla) -> AnyElement {
    div()
        .text_color(color)
        .child(text.into())
        .into_any_element()
}

fn header(cells: &[(&'static str, f32)], cx: &App) -> Div {
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
    _observe: Subscription,
}

dock_panel!(BooksPanel, "books", "Books");

impl BooksPanel {
    pub fn new(desk: Entity<Desk>, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            depth: Depth::Orders,
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
            h_flex().justify_between().child(tabs).child(
                h_flex()
                    .gap_1()
                    .child(self.depth_button(Depth::Orders, cx))
                    .child(self.depth_button(Depth::Levels, cx)),
            ),
        );
        let Some(instrument) = instrument else {
            return body;
        };
        let mut columns = h_flex().flex_1().min_h_0().gap_2().items_start();
        for info in instrument.listings.iter().take(MAX_VENUE_COLUMNS) {
            columns = columns.child(self.book_column(info.listing, chosen, window, cx));
        }
        let hint = match self.depth {
            Depth::Levels => {
                "Click a venue to send there, or a price to put it on the ticket. ● marks your resting quantity."
            }
            Depth::Orders => {
                "Each displayed order in queue order: its place at its price, the quantity ahead of it and its broker where the venue publishes one. Click a venue to send there, or an order to put its price on the ticket."
            }
        };
        body.child(columns).child(muted(hint, cx))
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
        let (asks, bids, spread) = feed.map_or_else(
            || (Vec::new(), Vec::new(), None),
            |feed| {
                let spread = match (feed.book.best_bid(), feed.book.best_ask()) {
                    (Some(bid), Some(ask)) => Some(ask.price - bid.price),
                    _ => None,
                };
                let asks: Vec<(Level, i64)> = feed
                    .book
                    .asks()
                    .into_iter()
                    .take(BOOK_DEPTH)
                    .map(|level| (level, own(level.price, Side::Sell)))
                    .collect();
                let bids: Vec<(Level, i64)> = feed
                    .book
                    .bids()
                    .into_iter()
                    .take(BOOK_DEPTH)
                    .map(|level| (level, own(level.price, Side::Buy)))
                    .collect();
                (asks, bids, spread)
            },
        );
        let selected = chosen == Some(listing);
        let venue_id = listing.venue_id;
        let theme = cx.theme();
        let border = if selected { theme.ring } else { theme.border };
        let mut column = v_flex()
            .id(SharedString::from(format!("venue-{venue_id}")))
            .test_support()
            .flex_1()
            .min_w(px(220.))
            .max_w(px(420.))
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
        let bid_color = cx.theme().chart_bullish;
        let ask_color = cx.theme().chart_bearish;
        if self.depth == Depth::Orders {
            if let Some(feed) = feed.filter(|feed| feed.order_by_order) {
                let asks = feed.book.orders(Side::Sell);
                let bids = feed.book.orders(Side::Buy);
                column = column.child(header(
                    &[
                        ("place", 44.),
                        ("ahead", 56.),
                        ("qty", 56.),
                        ("price", 64.),
                        ("broker", 56.),
                    ],
                    cx,
                ));
                for (index, order) in asks.iter().take(ORDER_DEPTH).enumerate().rev() {
                    column = column.child(self.order_row(listing, order, index, ask_color, window));
                }
                column = column.child(spread);
                for (index, order) in bids.iter().take(ORDER_DEPTH).enumerate() {
                    column = column.child(self.order_row(listing, order, index, bid_color, window));
                }
                let hidden =
                    asks.len().saturating_sub(ORDER_DEPTH) + bids.len().saturating_sub(ORDER_DEPTH);
                if hidden > 0 {
                    column = column.child(muted(format!("{hidden} more orders deeper"), cx));
                }
                return column;
            }
            if feed.is_some_and(|feed| matches!(feed.state, FeedState::Live)) {
                column = column.child(muted("This venue sends price levels only.", cx));
            }
        }
        column = column.child(header(
            &[("mine", 44.), ("bid", 64.), ("price", 72.), ("ask", 64.)],
            cx,
        ));
        for (index, (level, mine)) in asks.iter().enumerate().rev() {
            column = column.child(self.level_row(
                listing,
                *level,
                *mine,
                Side::Sell,
                index,
                ask_color,
                window,
            ));
        }
        column = column.child(spread);
        for (index, (level, mine)) in bids.iter().enumerate() {
            column = column.child(self.level_row(
                listing,
                *level,
                *mine,
                Side::Buy,
                index,
                bid_color,
                window,
            ));
        }
        column
    }

    /// One displayed order: its place at its price, the displayed quantity
    /// ahead of it, its size, price and broker.
    fn order_row(
        &self,
        listing: Listing,
        order: &QueuedOrder,
        index: usize,
        color: Hsla,
        window: &mut Window,
    ) -> impl IntoElement {
        let side_name = match order.side {
            Side::Buy => "bid",
            Side::Sell => "ask",
        };
        let price = order.price;
        div()
            .id(SharedString::from(format!(
                "{side_name}-order-{}-{index}",
                listing.venue_id
            )))
            .test_support()
            .cursor_pointer()
            .on_click(window.listener_for(&self.desk, move |desk, _, _, cx| {
                desk.pick_price(listing, price, cx);
            }))
            .child(row(vec![
                (cell(format!("#{}", order.position_at_price)), 44.),
                (cell(order.quantity_ahead.to_string()), 56.),
                (colored(order.quantity.to_string(), color), 56.),
                (colored(price.to_string(), color), 64.),
                (
                    cell(
                        order
                            .broker
                            .map(|broker| broker.to_string())
                            .unwrap_or_default(),
                    ),
                    56.,
                ),
            ]))
    }

    #[allow(clippy::too_many_arguments)]
    fn level_row(
        &self,
        listing: Listing,
        level: Level,
        mine: i64,
        side: Side,
        index: usize,
        color: Hsla,
        window: &mut Window,
    ) -> impl IntoElement {
        let mark = if mine > 0 {
            format!("● {mine}")
        } else {
            String::new()
        };
        let quantity = level.quantity.to_string();
        let (bid, ask) = match side {
            Side::Buy => (colored(quantity, color), cell("")),
            Side::Sell => (cell(""), colored(quantity, color)),
        };
        let side_name = match side {
            Side::Buy => "bid",
            Side::Sell => "ask",
        };
        let price = level.price;
        div()
            .id(SharedString::from(format!(
                "{side_name}-{}-{index}",
                listing.venue_id
            )))
            .test_support()
            .cursor_pointer()
            .on_click(window.listener_for(&self.desk, move |desk, _, _, cx| {
                desk.pick_price(listing, price, cx);
            }))
            .child(row(vec![
                (cell(mark), 44.),
                (bid, 64.),
                (colored(price.to_string(), color), 72.),
                (ask, 64.),
            ]))
    }
}

// ---- order ticket ------------------------------------------------------------

pub struct TicketPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    quantity: Entity<InputState>,
    price: Entity<InputState>,
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

dock_panel!(TicketPanel, "ticket", "Order ticket");

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
            display_quantity: None,
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
                            let desk = this.desk.clone();
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
                        }))
                }),
        )
    }
}

trait WhenToggled {
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

dock_panel!(TradesPanel, "tape", "Tape");

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
dock_panel!(OrdersPanel, "orders", "Orders");

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
dock_panel!(PositionsPanel, "positions", "Positions");

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
dock_panel!(AccountPanel, "account", "Account");

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
