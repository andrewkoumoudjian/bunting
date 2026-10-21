//! RIT's Portfolio: one row per listed security, whether or not it is
//! held, with the figures RIT shows in RIT's order and Bunting's open buy
//! and open sell after them. The rows are **derived** from committed
//! reports (your fills, the books, the trade feeds) and never feed back
//! into an order or any authority; the pane says so.
//!
//! Venues stay apart: Last, Bid and Ask each name the venue that
//! supplied them when the security is listed on several, and no book is
//! merged. Volume and VWAP count every venue's trades.

use crate::format;
use crate::ui::Col;
use bunting_trading_state::Model;
use bunting_trading_state::bunting_client::bnp_wire::Listing;
use std::cmp::Ordering;

/// RIT's columns, then Bunting's additions. The first is the key; the
/// widest figures are kept longest as the window narrows.
pub const COLUMNS: [Col; 16] = [
    Col::left("ticker", 64.).priority(2),
    Col::left("type", 48.).priority(0),
    Col::right("size", 48.).priority(0),
    Col::right("position", 72.).priority(2),
    Col::right("cost", 64.),
    Col::right("last", 88.).priority(2),
    Col::right("bid", 88.),
    Col::right("ask", 88.),
    Col::right("NLV", 80.),
    Col::right("realized", 80.),
    Col::right("unrealized", 88.),
    Col::right("VWAP", 64.).priority(0),
    Col::right("volume", 72.).priority(0),
    Col::right("open buy", 72.).priority(0),
    Col::right("open sell", 72.).priority(0),
    Col::right("fills since", 88.).priority(0),
];

/// Why the Type column is empty.
pub const TYPE_GAP: &str = "The venue does not send the instrument type yet (stock, future, option): BNP reference data gap";

/// A price and the venue that supplied it.
pub type Quote = Option<(i64, u128)>;

/// One security's row.
#[derive(Clone, Debug)]
pub struct Row {
    pub instrument_id: u128,
    pub symbol: String,
    /// The contract multiplier and whether the venue told us.
    pub contract_size: (i64, bool),
    /// `None` until the venue has answered the account request.
    pub position: Option<i64>,
    pub open_buy: Option<i64>,
    pub open_sell: Option<i64>,
    /// Fills seen since the last account answer, not yet in `position`.
    pub pending: i64,
    pub cost: Option<i64>,
    /// The last trade: price and the venue it printed on.
    pub last: Option<(i64, u128)>,
    /// Up-tick (`Greater`) or down-tick (`Less`) of the last price
    /// change on that venue; it holds until the price changes again.
    pub tick: Option<Ordering>,
    pub bid: Option<(i64, u128)>,
    pub ask: Option<(i64, u128)>,
    pub nlv: Option<i128>,
    pub realized: i128,
    pub unrealized: Option<i128>,
    pub vwap: Option<i64>,
    pub volume: i128,
    /// Whether the security is listed on more than one venue, so venue
    /// labels are needed.
    pub multi_venue: bool,
}

/// A price, with its venue when the security trades on several.
#[must_use]
pub fn priced(value: Option<(i64, u128)>, multi_venue: bool) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |(price, venue)| {
            if multi_venue {
                format!("{price} · V{venue}")
            } else {
                price.to_string()
            }
        },
    )
}

impl Row {
    /// The row's text, one entry per [`COLUMNS`] column. The table and
    /// "Copy to clipboard" both read this, so they always agree.
    #[must_use]
    pub fn texts(&self) -> Vec<String> {
        let dash = || "—".to_owned();
        let count = |value: Option<i64>| value.map_or_else(dash, |v| format::grouped(v.into()));
        let (multiplier, known) = self.contract_size;
        vec![
            self.symbol.clone(),
            dash(),
            if known {
                multiplier.to_string()
            } else {
                format!("{multiplier}*")
            },
            count(self.position),
            self.cost.map_or_else(dash, |cost| cost.to_string()),
            priced(self.last, self.multi_venue),
            priced(self.bid, self.multi_venue),
            priced(self.ask, self.multi_venue),
            self.nlv.map_or_else(dash, format::grouped),
            format::grouped_signed(self.realized),
            self.unrealized.map_or_else(dash, format::grouped_signed),
            self.vwap.map_or_else(dash, |vwap| vwap.to_string()),
            format::grouped(self.volume),
            count(self.open_buy),
            count(self.open_sell),
            if self.pending == 0 {
                String::new()
            } else {
                format!("{:+}", self.pending)
            },
        ]
    }
}

