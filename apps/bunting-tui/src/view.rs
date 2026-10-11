//! Draws the model. Rendering reads [`Model`] and [`Ui`] and changes
//! nothing, so it runs the same on a terminal and on a test buffer.

use crate::ui::{Field, Tab, TicketForm, Ui};
use bunting_trading_state::bunting_client::QueuedOrder;
use bunting_trading_state::bunting_client::bnp_wire::{Level, Liquidity, Listing, Side};
use bunting_trading_state::{
    Connection, Feed, FeedState, Instrument, Model, NoticeLevel, OrderStatus, TicketKind,
    TradePrint,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Cell, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap};

/// Everything one frame shows.
pub struct Screen<'a> {
    pub model: &'a Model,
    pub ui: &'a Ui,
    /// What the terminal connected to.
    pub label: &'a str,
    /// Microseconds since the terminal started.
    pub now_us: u64,
}

const BID: Style = Style::new().fg(Color::Green);
const ASK: Style = Style::new().fg(Color::Red);
const DIM: Style = Style::new().fg(Color::DarkGray);
const WARN: Style = Style::new().fg(Color::Yellow);
const BAD: Style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
const SELECTED: Style = Style::new().add_modifier(Modifier::REVERSED);
const TITLE: Style = Style::new().add_modifier(Modifier::BOLD);

pub fn render(frame: &mut Frame<'_>, screen: &Screen<'_>) {
    let [header, body, bottom, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(11),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    frame.render_widget(Paragraph::new(header_line(screen)), header);
    let [side, books, right] = Layout::horizontal([
        Constraint::Length(26),
        Constraint::Min(30),
        Constraint::Length(38),
    ])
    .areas(body);
    let [tape, trades] =
        Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(right);
    let instrument = screen.ui.instrument(screen.model);
    render_side(frame, screen, instrument.as_ref(), side);
    render_books(frame, screen, instrument.as_ref(), books);
    render_tape(frame, screen, instrument.as_ref(), tape);
    render_trades(frame, screen, trades);
    render_tabs(frame, screen, bottom);
    frame.render_widget(Paragraph::new(footer_line(screen)).style(DIM), footer);
    if let Some(form) = &screen.ui.ticket {
        render_ticket(frame, screen, form);
    } else if screen.ui.confirm_kill {
        render_confirm(frame, screen.model.kill_switch_active());
    } else if screen.ui.help {
        render_help(frame);
    }
}

/// `1234` µs as `1.23 ms`.
#[must_use]
pub fn duration(micros: u64) -> String {
    if micros < 1_000 {
        format!("{micros} µs")
    } else if micros < 1_000_000 {
        format!("{}.{:02} ms", micros / 1_000, (micros % 1_000) / 10)
    } else {
        format!(
            "{}.{:02} s",
            micros / 1_000_000,
            (micros % 1_000_000) / 10_000
        )
    }
}

/// Microseconds since start as `seconds.milliseconds`.
fn clock(micros: u64) -> String {
    format!("{}.{:03}", micros / 1_000_000, (micros % 1_000_000) / 1_000)
}

fn venue(listing: Listing) -> String {
    format!("V{}", listing.venue_id)
}

/// Broker identifiers of a trade, where the venue publishes them.
fn brokers(print: &TradePrint) -> String {
    match (print.buyer, print.seller) {
        (None, None) => String::new(),
        (buyer, seller) => format!("{}→{}", broker(buyer), broker(seller)),
    }
}

fn broker(id: Option<u128>) -> String {
    id.map_or_else(|| "?".to_owned(), |id| format!("b{id}"))
}

fn feed_state(state: Option<&FeedState>) -> &'static str {
    match state {
        Some(FeedState::Live) => "live",
        Some(FeedState::Requested) => "subscribing",
        Some(FeedState::Refused(_)) => "refused",
        None => "not subscribed",
    }
}

fn side_span(side: Side) -> Span<'static> {
    match side {
        Side::Buy => Span::styled("BUY", BID),
        Side::Sell => Span::styled("SELL", ASK),
    }
}

