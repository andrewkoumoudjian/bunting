//! RIT's book flashes: a new order or a grown level lights green for a
//! quarter of a second, a traded order or level lights red.
//!
//! Each Book Trader compares every venue's book with the one it last drew.
//! "Traded" is an order whose displayed quantity fell while it stayed in
//! the book (an order-by-order feed reports executions that way), or a
//! price where the venue's feed printed a trade since the last frame.

use bunting_trading_state::bunting_client::FeedBook;
use bunting_trading_state::bunting_client::bnp_wire::{Listing, Side};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How long a row stays lit.
pub const FLASH: Duration = Duration::from_millis(250);
/// Books remembered per panel; past this the memory starts again (nothing
/// flashes for one frame).
const MAX_BOOKS: usize = 64;

/// A lit row: one displayed order, or one price level.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Key {
    Order(u64),
    Level(Side, i64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flash {
    New,
    Traded,
}

#[derive(Default)]
struct Seen {
    orders: HashMap<u64, i64>,
    levels: HashMap<(Side, i64), i64>,
    trades: usize,
    last_trade: Option<(i64, i64)>,
}

/// One panel's memory of the books it drew and the rows lit now.
#[derive(Default)]
pub struct Flashes {
    seen: HashMap<Listing, Seen>,
    lit: HashMap<(Listing, Key), (Flash, Instant)>,
}

impl Flashes {
    /// Compares `book` with the last one drawn for `listing` and lights
    /// what changed. The first look at a book lights nothing.
    pub fn observe(&mut self, listing: Listing, book: &FeedBook, now: Instant) {
        self.lit
            .retain(|_, (_, at)| now.duration_since(*at) < FLASH);
        if self.seen.len() >= MAX_BOOKS && !self.seen.contains_key(&listing) {
            self.seen.clear();
        }
        let mut next = Seen {
            trades: book.trades().len(),
            last_trade: book.trades().last().copied(),
            ..Seen::default()
        };
        for side in [Side::Buy, Side::Sell] {
            for order in book.orders(side) {
                next.orders.insert(order.reference, order.quantity);
            }
        }
        for level in book.bids() {
            next.levels.insert((Side::Buy, level.price), level.quantity);
        }
        for level in book.asks() {
            next.levels
                .insert((Side::Sell, level.price), level.quantity);
        }
        if let Some(last) = self.seen.get(&listing) {
            let mut light = |key: Key, flash: Flash| {
                self.lit.insert((listing, key), (flash, now));
            };
            for (reference, quantity) in &next.orders {
                match last.orders.get(reference) {
                    None => light(Key::Order(*reference), Flash::New),
                    Some(before) if quantity < before => {
                        light(Key::Order(*reference), Flash::Traded);
                    }
                    Some(_) => {}
                }
            }
            for (&(side, price), quantity) in &next.levels {
                if last
                    .levels
                    .get(&(side, price))
                    .is_none_or(|before| quantity > before)
                {
                    light(Key::Level(side, price), Flash::New);
                }
            }
            for (price, _) in new_trades(last, book.trades()) {
                light(Key::Level(Side::Buy, price), Flash::Traded);
                light(Key::Level(Side::Sell, price), Flash::Traded);
            }
        }
        self.seen.insert(listing, next);
    }

    /// The flash on a row now, if any.
    #[must_use]
    pub fn get(&self, listing: Listing, key: Key, now: Instant) -> Option<Flash> {
        self.lit
            .get(&(listing, key))
            .filter(|(_, at)| now.duration_since(*at) < FLASH)
            .map(|(flash, _)| *flash)
    }

    /// When the next lit row goes out.
    #[must_use]
    pub fn next_change(&self, now: Instant) -> Option<Duration> {
        self.lit
            .values()
            .map(|(_, at)| FLASH.saturating_sub(now.duration_since(*at)))
            .min()
    }
}

/// The trades `book` printed since `last` saw it. The book keeps a bounded
/// list and drops the oldest first, so once full a new last trade counts
/// as one.
fn new_trades(last: &Seen, trades: &[(i64, i64)]) -> Vec<(i64, i64)> {
    let fresh = if trades.len() > last.trades {
        trades.len() - last.trades
    } else {
        usize::from(trades.last().copied() != last.last_trade && !trades.is_empty())
    };
    trades[trades.len() - fresh..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_trading_state::bunting_client::bnp_wire::{BookOrder, ServerMessage};

    const FAR: Listing = Listing {
        venue_id: 2,
        instrument_id: 7,
    };

    fn snapshot(orders: Vec<(u64, i64)>) -> FeedBook {
        let mut book = FeedBook::new(1);
        let applied = book.apply(&ServerMessage::OrderSnapshot {
            request_id: 1,
            listing: FAR,
            next_report: 1,
            complete: true,
            orders: orders
                .into_iter()
                .enumerate()
                .map(|(index, (reference, quantity))| BookOrder {
                    side: Side::Sell,
                    reference,
                    price: 102,
                    quantity,
                    broker: None,
                    queue_position: u32::try_from(index + 1).unwrap_or(u32::MAX),
                })
                .collect(),
        });
        assert!(applied.is_ok());
        book
    }

    #[test]
    fn new_orders_light_green_and_reduced_ones_red_for_a_quarter_second() {
        let mut flashes = Flashes::default();
        let start = Instant::now();
        flashes.observe(FAR, &snapshot(vec![(1, 3)]), start);
        // The first look lights nothing.
        assert_eq!(flashes.get(FAR, Key::Order(1), start), None);
        let later = start + Duration::from_millis(10);
        flashes.observe(FAR, &snapshot(vec![(1, 2), (2, 4)]), later);
        assert_eq!(flashes.get(FAR, Key::Order(1), later), Some(Flash::Traded));
        assert_eq!(flashes.get(FAR, Key::Order(2), later), Some(Flash::New));
        assert_eq!(
            flashes.get(FAR, Key::Level(Side::Sell, 102), later),
            Some(Flash::New)
        );
        assert_eq!(flashes.next_change(later), Some(FLASH));
        let out = later + FLASH;
        assert_eq!(flashes.get(FAR, Key::Order(2), out), None);
    }
}