/// The last trade of `instrument_id` on any direct feed, and the tick
/// direction of the last price change on its venue.
fn last_trade(model: &Model, instrument_id: u128) -> (Option<(i64, u128)>, Option<Ordering>) {
    let mut newest: Option<(i64, Listing)> = None;
    for print in model
        .prints()
        .iter()
        .rev()
        .filter(|print| print.listing.instrument_id == instrument_id)
    {
        match newest {
            None => newest = Some((print.price, print.listing)),
            Some((price, listing)) => {
                if print.listing == listing && print.price != price {
                    return (
                        Some((price, listing.venue_id)),
                        Some(price.cmp(&print.price)),
                    );
                }
            }
        }
    }
    if let Some((price, listing)) = newest {
        return (Some((price, listing.venue_id)), None);
    }
    // Trades older than the print window: the series still has the last,
    // on the instrument's primary listing (venue 1 where listed).
    let listings: Vec<Listing> = model
        .listings()
        .iter()
        .map(|info| info.listing)
        .filter(|listing| listing.instrument_id == instrument_id)
        .collect();
    let primary = listings
        .iter()
        .copied()
        .find(|listing| listing.venue_id == 1)
        .or_else(|| listings.first().copied());
    (
        primary.and_then(|listing| {
            model
                .last_price(listing)
                .map(|price| (price, listing.venue_id))
        }),
        None,
    )
}

/// How many venues list `instrument_id`.
#[must_use]
pub fn venue_count(model: &Model, instrument_id: u128) -> usize {
    model
        .listings()
        .iter()
        .filter(|info| info.listing.instrument_id == instrument_id)
        .count()
}

/// The best bid and best ask for `instrument_id` across its venues'
/// direct feeds, each with the venue that supplied it. Books are never
/// merged: this picks one venue's level for each side.
#[must_use]
pub fn touch(model: &Model, instrument_id: u128) -> (Quote, Quote) {
    let mut bid: Option<(i64, u128)> = None;
    let mut ask: Option<(i64, u128)> = None;
    for info in model
        .listings()
        .iter()
        .filter(|info| info.listing.instrument_id == instrument_id)
    {
        let Some(feed) = model.feed(info.listing) else {
            continue;
        };
        if let Some(level) = feed.book.best_bid()
            && bid.is_none_or(|(price, _)| level.price > price)
        {
            bid = Some((level.price, info.listing.venue_id));
        }
        if let Some(level) = feed.book.best_ask()
            && ask.is_none_or(|(price, _)| level.price < price)
        {
            ask = Some((level.price, info.listing.venue_id));
        }
    }
    (bid, ask)
}

/// Every listed security's row, in listing order.
#[must_use]
pub fn rows(model: &Model) -> Vec<Row> {
    let account = model.account();
    model
        .instruments()
        .into_iter()
        .map(|instrument| {
            let id = instrument.instrument_id;
            let held = account.and_then(|account| {
                account
                    .positions
                    .iter()
                    .find(|position| position.instrument_id == id)
            });
            let position = account.map(|_| held.map_or(0, |held| held.position));
            let (bid, ask) = touch(model, id);
            let mut volume: i128 = 0;
            let mut weighted: i128 = 0;
            for info in &instrument.listings {
                let traded = model.market_volume(info.listing);
                if let Some(vwap) = model.market_vwap(info.listing) {
                    weighted = weighted.saturating_add(i128::from(vwap).saturating_mul(traded));
                }
                volume = volume.saturating_add(traded);
            }
            let vwap = (volume > 0)
                .then(|| weighted.saturating_add(volume / 2) / volume)
                .and_then(|vwap| i64::try_from(vwap).ok());
            let (last, tick) = last_trade(model, id);
            let contract_size = model.multiplier(id);
            // Closing now: sell into the bid when long, buy from the ask
            // when short. Flat is worth nothing.
            let nlv = position.and_then(|position| {
                let price = match position.cmp(&0) {
                    Ordering::Equal => return Some(0),
                    Ordering::Greater => bid,
                    Ordering::Less => ask,
                }?
                .0;
                Some(
                    i128::from(price)
                        .saturating_mul(i128::from(position))
                        .saturating_mul(i128::from(contract_size.0)),
                )
            });
            Row {
                instrument_id: id,
                symbol: instrument.symbol.clone(),
                contract_size,
                position,
                open_buy: account.map(|_| held.map_or(0, |held| held.open_buy)),
                open_sell: account.map(|_| held.map_or(0, |held| held.open_sell)),
                pending: model.unreported_position(id),
                cost: model.cost(id),
                last,
                tick,
                bid,
                ask,
                nlv,
                realized: model.realized_pnl(id),
                unrealized: model.unrealized_pnl(id),
                vwap,
                volume,
                multi_venue: instrument.listings.len() > 1,
            }
        })
        .collect()
}

/// The table as tab-separated text with a header row.
#[must_use]
pub fn copy_text(model: &Model) -> String {
    let header: Vec<&str> = COLUMNS.iter().map(|col| col.label).collect();
    let body: Vec<Vec<String>> = rows(model).iter().map(Row::texts).collect();
    crate::ui::tsv(&header, &body)
}