fn header_line(screen: &Screen<'_>) -> Line<'static> {
    let model = screen.model;
    let mut spans = vec![
        Span::styled(" BUNTING ", TITLE.add_modifier(Modifier::REVERSED)),
        Span::raw(format!(" {} ", screen.label)),
    ];
    match model.connection() {
        Connection::Idle => spans.push(Span::styled("○ idle", DIM)),
        Connection::Connecting { attempt } => {
            spans.push(Span::styled(
                format!("◌ connecting (attempt {attempt})"),
                WARN,
            ));
        }
        Connection::Connected { welcome, since_us } => spans.push(Span::styled(
            format!(
                "● connected · participant {} · run {} · {}",
                welcome.participant_id,
                welcome.run_id,
                duration(screen.now_us.saturating_sub(*since_us))
            ),
            BID,
        )),
        Connection::Disconnected { reason, .. } => {
            spans.push(Span::styled(format!("✕ disconnected: {reason}"), BAD));
        }
    }
    let trips = model.round_trips();
    if let (Some(last), Some(min), Some(mean)) = (trips.last_us, trips.min_us, trips.mean_us()) {
        spans.push(Span::raw(format!(
            " │ ping {} (min {} · avg {})",
            duration(last),
            duration(min),
            duration(mean)
        )));
    }
    // What the venue measured for this connection: published, never used
    // to order anything (ADR 0035).
    if let Some(one_way) = model.access_delay().and_then(|delay| delay.one_way_us) {
        spans.push(Span::raw(format!(
            " │ venue measures {} one way",
            duration(one_way)
        )));
    }
    spans.push(Span::raw(format!(" │ seq {}", model.last_sequence())));
    if model.kill_switch_active() {
        spans.push(Span::styled(" │ KILL SWITCH ACTIVE", BAD));
    }
    Line::from(spans)
}

fn footer_line(screen: &Screen<'_>) -> &'static str {
    if screen.ui.ticket.is_some() {
        " Tab/↑↓ field · ←→/space change · digits edit · Enter send · Esc close"
    } else {
        " b buy · s sell · ←→ venue · ↑↓ instrument · o orders/levels · [ ] order · c cancel · C cancel all · K kill switch · r reload · 1-4 tabs · ? help · q quit"
    }
}

fn render_side(
    frame: &mut Frame<'_>,
    screen: &Screen<'_>,
    instrument: Option<&Instrument>,
    area: Rect,
) {
    let [top, bottom] =
        Layout::vertical([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(area);
    let instruments = screen.model.instruments();
    let selected = instrument.map(|i| i.instrument_id);
    let items: Vec<ListItem<'_>> = instruments
        .iter()
        .map(|i| {
            let text = format!(
                "{} #{} · {} venue{}",
                i.symbol,
                i.instrument_id,
                i.listings.len(),
                if i.listings.len() == 1 { "" } else { "s" }
            );
            let item = ListItem::new(text);
            if Some(i.instrument_id) == selected {
                item.style(SELECTED)
            } else {
                item
            }
        })
        .collect();
    let empty = items.is_empty();
    let list = List::new(items).block(Block::bordered().title(" Instruments "));
    frame.render_widget(list, top);
    if empty {
        frame.render_widget(
            Paragraph::new("waiting for listings").style(DIM),
            inner(top),
        );
    }
    // Per venue: this client's measured order round trip and the feed.
    let mut lines = Vec::new();
    if let Some(instrument) = instrument {
        let chosen = screen.ui.listing(screen.model);
        for info in &instrument.listings {
            let marker = if Some(info.listing) == chosen {
                "▸ "
            } else {
                "  "
            };
            lines.push(Line::from(vec![
                Span::raw(marker),
                Span::styled(format!("Venue {}", info.listing.venue_id), TITLE),
            ]));
            let feed = match screen.model.feed(info.listing).map(|feed| &feed.state) {
                Some(FeedState::Live) => Span::styled("    feed live", BID),
                Some(FeedState::Requested) => Span::styled("    feed subscribing", WARN),
                Some(FeedState::Refused(reason)) => {
                    Span::styled(format!("    feed refused: {reason}"), BAD)
                }
                None => Span::styled("    no feed", DIM),
            };
            lines.push(Line::from(feed));
            lines.push(Line::from(
                match screen.model.venue_round_trips().get(&info.listing.venue_id) {
                    Some(trip) => Span::raw(format!(
                        "    order rt {} (min {})",
                        duration(trip.last_us),
                        duration(trip.min_us)
                    )),
                    None => Span::styled("    order rt: send one", DIM),
                },
            ));
        }
    }
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" Venues ")),
        bottom,
    );
}

fn render_books(
    frame: &mut Frame<'_>,
    screen: &Screen<'_>,
    instrument: Option<&Instrument>,
    area: Rect,
) {
    let Some(instrument) = instrument else {
        frame.render_widget(Block::bordered().title(" Books "), area);
        return;
    };
    let listings: Vec<Listing> = instrument
        .listings
        .iter()
        .take(4)
        .map(|info| info.listing)
        .collect();
    let columns = Layout::horizontal(vec![Constraint::Fill(1); listings.len()]).split(area);
    let chosen = screen.ui.listing(screen.model);
    for (listing, column) in listings.iter().zip(columns.iter()) {
        render_book(
            frame,
            screen,
            &instrument.symbol,
            *listing,
            Some(*listing) == chosen,
            *column,
        );
    }
}

fn render_book(
    frame: &mut Frame<'_>,
    screen: &Screen<'_>,
    symbol: &str,
    listing: Listing,
    chosen: bool,
    area: Rect,
) {
    let feed = screen.model.feed(listing);
    let by_order = screen.ui.order_by_order && feed.is_some_and(|feed| feed.order_by_order);
    let view = match (screen.ui.order_by_order, by_order) {
        (false, _) => "levels",
        (true, true) => "orders",
        (true, false) => "levels only",
    };
    let block = Block::bordered()
        .title(format!(
            " {symbol} · Venue {} · {} · {view} ",
            listing.venue_id,
            feed_state(feed.map(|feed| &feed.state))
        ))
        .border_style(if chosen {
            TITLE.fg(Color::Cyan)
        } else {
            Style::new()
        });
    let rows_available = usize::from(area.height.saturating_sub(4));
    let depth = (rows_available / 2).max(1);
    let mut rows = Vec::new();
    if let Some(feed) = feed {
        let (asks, bids) = if by_order {
            order_rows(feed, depth)
        } else {
            level_rows(screen.model, feed, listing, depth)
        };
        for _ in asks.len()..depth {
            rows.push(Row::new(vec![Cell::from("")]));
        }
        rows.extend(asks);
        let spread = match (feed.book.best_bid(), feed.book.best_ask()) {
            (Some(bid), Some(ask)) => format!("spread {}", ask.price - bid.price),
            _ => "one-sided".to_owned(),
        };
        rows.push(Row::new(vec![
            Cell::from(""),
            Cell::from(""),
            Cell::from(spread).style(DIM),
            Cell::from(""),
        ]));
        rows.extend(bids);
    }
    let first = if by_order { "queue" } else { "mine" };
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Fill(1),
            Constraint::Length(10),
            Constraint::Fill(1),
        ],
    )
    .header(Row::new(vec![first, "bid qty", "price", "ask qty"]).style(DIM))
    .block(block);
    frame.render_widget(table, area);
}

