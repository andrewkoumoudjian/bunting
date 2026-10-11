//! A venue's visible book rebuilt from one feed: its snapshot, then every
//! update in report order. A price-level feed gives levels; an
//! order-by-order feed (`FeedFlags::ORDERS`) gives every displayed order,
//! from which the levels and each order's place in its queue follow.

use bnp_wire::{
    BookOrder, Entry, EntryKind, Level, Listing, OrderAction, OrderEntry, ServerMessage, Side,
};
use std::collections::BTreeMap;

/// Why a feed message could not be applied. After any of these the book is
/// stale: unsubscribe and subscribe again for a fresh snapshot.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeedError {
    /// An update arrived before the feed's snapshot.
    NoSnapshot,
    /// Entries were missed: the update starts at `received`, not `expected`.
    Gap { expected: u64, received: u64 },
    /// The message is for another feed or listing.
    WrongFeed,
}

impl std::fmt::Display for FeedError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSnapshot => formatter.write_str("update before snapshot"),
            Self::Gap { expected, received } => {
                write!(
                    formatter,
                    "feed gap: expected report {expected}, received {received}"
                )
            }
            Self::WrongFeed => formatter.write_str("message belongs to another feed"),
        }
    }
}

impl std::error::Error for FeedError {}

/// One feed's view of a listing's visible depth, plus the trades it saw.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FeedBook {
    request_id: u32,
    listing: Option<Listing>,
    bids: BTreeMap<i64, i64>,
    asks: BTreeMap<i64, i64>,
    next_report: Option<u64>,
    trades: Vec<(i64, i64)>,
    /// Order-by-order feeds: displayed orders by public reference.
    orders: BTreeMap<u64, BookOrder>,
    /// An order snapshot's parts are still arriving.
    assembling: bool,
}

/// One displayed order on an order-by-order feed, with its place in its
/// side's queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct QueuedOrder {
    pub reference: u64,
    pub side: Side,
    pub price: i64,
    pub quantity: i64,
    /// The owner's broker identifier on venues that publish them.
    pub broker: Option<u128>,
    /// Displayed quantity ahead of this order at its price.
    pub quantity_ahead: i64,
    /// 1-based place at its price.
    pub position_at_price: usize,
}

/// Trades kept per book; older ones are dropped first.
const MAX_TRADES: usize = 1_024;

impl FeedBook {
    #[must_use]
    pub fn new(request_id: u32) -> Self {
        Self {
            request_id,
            ..Self::default()
        }
    }

