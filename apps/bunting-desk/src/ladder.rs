//! RIT's Ladder Trader: one venue's book on a fixed price axis, with your
//! own resting quantity beside each side.
//!
//! Columns, as in RIT: your bids, bid size, price (high at the top), ask
//! size, your asks. Click a bid size to buy V at that price, an ask size to
//! sell V at it; click your own quantity to cancel it. The axis stays put
//! while the market moves, so a click lands on the price it shows; Centre
//! (or Space) brings the market back to the middle. Every order goes to
//! the venue this ladder shows, which the trader picks.

use crate::actions::CentreLadder;
use crate::desk::Desk;
use crate::format;
use crate::panels::{WhenToggled as _, cell, dock_panel, header, muted, panel_body, row};
use bunting_trading_state::bunting_client::bnp_wire::{Listing, Side};
use bunting_trading_state::{OrderTicket, TicketKind};
use gpui_kit::component::button::Button;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, div, px,
};
use std::collections::BTreeMap;

/// Prices shown above and below the centre.
pub const HALF_SPAN: i64 = 12;
/// The key context the ladder sets.
pub const CONTEXT: &str = "Ladder";

const COLUMNS: [(&str, f32); 5] = [
    ("mine", 56.),
    ("bid", 64.),
    ("price", 72.),
    ("ask", 64.),
    ("mine", 56.),
];

pub struct LadderPanel {
    desk: Entity<Desk>,
    focus: FocusHandle,
    /// The venue shown and traded; none until the trader picks one.
    listing: Option<Listing>,
    /// The price in the middle of the axis.
    centre: Option<i64>,
    volume: Entity<InputState>,
    problem: Option<String>,
    _observe: Subscription,
}

dock_panel!(LadderPanel, "ladder", "Ladder Trader", true);

impl LadderPanel {
    pub fn new(desk: Entity<Desk>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            _observe: cx.observe(&desk, |_, _, cx| cx.notify()),
            desk,
            focus: cx.focus_handle(),
            listing: None,
            centre: None,
            volume: cx.new(|cx| InputState::new(window, cx).default_value("100")),
            problem: None,
        }
    }

    /// The venue shown, if it lists the desk's instrument.
    fn listing(&self, cx: &App) -> Option<Listing> {
        let instrument = self.desk.read(cx).instrument()?;
        self.listing
            .filter(|listing| listing.instrument_id == instrument.instrument_id)
    }

    /// The middle of the market on the shown venue: the mid of the best
    /// prices, else the one side shown, else the last trade.
    fn market(&self, listing: Listing, cx: &App) -> Option<i64> {
        let desk = self.desk.read(cx);
        let feed = desk.model().feed(listing)?;
        match (feed.book.best_bid(), feed.book.best_ask()) {
            (Some(bid), Some(ask)) => Some(bid.price + (ask.price - bid.price) / 2),
            (Some(one), None) | (None, Some(one)) => Some(one.price),
            (None, None) => feed.book.trades().last().map(|(price, _)| *price),
        }
    }

    /// Puts the market back in the middle of the axis.
    pub fn centre(&mut self, cx: &mut Context<Self>) {
        if let Some(listing) = self.listing(cx) {
            self.centre = self.market(listing, cx);
            cx.notify();
        }
    }

    fn volume(&self, cx: &App) -> i64 {
        self.volume
            .read(cx)
            .value()
            .trim()
            .parse::<i64>()
            .unwrap_or(0)
    }

    /// A limit for V at `price` on the shown venue.
    fn trade(&mut self, side: Side, price: i64, cx: &mut Context<Self>) {
        let Some(listing) = self.listing(cx) else {
            return;
        };
        let mut ticket = OrderTicket::new(Some(listing), side);
        ticket.kind = TicketKind::Limit;
        ticket.price = price;
        ticket.quantity = self.volume(cx);
        let result = self.desk.update(cx, |desk, cx| desk.place(&ticket, cx));
        self.problem = result.err().map(|error| error.to_string());
        cx.notify();
    }

    fn cancel(&mut self, side: Side, price: i64, cx: &mut Context<Self>) {
        if let Some(listing) = self.listing(cx) {
            self.desk
                .update(cx, |desk, cx| desk.cancel_at(listing, side, price, cx));
            self.problem = None;
            cx.notify();
        }
    }

    fn venue_buttons(&self, cx: &mut Context<Self>) -> AnyElement {
        let desk = self.desk.read(cx);
        let Some(instrument) = desk.instrument() else {
            return div().into_any_element();
        };
        let shown = self.listing(cx);
        let mut buttons = h_flex().gap_1();
        for info in &instrument.listings {
            let listing = info.listing;
            buttons = buttons.child(
                Button::new(SharedString::from(format!(
                    "ladder-venue-{}",
                    listing.venue_id
                )))
                .label(format::venue(listing))
                .xsmall()
                .when_toggled(shown == Some(listing))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.listing = Some(listing);
                    this.centre = this.market(listing, cx);
                    cx.notify();
                })),
            );
        }
        buttons.into_any_element()
    }

    /// One clickable cell: a size that trades, or your quantity that
    /// cancels.
    fn cell_button(
        id: String,
        text: String,
        color: Option<gpui_kit::Hsla>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let mut element = div()
            .id(SharedString::from(id))
            .test_support()
            .w_full()
            .cursor_pointer()
            .rounded_sm()
            .hover(|style| style.bg(gpui_kit::hsla(0., 0., 1., 0.06)))
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
            .child(text);
        if let Some(color) = color {
            element = element.text_color(color);
        }
        element.into_any_element()
    }
}