fn book_row(first: String, side: Side, price: i64, quantity: i64) -> Row<'static> {
    match side {
        Side::Buy => Row::new(vec![
            Cell::from(first),
            Cell::from(quantity.to_string()).style(BID),
            Cell::from(price.to_string()).style(BID),
            Cell::from(""),
        ]),
        Side::Sell => Row::new(vec![
            Cell::from(first),
            Cell::from(""),
            Cell::from(price.to_string()).style(ASK),
            Cell::from(quantity.to_string()).style(ASK),
        ]),
    }
}

/// Price levels, asks (worst shown first) then bids, with this
/// participant's own resting quantity at each price.
fn level_rows(
    model: &Model,
    feed: &Feed,
    listing: Listing,
    depth: usize,
) -> (Vec<Row<'static>>, Vec<Row<'static>>) {
    let row = |level: Level, side: Side| {
        let mine: i64 = model
            .live_orders()
            .filter(|row| {
                row.listing == listing && row.side == side && row.price == Some(level.price)
            })
            .map(|row| row.remaining)
            .sum();
        let mark = if mine > 0 {
            format!("{mine}*")
        } else {
            String::new()
        };
        book_row(mark, side, level.price, level.quantity)
    };
    let asks = feed.book.asks();
    let asks = asks
        .iter()
        .take(depth)
        .rev()
        .map(|&level| row(level, Side::Sell))
        .collect();
    let bids = feed
        .book
        .bids()
        .iter()
        .take(depth)
        .map(|&level| row(level, Side::Buy))
        .collect();
    (asks, bids)
}