    /// Applies a feed message. Messages that are not this feed's snapshot
    /// or update are ignored (`Ok(false)`).
    ///
    /// # Errors
    /// Returns a [`FeedError`] when the update cannot continue the book.
    pub fn apply(&mut self, message: &ServerMessage) -> Result<bool, FeedError> {
        match message {
            ServerMessage::MarketSnapshot {
                request_id,
                listing,
                next_report,
                bids,
                asks,
            } if *request_id == self.request_id => {
                self.listing = Some(*listing);
                self.bids = bids
                    .iter()
                    .map(|level| (level.price, level.quantity))
                    .collect();
                self.asks = asks
                    .iter()
                    .map(|level| (level.price, level.quantity))
                    .collect();
                self.next_report = Some(*next_report);
                self.trades.clear();
                self.orders.clear();
                self.assembling = false;
                Ok(true)
            }
            ServerMessage::MarketUpdate {
                request_id,
                listing,
                first_report,
                entries,
            } if *request_id == self.request_id => {
                self.continues(*listing, *first_report, entries.len())?;
                for entry in entries {
                    self.apply_entry(*entry);
                }
                Ok(true)
            }
            ServerMessage::OrderSnapshot {
                request_id,
                listing,
                next_report,
                complete,
                orders,
            } if *request_id == self.request_id => {
                if !self.assembling || self.listing != Some(*listing) {
                    self.listing = Some(*listing);
                    self.orders.clear();
                    self.bids.clear();
                    self.asks.clear();
                    self.trades.clear();
                    self.next_report = None;
                }
                for order in orders {
                    self.add_order(*order);
                }
                self.assembling = !complete;
                if *complete {
                    self.next_report = Some(*next_report);
                }
                Ok(true)
            }
            ServerMessage::OrderUpdate {
                request_id,
                listing,
                first_report,
                entries,
            } if *request_id == self.request_id => {
                self.continues(*listing, *first_report, entries.len())?;
                for entry in entries {
                    self.apply_order_entry(*entry);
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Checks that an update continues the feed and advances its report
    /// sequence past it.
    fn continues(
        &mut self,
        listing: Listing,
        first_report: u64,
        entries: usize,
    ) -> Result<(), FeedError> {
        let expected = self.next_report.ok_or(FeedError::NoSnapshot)?;
        if self.listing != Some(listing) {
            return Err(FeedError::WrongFeed);
        }
        if first_report != expected {
            return Err(FeedError::Gap {
                expected,
                received: first_report,
            });
        }
        let count = u64::try_from(entries).unwrap_or(u64::MAX);
        self.next_report = Some(expected.saturating_add(count));
        Ok(())
    }

    fn record_trade(&mut self, price: i64, quantity: i64) {
        if self.trades.len() >= MAX_TRADES {
            self.trades.remove(0);
        }
        self.trades.push((price, quantity));
    }

    fn level_mut(&mut self, side: Side) -> &mut BTreeMap<i64, i64> {
        match side {
            Side::Buy => &mut self.bids,
            Side::Sell => &mut self.asks,
        }
    }

    /// Adds `delta` to the level at `price`, removing it at zero.
    fn adjust_level(&mut self, side: Side, price: i64, delta: i64) {
        let levels = self.level_mut(side);
        let quantity = levels.entry(price).or_insert(0);
        *quantity = quantity.saturating_add(delta);
        if *quantity <= 0 {
            levels.remove(&price);
        }
    }

    fn add_order(&mut self, order: BookOrder) {
        if let Some(previous) = self.orders.insert(order.reference, order) {
            self.adjust_level(previous.side, previous.price, -previous.quantity);
        }
        self.adjust_level(order.side, order.price, order.quantity);
    }

    fn apply_order_entry(&mut self, entry: OrderEntry) {
        match entry {
            OrderEntry::Trade {
                price, quantity, ..
            } => self.record_trade(price, quantity),
            OrderEntry::Order {
                action: OrderAction::Delete,
                reference,
                ..
            } => {
                if let Some(order) = self.orders.remove(&reference) {
                    self.adjust_level(order.side, order.price, -order.quantity);
                }
            }
            OrderEntry::Order {
                side,
                reference,
                price,
                quantity,
                broker,
                ..
            } => self.add_order(BookOrder {
                side,
                reference,
                price,
                quantity,
                broker,
                queue_position: 0,
            }),
        }
    }

    fn apply_entry(&mut self, entry: Entry) {
        let side = match entry.kind {
            EntryKind::Trade => {
                self.record_trade(entry.price, entry.quantity);
                return;
            }
            EntryKind::Bid => &mut self.bids,
            EntryKind::Ask => &mut self.asks,
        };
        if entry.quantity == 0 {
            side.remove(&entry.price);
        } else {
            side.insert(entry.price, entry.quantity);
        }
    }

    /// Whether the snapshot has arrived.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.next_report.is_some()
    }

    #[must_use]
    pub const fn listing(&self) -> Option<Listing> {
        self.listing
    }

    /// Bids, best (highest) first.
    #[must_use]
    pub fn bids(&self) -> Vec<Level> {
        self.bids
            .iter()
            .rev()
            .map(|(&price, &quantity)| Level { price, quantity })
            .collect()
    }

    /// Asks, best (lowest) first.
    #[must_use]
    pub fn asks(&self) -> Vec<Level> {
        self.asks
            .iter()
            .map(|(&price, &quantity)| Level { price, quantity })
            .collect()
    }

    #[must_use]
    pub fn best_bid(&self) -> Option<Level> {
        self.bids
            .iter()
            .next_back()
            .map(|(&price, &quantity)| Level { price, quantity })
    }

    #[must_use]
    pub fn best_ask(&self) -> Option<Level> {
        self.asks
            .iter()
            .next()
            .map(|(&price, &quantity)| Level { price, quantity })
    }

    /// Trades seen since the snapshot, oldest first, as `(price, quantity)`.
    #[must_use]
    pub fn trades(&self) -> &[(i64, i64)] {
        &self.trades
    }

    /// Displayed orders on one side in matching priority (best price first,
    /// then time priority), each with its place in its price's queue. Empty
    /// for a price-level feed.
    #[must_use]
    pub fn orders(&self, side: Side) -> Vec<QueuedOrder> {
        let mut orders: Vec<&BookOrder> = self
            .orders
            .values()
            .filter(|order| order.side == side)
            .collect();
        // A public reference is the order's time priority in the book.
        orders.sort_by_key(|order| {
            (
                match side {
                    Side::Buy => i128::from(order.price).saturating_neg(),
                    Side::Sell => i128::from(order.price),
                },
                order.reference,
            )
        });
        let mut queued = Vec::with_capacity(orders.len());
        let mut price = None;
        let (mut ahead, mut position) = (0_i64, 0_usize);
        for order in orders {
            if price != Some(order.price) {
                price = Some(order.price);
                ahead = 0;
                position = 0;
            }
            position += 1;
            queued.push(QueuedOrder {
                reference: order.reference,
                side: order.side,
                price: order.price,
                quantity: order.quantity,
                broker: order.broker,
                quantity_ahead: ahead,
                position_at_price: position,
            });
            ahead = ahead.saturating_add(order.quantity);
        }
        queued
    }

    /// One displayed order and its place in its queue.
    #[must_use]
    pub fn order(&self, reference: u64) -> Option<QueuedOrder> {
        let side = self.orders.get(&reference)?.side;
        self.orders(side)
            .into_iter()
            .find(|order| order.reference == reference)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: Listing = Listing {
        venue_id: 1,
        instrument_id: 1,
    };

    fn update(first_report: u64, entries: Vec<Entry>) -> ServerMessage {
        ServerMessage::MarketUpdate {
            request_id: 7,
            listing: LISTING,
            first_report,
            entries,
        }
    }

    fn entry(kind: EntryKind, price: i64, quantity: i64) -> Entry {
        Entry {
            kind,
            price,
            quantity,
        }
    }

    fn book_order(side: Side, reference: u64, price: i64, quantity: i64) -> BookOrder {
        BookOrder {
            side,
            reference,
            price,
            quantity,
            broker: None,
            queue_position: 0,
        }
    }

    fn order_entry(
        action: OrderAction,
        side: Side,
        reference: u64,
        price: i64,
        quantity: i64,
    ) -> OrderEntry {
        OrderEntry::Order {
            action,
            side,
            reference,
            price,
            quantity,
            broker: None,
        }
    }

    #[test]
    fn an_order_by_order_feed_gives_levels_and_queue_places() -> Result<(), FeedError> {
        let mut book = FeedBook::new(7);
        // The snapshot arrives in two parts; the book is ready after the last.
        book.apply(&ServerMessage::OrderSnapshot {
            request_id: 7,
            listing: LISTING,
            next_report: 1,
            complete: false,
            orders: vec![
                book_order(Side::Buy, 3, 99, 5),
                book_order(Side::Buy, 8, 99, 2),
            ],
        })?;
        assert!(!book.is_ready());
        book.apply(&ServerMessage::OrderSnapshot {
            request_id: 7,
            listing: LISTING,
            next_report: 1,
            complete: true,
            orders: vec![book_order(Side::Sell, 5, 101, 4)],
        })?;
        assert!(book.is_ready());
        assert_eq!(
            book.best_bid(),
            Some(Level {
                price: 99,
                quantity: 7
            })
        );
        let second = book.order(8).ok_or(FeedError::NoSnapshot)?;
        assert_eq!((second.position_at_price, second.quantity_ahead), (2, 5));
        // A trade against reference 3 reduces it, then it leaves; 8 moves up.
        book.apply(&ServerMessage::OrderUpdate {
            request_id: 7,
            listing: LISTING,
            first_report: 1,
            entries: vec![
                OrderEntry::Trade {
                    price: 99,
                    quantity: 5,
                    resting_reference: Some(3),
                    buyer: None,
                    seller: None,
                },
                order_entry(OrderAction::Delete, Side::Buy, 3, 99, 0),
                order_entry(OrderAction::Add, Side::Buy, 11, 100, 1),
                order_entry(OrderAction::Change, Side::Sell, 5, 101, 1),
            ],
        })?;
        let moved = book.order(8).ok_or(FeedError::NoSnapshot)?;
        assert_eq!((moved.position_at_price, moved.quantity_ahead), (1, 0));
        assert_eq!(
            book.bids(),
            vec![
                Level {
                    price: 100,
                    quantity: 1
                },
                Level {
                    price: 99,
                    quantity: 2
                }
            ]
        );
        assert_eq!(
            book.best_ask(),
            Some(Level {
                price: 101,
                quantity: 1
            })
        );
        assert_eq!(book.trades(), &[(99, 5)]);
        assert_eq!(
            book.orders(Side::Buy)
                .iter()
                .map(|order| order.reference)
                .collect::<Vec<_>>(),
            vec![11, 8]
        );
        assert_eq!(
            book.apply(&ServerMessage::OrderUpdate {
                request_id: 7,
                listing: LISTING,
                first_report: 9,
                entries: Vec::new(),
            }),
            Err(FeedError::Gap {
                expected: 5,
                received: 9
            })
        );
        Ok(())
    }

    #[test]
    fn snapshot_then_updates_rebuild_the_book() -> Result<(), FeedError> {
        let mut book = FeedBook::new(7);
        assert_eq!(
            book.apply(&update(1, Vec::new())),
            Err(FeedError::NoSnapshot)
        );
        assert!(book.apply(&ServerMessage::MarketSnapshot {
            request_id: 7,
            listing: LISTING,
            next_report: 1,
            bids: vec![Level {
                price: 99,
                quantity: 5,
            }],
            asks: vec![Level {
                price: 101,
                quantity: 2,
            }],
        })?);
        book.apply(&update(
            1,
            vec![
                entry(EntryKind::Trade, 101, 2),
                entry(EntryKind::Ask, 101, 0),
                entry(EntryKind::Bid, 100, 3),
            ],
        ))?;
        assert_eq!(book.best_ask(), None);
        assert_eq!(
            book.best_bid(),
            Some(Level {
                price: 100,
                quantity: 3
            })
        );
        assert_eq!(book.bids().len(), 2);
        assert_eq!(book.trades(), &[(101, 2)]);
        // Reports 1..=3 were applied; 5 means 4 was missed.
        assert_eq!(
            book.apply(&update(5, Vec::new())),
            Err(FeedError::Gap {
                expected: 4,
                received: 5
            })
        );
        // Another feed's messages are not this book's.
        assert_eq!(
            book.apply(&ServerMessage::MarketUpdate {
                request_id: 8,
                listing: LISTING,
                first_report: 1,
                entries: Vec::new(),
            }),
            Ok(false)
        );
        Ok(())
    }
}
