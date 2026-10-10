//! A venue's visible book rebuilt from one feed: its snapshot, then every
//! update in report order.

use bnp_wire::{Entry, EntryKind, Level, Listing, ServerMessage};
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
                Ok(true)
            }
            ServerMessage::MarketUpdate {
                request_id,
                listing,
                first_report,
                entries,
            } if *request_id == self.request_id => {
                let expected = self.next_report.ok_or(FeedError::NoSnapshot)?;
                if self.listing != Some(*listing) {
                    return Err(FeedError::WrongFeed);
                }
                if *first_report != expected {
                    return Err(FeedError::Gap {
                        expected,
                        received: *first_report,
                    });
                }
                for entry in entries {
                    self.apply_entry(*entry);
                }
                let count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
                self.next_report = Some(expected.saturating_add(count));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn apply_entry(&mut self, entry: Entry) {
        let side = match entry.kind {
            EntryKind::Trade => {
                if self.trades.len() >= MAX_TRADES {
                    self.trades.remove(0);
                }
                self.trades.push((entry.price, entry.quantity));
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
