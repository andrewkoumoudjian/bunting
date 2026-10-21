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
use crate::panels::{WhenToggled as _, dock_panel};
use crate::ui::{self, Col};
use bunting_trading_state::bunting_client::bnp_wire::{Listing, Side};
use bunting_trading_state::{OrderTicket, TicketKind};
use gpui_kit::component::button::Button;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputState, NumberInput};
use gpui_kit::component::{ActiveTheme as _, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _,
    Window, div,
};
use std::collections::BTreeMap;

/// Prices shown above and below the centre.
pub const HALF_SPAN: i64 = 12;
/// The key context the ladder sets.
pub const CONTEXT: &str = "Ladder";

/// Bid sizes right-align toward the price axis and ask sizes left-align
/// away from it, so size grows toward the market on both sides.
const COLUMNS: [Col; 5] = [
    Col::center("mine", 56.).priority(0),
    Col::right("bid", 64.).priority(2),
    Col::center("price", 72.).priority(2),
    Col::left("ask", 64.).priority(2),
    Col::center("mine", 56.).priority(0),
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
        align: ui::Align,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        cx: &Context<Self>,
    ) -> AnyElement {
        let hover_bg = cx.theme().secondary;
        let element = div()
            .id(SharedString::from(id))
            .test_support()
            .w_full()
            .cursor_pointer()
            .rounded_sm()
            .hover(move |style| style.bg(hover_bg))
            .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)));
        let mut element = match align {
            ui::Align::Left => element.text_left(),
            ui::Align::Right => element.text_right(),
            ui::Align::Center => element.text_center(),
        }
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
        let mut body = ui::pane_body("ladder", &self.focus)
            .key_context(CONTEXT)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                window.focus(&focus, cx);
            })
            .on_action(cx.listener(|this, _: &CentreLadder, _, cx| this.centre(cx)));
        if self.desk.read(cx).instrument().is_none() {
            return body.child(ui::empty_state(
                "ladder-empty",
                IconName::ChevronsUpDown,
                "No listings yet",
                None,
                cx,
            ));
        }
        let venues = self.venue_buttons(cx);
        body = body.child(
            ui::toolbar(cx)
                .w_full()
                .flex_wrap()
                .child(venues)
                .child(ui::muted("V", cx))
                .child(
                    div()
                        .id("ladder-volume")
                        .test_support()
                        .w(ui::rem(ui::NUMBER_INPUT_MIN_WIDTH))
                        .child(NumberInput::new(&self.volume).xsmall()),
                )
                .child(
                    Button::new("ladder-centre")
                        .label("Centre")
                        .xsmall()
                        .tooltip(
                            "Click a bid size to buy V there, an ask size to sell V there; click ● to cancel yours. Space recentres",
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.centre(cx))),
                ),
        );
        let Some(listing) = self.listing(cx) else {
            return body.child(ui::empty_state(
                "ladder-no-venue",
                IconName::ChevronsUpDown,
                "Choose the venue this ladder shows and trades on",
                None,
                cx,
            ));
        };
        if self.centre.is_none() {
            self.centre = self.market(listing, cx);
        }
        let Some(centre) = self.centre else {
            return body.child(ui::empty_state(
                "ladder-no-prices",
                IconName::ChevronsUpDown,
                "No prices on this venue yet",
                None,
                cx,
            ));
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
        let cost = model.cost(listing.instrument_id);
        let theme = cx.theme();
        let (accent, quote_bg) = (theme.primary, theme.secondary);
        let position_color = match position {
            Some(value) if value > 0 => theme.chart_bullish,
            Some(value) if value < 0 => theme.chart_bearish,
            _ => theme.foreground,
        };
        body = body.child(
            h_flex()
                .gap_4()
                .items_start()
                .child(ui::stat("Venue", format::venue(listing), cx))
                .child(ui::stat_colored(
                    "Position",
                    position.map_or_else(|| "–".to_owned(), |value| value.to_string()),
                    position_color,
                    cx,
                ))
                .child(ui::tipped(
                    "ladder-cost-tip",
                    "Volume-weighted average price of your current position, derived from your fills",
                    ui::stat(
                        "Cost",
                        cost.map_or_else(|| "–".to_owned(), |price| price.to_string()),
                        cx,
                    ),
                ))
                .child(ui::stat(
                    "Last",
                    last.map_or_else(|| "–".to_owned(), |price| price.to_string()),
                    cx,
                )),
        );
        let mut rows: Vec<(i64, bool, Vec<AnyElement>)> = Vec::new();
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
                        ui::Align::Center,
                        move |this, cx| this.cancel(side, price, cx),
                        cx,
                    )
                } else {
                    ui::cell("")
                }
            };
            // Quantities are not coloured; the columns say which side.
            let size_cell = |side: Side, size: Option<i64>| {
                let name = match side {
                    Side::Buy => "bid",
                    Side::Sell => "ask",
                };
                Self::cell_button(
                    format!("ladder-{name}-{price}"),
                    size.map(|size| size.to_string()).unwrap_or_default(),
                    None,
                    match side {
                        Side::Buy => ui::Align::Right,
                        Side::Sell => ui::Align::Left,
                    },
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
            rows.push((
                price,
                quoted,
                vec![
                    mine_cell(Side::Buy, my_bid),
                    size_cell(Side::Buy, bid),
                    price_cell.into_any_element(),
                    size_cell(Side::Sell, ask),
                    mine_cell(Side::Sell, my_ask),
                ],
            ));
        }
        if let Some(problem) = &self.problem {
            body = body.child(
                div()
                    .text_size(ui::rem(ui::LABEL_SIZE))
                    .text_color(cx.theme().danger)
                    .child(problem.clone()),
            );
        }
        body.child(ui::responsive_table(&COLUMNS, move |fitted, _, cx| {
            let mut scroll = v_flex()
                .id("ladder-rows")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll();
            for (price, quoted, cells) in rows {
                let mut line = div().id(SharedString::from(format!("ladder-row-{price}")));
                if quoted {
                    line = line.bg(quote_bg);
                }
                scroll = scroll.child(line.child(ui::table_row(fitted, cells, cx)));
            }
            v_flex()
                .size_full()
                .child(ui::table_head(fitted, cx))
                .child(scroll)
        }))
    }
}