/// Every displayed order, asks (worst shown first) then bids, each with its
/// place in its price's queue and its broker where the venue publishes it.
fn order_rows(feed: &Feed, depth: usize) -> (Vec<Row<'static>>, Vec<Row<'static>>) {
    let row = |order: &QueuedOrder| {
        let place = order.broker.map_or_else(
            || format!("{}", order.position_at_price),
            |broker| format!("{} b{broker}", order.position_at_price),
        );
        let style = if order.position_at_price == 1 {
            Style::new()
        } else {
            DIM
        };
        book_row(place, order.side, order.price, order.quantity).style(style)
    };
    let asks = feed.book.orders(Side::Sell);
    let asks = asks.iter().take(depth).rev().map(row).collect();
    let bids = feed
        .book
        .orders(Side::Buy)
        .iter()
        .take(depth)
        .map(row)
        .collect();
    (asks, bids)
}

/// The instrument's consolidated tape: each venue's best bid and offer and
/// every venue's trades as the processor at the hub published them.
fn render_tape(
    frame: &mut Frame<'_>,
    screen: &Screen<'_>,
    instrument: Option<&Instrument>,
    area: Rect,
) {
    let tape = instrument.and_then(|i| screen.model.tape(i.instrument_id));
    let block = Block::bordered().title(format!(
        " Tape from the hub · {} ",
        feed_state(tape.map(|tape| &tape.state))
    ));
    let inside = block.inner(area);
    frame.render_widget(block, area);
    let (Some(instrument), Some(tape)) = (instrument, tape) else {
        return;
    };
    let best_bid = tape.book.best(Side::Buy).map(|(venue, _)| venue);
    let best_ask = tape.book.best(Side::Sell).map(|(venue, _)| venue);
    let quote = |venue_id: u128, side: Side, best: Option<u128>| -> Cell<'static> {
        let Some(level) = tape.book.quote(venue_id, side) else {
            return Cell::from("—").style(DIM);
        };
        let style = match side {
            Side::Buy => BID,
            Side::Sell => ASK,
        };
        let style = if best == Some(venue_id) {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        };
        Cell::from(format!("{} × {}", level.quantity, level.price)).style(style)
    };
    let quotes: Vec<Row<'_>> = instrument
        .listings
        .iter()
        .map(|info| {
            let venue_id = info.listing.venue_id;
            Row::new(vec![
                Cell::from(venue(info.listing)),
                quote(venue_id, Side::Buy, best_bid),
                quote(venue_id, Side::Sell, best_ask),
            ])
        })
        .collect();
    let quote_height = u16::try_from(quotes.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let [top, bottom] =
        Layout::vertical([Constraint::Length(quote_height), Constraint::Min(0)]).areas(inside);
    frame.render_widget(
        Table::new(
            quotes,
            [
                Constraint::Length(4),
                Constraint::Fill(1),
                Constraint::Fill(1),
            ],
        )
        .header(Row::new(vec!["ven", "best bid", "best ask"]).style(DIM)),
        top,
    );
    let height = usize::from(bottom.height.saturating_sub(1));
    let rows: Vec<Row<'_>> = screen
        .model
        .tape_prints()
        .iter()
        .rev()
        .filter(|print| print.listing.instrument_id == instrument.instrument_id)
        .take(height)
        .map(|print| {
            Row::new(vec![
                Cell::from(clock(print.received_us)).style(DIM),
                Cell::from(venue(print.listing)),
                Cell::from(print.price.to_string()),
                Cell::from(print.quantity.to_string()),
            ])
        })
        .collect();
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Length(4),
                Constraint::Length(10),
                Constraint::Fill(1),
            ],
        )
        .header(Row::new(vec!["received", "ven", "price", "qty"]).style(DIM)),
        bottom,
    );
}

