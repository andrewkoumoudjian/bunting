//! An instrument's consolidated tape (ADR 0036 §6): every venue's best bid
//! and offer and every venue's trades, as the processor at the hub
//! publishes them, numbered by one report sequence shared by every
//! subscriber.
//!
//! The tape is slower than a venue's direct feed for anyone near that venue;
//! a strategy that watches only the tape trades on older prices.

use crate::book::FeedError;
use bnp_wire::{EntryKind, Level, ServerMessage, Side};
use std::collections::BTreeMap;

/// Trades kept; older ones are dropped first.
const MAX_TRADES: usize = 1_024;

/// One trade on the tape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TapeTrade {
    pub venue_id: u128,
    pub price: i64,
    pub quantity: i64,
}

/// One tape subscription's view of an instrument.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TapeBook {
    request_id: u32,
    instrument_id: Option<u128>,
    /// Best bid and offer per venue.
    quotes: BTreeMap<(u128, Side), Level>,
    next_report: Option<u64>,
    trades: Vec<TapeTrade>,
}

impl TapeBook {
    #[must_use]
    pub fn new(request_id: u32) -> Self {
        Self {
            request_id,
            ..Self::default()
        }
    }

    /// Applies a tape message. Messages that are not this subscription's
    /// snapshot or update are ignored (`Ok(false)`).
    ///
    /// # Errors
    /// Returns a [`FeedError`] when an update cannot continue the tape:
    /// unsubscribe and subscribe again.
    pub fn apply(&mut self, message: &ServerMessage) -> Result<bool, FeedError> {
        match message {
            ServerMessage::TapeSnapshot {
                request_id,
                instrument_id,
                last_report,
                quotes,
            } if *request_id == self.request_id => {
                self.instrument_id = Some(*instrument_id);
                self.quotes = quotes
                    .iter()
                    .map(|quote| {
                        (
                            (quote.venue_id, quote.side),
                            Level {
                                price: quote.price,
                                quantity: quote.quantity,
                            },
                        )
                    })
                    .collect();
                self.next_report = Some(last_report.saturating_add(1));
                self.trades.clear();
                Ok(true)
            }
            ServerMessage::TapeUpdate {
                request_id,
                instrument_id,
                first_report,
                entries,
            } if *request_id == self.request_id => {
                let expected = self.next_report.ok_or(FeedError::NoSnapshot)?;
                if self.instrument_id != Some(*instrument_id) {
                    return Err(FeedError::WrongFeed);
                }
                if *first_report != expected {
                    return Err(FeedError::Gap {
                        expected,
                        received: *first_report,
                    });
                }
                for entry in entries {
                    let side = match entry.kind {
                        EntryKind::Trade => {
                            if self.trades.len() >= MAX_TRADES {
                                self.trades.remove(0);
                            }
                            self.trades.push(TapeTrade {
                                venue_id: entry.venue_id,
                                price: entry.price,
                                quantity: entry.quantity,
                            });
                            continue;
                        }
                        EntryKind::Bid => Side::Buy,
                        EntryKind::Ask => Side::Sell,
                    };
                    if entry.quantity == 0 {
                        self.quotes.remove(&(entry.venue_id, side));
                    } else {
                        self.quotes.insert(
                            (entry.venue_id, side),
                            Level {
                                price: entry.price,
                                quantity: entry.quantity,
                            },
                        );
                    }
                }
                let count = u64::try_from(entries.len()).unwrap_or(u64::MAX);
                self.next_report = Some(expected.saturating_add(count));
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.next_report.is_some()
    }

    #[must_use]
    pub const fn instrument_id(&self) -> Option<u128> {
        self.instrument_id
    }

    /// One venue's best bid (`Side::Buy`) or offer as the tape last showed it.
    #[must_use]
    pub fn quote(&self, venue_id: u128, side: Side) -> Option<Level> {
        self.quotes.get(&(venue_id, side)).copied()
    }

    /// Every venue's quote on one side, as `(venue_id, level)`, best first
    /// (venue ID breaks ties).
    #[must_use]
    pub fn quotes(&self, side: Side) -> Vec<(u128, Level)> {
        let mut quotes: Vec<(u128, Level)> = self
            .quotes
            .iter()
            .filter(|((_, quote_side), _)| *quote_side == side)
            .map(|(&(venue_id, _), &level)| (venue_id, level))
            .collect();
        quotes.sort_by_key(|(venue_id, level)| {
            (
                match side {
                    Side::Buy => i128::from(level.price).saturating_neg(),
                    Side::Sell => i128::from(level.price),
                },
                *venue_id,
            )
        });
        quotes
    }

    /// The best bid or offer across venues, with its venue.
    #[must_use]
    pub fn best(&self, side: Side) -> Option<(u128, Level)> {
        self.quotes(side).into_iter().next()
    }

    /// Trades seen since the snapshot, oldest first.
    #[must_use]
    pub fn trades(&self) -> &[TapeTrade] {
        &self.trades
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bnp_wire::{TapeEntry, TapeQuote};

    #[test]
    fn the_tape_tracks_each_venues_quote_and_trade() -> Result<(), FeedError> {
        let mut tape = TapeBook::new(3);
        tape.apply(&ServerMessage::TapeSnapshot {
            request_id: 3,
            instrument_id: 1,
            last_report: 10,
            quotes: vec![
                TapeQuote {
                    venue_id: 1,
                    side: Side::Sell,
                    price: 101,
                    quantity: 5,
                },
                TapeQuote {
                    venue_id: 2,
                    side: Side::Sell,
                    price: 100,
                    quantity: 1,
                },
            ],
        })?;
        assert_eq!(
            tape.best(Side::Sell),
            Some((
                2,
                Level {
                    price: 100,
                    quantity: 1
                }
            ))
        );
        tape.apply(&ServerMessage::TapeUpdate {
            request_id: 3,
            instrument_id: 1,
            first_report: 11,
            entries: vec![
                TapeEntry {
                    venue_id: 2,
                    kind: EntryKind::Trade,
                    price: 100,
                    quantity: 1,
                },
                TapeEntry {
                    venue_id: 2,
                    kind: EntryKind::Ask,
                    price: 100,
                    quantity: 0,
                },
            ],
        })?;
        assert_eq!(tape.best(Side::Sell).map(|(venue, _)| venue), Some(1));
        assert_eq!(
            tape.trades(),
            &[TapeTrade {
                venue_id: 2,
                price: 100,
                quantity: 1
            }]
        );
        assert_eq!(
            tape.apply(&ServerMessage::TapeUpdate {
                request_id: 3,
                instrument_id: 1,
                first_report: 14,
                entries: Vec::new(),
            }),
            Err(FeedError::Gap {
                expected: 13,
                received: 14
            })
        );
        Ok(())
    }
}