impl Render for LadderPanel {
    #[expect(
        clippy::too_many_lines,
        reason = "the ladder's header and its rows read best together"
    )]
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus.clone();
        let mut body = panel_body("ladder", &self.focus)
            .key_context(CONTEXT)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&focus, cx);
            })
            .on_action(cx.listener(|this, _: &CentreLadder, _, cx| this.centre(cx)));
        if self.desk.read(cx).instrument().is_none() {
            return body.child("No listings yet");
        }
        let venues = self.venue_buttons(cx);
        body = body.child(
            h_flex()
                .gap_2()
                .items_center()
                .flex_wrap()
                .child(venues)
                .child(muted("V", cx))
                .child(
                    div()
                        .id("ladder-volume")
                        .test_support()
                        .w(px(92.))
                        .child(NumberInput::new(&self.volume).xsmall()),
                )
                .child(
                    Button::new("ladder-centre")
                        .label("Centre")
                        .xsmall()
                        .on_click(cx.listener(|this, _, _, cx| this.centre(cx))),
                ),
        );
        let Some(listing) = self.listing(cx) else {
            return body.child(muted(
                "Choose the venue this ladder shows and trades on. Each venue's book is its own.",
                cx,
            ));
        };
        if self.centre.is_none() {
            self.centre = self.market(listing, cx);
        }
        let Some(centre) = self.centre else {
            return body.child(muted("No prices on this venue yet.", cx));
        };
        let desk = self.desk.read(cx);
        let model = desk.model();
        let feed = model.feed(listing);
        let levels = |side: Side| -> BTreeMap<i64, i64> {
            feed.map(|feed| match side {
                Side::Buy => feed.book.bids(),
                Side::Sell => feed.book.asks(),
            })
            .unwrap_or_default()
            .into_iter()
            .map(|level| (level.price, level.quantity))
            .collect()
        };
        let (bids, asks) = (levels(Side::Buy), levels(Side::Sell));
        let mut mine: BTreeMap<(Side, i64), i64> = BTreeMap::new();
        for order in model.live_orders().filter(|order| order.listing == listing) {
            if let Some(price) = order.price {
                *mine.entry((order.side, price)).or_default() += order.remaining;
            }
        }
        let last = feed.and_then(|feed| feed.book.trades().last().map(|(price, _)| *price));
        let best_bid = bids.keys().next_back().copied();
        let best_ask = asks.keys().next().copied();
        let position = model.account().map(|account| {
            account
                .positions
                .iter()
                .find(|each| each.instrument_id == listing.instrument_id)
                .map_or(0, |each| each.position)
                + model.unreported_position(listing.instrument_id)
        });
        let theme = cx.theme();
        let (bid_color, ask_color) = (theme.chart_bullish, theme.chart_bearish);
        let (accent, quote_bg) = (theme.primary, theme.secondary);
        body = body.child(muted(
            format!(
                "{} · position {} · last {}",
                format::venue(listing),
                position.map_or_else(|| "–".to_owned(), |position| position.to_string()),
                last.map_or_else(|| "–".to_owned(), |price| price.to_string()),
            ),
            cx,
        ));
        let mut rows = v_flex()
            .id("ladder-rows")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(header(&COLUMNS, cx));
        for price in (centre - HALF_SPAN..=centre + HALF_SPAN).rev() {
            let my_bid = mine.get(&(Side::Buy, price)).copied().unwrap_or(0);
            let my_ask = mine.get(&(Side::Sell, price)).copied().unwrap_or(0);
            let bid = bids.get(&price).copied();
            let ask = asks.get(&price).copied();
            let mine_cell = |side: Side, quantity: i64| {
                if quantity > 0 {
                    let name = match side {
                        Side::Buy => "mybid",
                        Side::Sell => "myask",
                    };
                    Self::cell_button(
                        format!("ladder-{name}-{price}"),
                        format!("● {quantity}"),
                        Some(accent),
                        move |this, cx| this.cancel(side, price, cx),
                        cx,
                    )
                } else {
                    cell("")
                }
            };
            let size_cell = |side: Side, size: Option<i64>, color| {
                let name = match side {
                    Side::Buy => "bid",
                    Side::Sell => "ask",
                };
                Self::cell_button(
                    format!("ladder-{name}-{price}"),
                    size.map(|size| size.to_string()).unwrap_or_default(),
                    Some(color),
                    move |this, cx| this.trade(side, price, cx),
                    cx,
                )
            };
            let mut price_cell = div().child(price.to_string());
            if last == Some(price) {
                price_cell = price_cell
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(accent);
            }
            let quoted = Some(price) == best_bid || Some(price) == best_ask;
            let mut line = div().id(SharedString::from(format!("ladder-row-{price}")));
            if quoted {
                line = line.bg(quote_bg);
            }
            rows = rows.child(line.child(row(vec![
                (mine_cell(Side::Buy, my_bid), 56.),
                (size_cell(Side::Buy, bid, bid_color), 64.),
                (price_cell.into_any_element(), 72.),
                (size_cell(Side::Sell, ask, ask_color), 64.),
                (mine_cell(Side::Sell, my_ask), 56.),
            ])));
        }
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        body.child(rows).child(muted(
            "Click a bid size to buy V there, an ask size to sell V there; click ● to cancel yours. Space or Centre recentres.",
            cx,
        ))
    }
}