fn render_trades(frame: &mut Frame<'_>, screen: &Screen<'_>, area: Rect) {
    let height = usize::from(area.height.saturating_sub(3));
    let rows: Vec<Row<'_>> = screen
        .model
        .prints()
        .iter()
        .rev()
        .take(height)
        .map(|print| {
            Row::new(vec![
                Cell::from(clock(print.received_us)).style(DIM),
                Cell::from(venue(print.listing)),
                Cell::from(print.price.to_string()),
                Cell::from(print.quantity.to_string()),
                Cell::from(brokers(print)).style(DIM),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(4),
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Fill(1),
        ],
    )
    .header(Row::new(vec!["received", "ven", "price", "qty", "brokers"]).style(DIM))
    .block(Block::bordered().title(" Direct feeds, as they reached you "));
    frame.render_widget(table, area);
}

fn render_tabs(frame: &mut Frame<'_>, screen: &Screen<'_>, area: Rect) {
    let [tabs, content] = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    let live = screen.model.live_orders().count();
    let titles = Tab::ALL.iter().enumerate().map(|(index, tab)| {
        if *tab == Tab::Orders {
            format!("{} {} ({live} live)", index + 1, tab.title())
        } else {
            format!("{} {}", index + 1, tab.title())
        }
    });
    let selected = Tab::ALL
        .iter()
        .position(|tab| *tab == screen.ui.tab)
        .unwrap_or(0);
    frame.render_widget(
        Tabs::new(titles).select(selected).highlight_style(SELECTED),
        tabs,
    );
    match screen.ui.tab {
        Tab::Orders => render_orders(frame, screen, content),
        Tab::Fills => render_fills(frame, screen, content),
        Tab::Account => render_account(frame, screen, content),
        Tab::Log => render_log(frame, screen, content),
    }
}

fn render_orders(frame: &mut Frame<'_>, screen: &Screen<'_>, area: Rect) {
    let rows = Ui::order_rows(screen.model);
    let visible = usize::from(area.height.saturating_sub(3)).max(1);
    let selected = screen.ui.order.min(rows.len().saturating_sub(1));
    let first = selected.saturating_sub(visible.saturating_sub(1));
    let table_rows: Vec<Row<'_>> = rows
        .iter()
        .enumerate()
        .skip(first)
        .take(visible)
        .map(|(index, row)| {
            let status_style = match &row.status {
                OrderStatus::Rejected(_) => BAD,
                OrderStatus::Filled => BID,
                status if status.is_live() => Style::new(),
                _ => DIM,
            };
            let cells = Row::new(vec![
                Cell::from(venue(row.listing)),
                Cell::from(side_span(row.side)),
                Cell::from(row.quantity.to_string()),
                Cell::from(
                    row.price
                        .map_or_else(|| "MKT".to_owned(), |p| p.to_string()),
                ),
                Cell::from(row.filled.to_string()),
                Cell::from(row.remaining.to_string()),
                Cell::from(row.status.label()).style(status_style),
                Cell::from(row.answer_us.map_or_else(String::new, duration)),
                Cell::from(if row.client_order_id == 0 {
                    "(FIX)".to_owned()
                } else {
                    row.client_order_id.to_string()
                })
                .style(DIM),
            ]);
            if index == selected {
                cells.style(SELECTED)
            } else {
                cells
            }
        })
        .collect();
    let table = Table::new(
        table_rows,
        [
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(7),
            Constraint::Length(7),
            Constraint::Fill(2),
            Constraint::Length(10),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(vec![
            "ven",
            "side",
            "qty",
            "price",
            "filled",
            "left",
            "status",
            "answer",
            "client id",
        ])
        .style(DIM),
    )
    .block(Block::bordered());
    frame.render_widget(table, area);
}

fn render_fills(frame: &mut Frame<'_>, screen: &Screen<'_>, area: Rect) {
    let visible = usize::from(area.height.saturating_sub(3));
    let rows: Vec<Row<'_>> = screen
        .model
        .fills()
        .iter()
        .rev()
        .take(visible)
        .map(|fill| {
            Row::new(vec![
                Cell::from(clock(fill.received_us)).style(DIM),
                Cell::from(venue(fill.listing)),
                Cell::from(side_span(fill.side)),
                Cell::from(fill.quantity.to_string()),
                Cell::from(fill.price.to_string()),
                Cell::from(fill.fee.to_string()),
                Cell::from(match fill.liquidity {
                    Liquidity::Maker => "maker",
                    Liquidity::Taker => "taker",
                }),
                Cell::from(fill.sequence.to_string()).style(DIM),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Length(4),
            Constraint::Length(5),
            Constraint::Length(7),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(vec![
            "received", "ven", "side", "qty", "price", "fee", "liq", "seq",
        ])
        .style(DIM),
    )
    .block(Block::bordered());
    frame.render_widget(table, area);
}

fn render_account(frame: &mut Frame<'_>, screen: &Screen<'_>, area: Rect) {
    let Some(account) = screen.model.account() else {
        frame.render_widget(
            Paragraph::new("no account answer yet")
                .style(DIM)
                .block(Block::bordered()),
            area,
        );
        return;
    };
    let [cash, positions] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(area);
    let title = format!(
        " Cash · as of seq {} · {} ago ",
        account.run_sequence,
        duration(screen.now_us.saturating_sub(account.received_us))
    );
    let cash_rows: Vec<Row<'_>> = account
        .cash
        .iter()
        .map(|balance| {
            Row::new(vec![
                balance.currency_id.to_string(),
                balance.balance.to_string(),
                balance.reserved.to_string(),
            ])
        })
        .collect();
    frame.render_widget(
        Table::new(
            cash_rows,
            [
                Constraint::Length(8),
                Constraint::Fill(1),
                Constraint::Fill(1),
            ],
        )
        .header(Row::new(vec!["ccy", "balance", "reserved"]).style(DIM))
        .block(Block::bordered().title(title)),
        cash,
    );
    let position_rows: Vec<Row<'_>> = account
        .positions
        .iter()
        .map(|position| {
            let pending = screen.model.unreported_position(position.instrument_id);
            Row::new(vec![
                Cell::from(
                    screen
                        .model
                        .instruments()
                        .iter()
                        .find(|i| i.instrument_id == position.instrument_id)
                        .map_or_else(
                            || format!("#{}", position.instrument_id),
                            |i| i.symbol.clone(),
                        ),
                ),
                Cell::from(position.position.to_string()),
                Cell::from(position.open_buy.to_string()).style(BID),
                Cell::from(position.open_sell.to_string()).style(ASK),
                Cell::from(if pending == 0 {
                    String::new()
                } else {
                    format!("{pending:+} since")
                })
                .style(WARN),
            ])
        })
        .collect();
    frame.render_widget(
        Table::new(
            position_rows,
            [
                Constraint::Length(8),
                Constraint::Length(10),
                Constraint::Length(9),
                Constraint::Length(9),
                Constraint::Fill(1),
            ],
        )
        .header(Row::new(vec!["inst", "position", "open buy", "open sell", "fills"]).style(DIM))
        .block(Block::bordered().title(" Positions ")),
        positions,
    );
}

fn render_log(frame: &mut Frame<'_>, screen: &Screen<'_>, area: Rect) {
    let visible = usize::from(area.height.saturating_sub(2));
    let notices = screen.model.notices();
    let lines: Vec<Line<'_>> = notices
        .iter()
        .skip(notices.len().saturating_sub(visible))
        .map(|notice| {
            Line::from(vec![
                Span::styled(format!("{} ", clock(notice.at_us)), DIM),
                Span::styled(
                    notice.text.clone(),
                    match notice.level {
                        NoticeLevel::Info => Style::new(),
                        NoticeLevel::Warning => WARN,
                        NoticeLevel::Error => BAD,
                    },
                ),
            ])
        })
        .collect();
    frame.render_widget(Paragraph::new(lines).block(Block::bordered()), area);
}

fn render_ticket(frame: &mut Frame<'_>, screen: &Screen<'_>, form: &TicketForm) {
    let area = centered(frame.area(), 52, 16);
    frame.render_widget(Clear, area);
    let ticket = &form.ticket;
    let venue_text = ticket.listing.map_or_else(
        || "none".to_owned(),
        |listing| {
            let trip = screen
                .model
                .venue_round_trips()
                .get(&listing.venue_id)
                .map(|trip| format!(" (order rt {})", duration(trip.last_us)))
                .unwrap_or_default();
            format!("Venue {}{trip}", listing.venue_id)
        },
    );
    let on = |flag: bool| if flag { "yes" } else { "no" }.to_owned();
    let value = |field: Field| -> String {
        match field {
            Field::Venue => venue_text.clone(),
            Field::Side => match ticket.side {
                Side::Buy => "BUY".to_owned(),
                Side::Sell => "SELL".to_owned(),
            },
            Field::Quantity => form.quantity.clone(),
            Field::Kind => match ticket.kind {
                TicketKind::Limit => "limit".to_owned(),
                TicketKind::Market => "market (IOC)".to_owned(),
            },
            Field::Price => match ticket.kind {
                TicketKind::Limit => form.price.clone(),
                TicketKind::Market => "—".to_owned(),
            },
            Field::Tif => ticket.tif.label().to_owned(),
            Field::PostOnly => on(ticket.post_only),
            Field::Anonymous => on(ticket.anonymous),
            Field::Display => {
                if form.display.is_empty() {
                    "all".to_owned()
                } else {
                    form.display.clone()
                }
            }
        }
    };
    let mut lines: Vec<Line<'_>> = Field::ALL
        .iter()
        .map(|&field| {
            let style = if field == form.field {
                SELECTED
            } else {
                Style::new()
            };
            Line::from(vec![
                Span::styled(format!(" {:<14}", field.label()), DIM),
                Span::styled(format!(" {} ", value(field)), style),
            ])
        })
        .collect();
    lines.push(Line::raw(""));
    match &form.error {
        Some(error) => lines.push(Line::styled(format!(" {error}"), BAD)),
        None => lines.push(Line::styled(
            " Enter sends to the chosen venue; it travels that venue's distance.",
            DIM,
        )),
    }
    let title = match ticket.side {
        Side::Buy => Span::styled(" New BUY order ", BID.add_modifier(Modifier::BOLD)),
        Side::Sell => Span::styled(" New SELL order ", ASK.add_modifier(Modifier::BOLD)),
    };
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::bordered().title(title)),
        area,
    );
}

fn render_confirm(frame: &mut Frame<'_>, active: bool) {
    let area = centered(frame.area(), 54, 6);
    frame.render_widget(Clear, area);
    if active {
        frame.render_widget(
            Paragraph::new(vec![
                Line::raw(" The kill switch is active. Accept new orders"),
                Line::raw(" again?"),
                Line::raw(""),
                Line::styled(" y to reset it · any other key keeps it on", WARN),
            ])
            .block(Block::bordered().title(Span::styled(" Kill switch ", BAD))),
            area,
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(vec![
            Line::raw(" Cancel every live order on every venue,"),
            Line::raw(" including FIX ones, and refuse new orders?"),
            Line::raw(""),
            Line::styled(
                " y to activate the kill switch · any other key keeps trading",
                WARN,
            ),
        ])
        .block(Block::bordered().title(Span::styled(" Kill switch ", BAD))),
        area,
    );
}

fn render_help(frame: &mut Frame<'_>) {
    let area = centered(frame.area(), 70, 21);
    frame.render_widget(Clear, area);
    let text = [
        " Each venue lists the instrument separately and sits at its own",
        " distance. An order waits that distance before the venue sees it,",
        " and its reports and the venue's feed come back the same way.",
        "",
        " Books: one direct feed per venue. 'mine' is your own resting",
        " quantity at a price; o shows every order with its queue place.",
        " Tape: each venue's best bid and offer and all trades, published",
        " at the hub; near a venue it lags that venue's direct feed.",
        " Direct feeds: every venue's trades in the order they reached you.",
        " Order rt: time from sending an order to the venue's answer.",
        " Ping: round trip to the venue host without virtual distance.",
        " Venue measures: the one-way delay the venue measured for you.",
        "",
        " b/s ticket · ←→ venue · ↑↓ instrument · o orders/levels",
        " [ ] select order · c cancel · C cancel all · r reload",
        " K kill switch (again to reset it)",
        " 1-4 or Tab: orders, fills, account, log · q quit",
        "",
        " any key closes this help",
    ];
    frame.render_widget(
        Paragraph::new(text.iter().map(|line| Line::raw(*line)).collect::<Vec<_>>())
            .block(Block::bordered().title(" Help ")),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

#[cfg(test)]
mod tests;
