//! Per-venue public market-data subscriptions of one FIX session.
//!
//! A subscription (FIX `V`, 263=1) is a direct venue feed: a full-depth
//! snapshot taken at the venue when the request arrives there, then every
//! later commit's anonymous trades and visible depth changes for that
//! listing (`X`), each sent from the venue over the team's virtual path.
//! Everything published is derived from committed state; no order,
//! participant or command identity crosses into a feed.
//!
//! The snapshot is taken on the sequencer thread between commits; its reply
//! carries how many batches were published before it, so the feed resumes at
//! exactly the next batch. Batches drained before the reply arrives are
//! buffered (bounded) until then.

use crate::distributor::CommittedBatch;
use bunting_application::{MarketDataEntryType, PublicListingUpdate};
use bunting_market_events::Side;
use bunting_market_types::{ListingKey, VenueId};
use simfix_mapping::{MarketDataIncrement, MarketDataUpdateAction, market_incremental};
use simfix_wire::FixMessage;

/// Concurrent subscriptions one session may hold.
pub(crate) const MAX_FEEDS_PER_SESSION: usize = 32;
/// Batches buffered per feed while its snapshot is in flight.
pub(crate) const MAX_BUFFERED_BATCHES: usize = crate::distributor::MAX_PENDING_BATCHES;

/// One message ready to leave the venue: its venue, the venue time it
/// became available, and the earliest send time the feed's order allows.
pub(crate) struct FeedMessage {
    pub(crate) venue_id: VenueId,
    pub(crate) available_us: u64,
    pub(crate) request_id: String,
    pub(crate) message: FixMessage,
}

struct Feed {
    request_id: String,
    listing_key: ListingKey,
    bids: bool,
    offers: bool,
    trades: bool,
    /// Report sequence (FIX 83) of this feed's next entry.
    next_report: u64,
    /// `Some` until the snapshot's reply arrives.
    pending: Option<Vec<CommittedBatch>>,
    /// Latest send time handed out, so jitter never reorders one feed.
    last_send_us: u64,
}

#[derive(Default)]
pub(crate) struct PublicFeeds {
    feeds: Vec<Feed>,
}

impl PublicFeeds {
    /// Registers a pending subscription before its request is admitted, so
    /// no batch committed after the snapshot can be missed. Returns a
    /// rejection reason for a duplicate request ID, the session limit, or a
    /// request with no supported entry type.
    pub(crate) fn subscribe(
        &mut self,
        request_id: &str,
        listing_key: ListingKey,
        entry_types: &[MarketDataEntryType],
    ) -> Option<String> {
        if self.feeds.iter().any(|feed| feed.request_id == request_id) {
            return Some(format!(
                "market data request {request_id} is already subscribed"
            ));
        }
        if self.feeds.len() >= MAX_FEEDS_PER_SESSION {
            return Some(format!(
                "max market data subscriptions {MAX_FEEDS_PER_SESSION}"
            ));
        }
        let wants = |wanted| entry_types.contains(&wanted);
        let feed = Feed {
            request_id: request_id.to_owned(),
            listing_key,
            bids: wants(MarketDataEntryType::Bid),
            offers: wants(MarketDataEntryType::Offer),
            trades: wants(MarketDataEntryType::Trade),
            next_report: 1,
            pending: Some(Vec::new()),
            last_send_us: 0,
        };
        if !(feed.bids || feed.offers || feed.trades) {
            return Some("market data subscription names no entry type".to_owned());
        }
        self.feeds.push(feed);
        None
    }

    /// Stops a subscription; returns whether one with this request ID existed.
    pub(crate) fn unsubscribe(&mut self, request_id: &str) -> bool {
        let before = self.feeds.len();
        self.feeds.retain(|feed| feed.request_id != request_id);
        self.feeds.len() != before
    }

    /// The snapshot reply arrived: the feed continues with the first batch
    /// published after it. Returns the buffered increments that follow.
    pub(crate) fn activate(&mut self, request_id: &str, published_before: u64) -> Vec<FeedMessage> {
        let Some(feed) = self
            .feeds
            .iter_mut()
            .find(|feed| feed.request_id == request_id)
        else {
            return Vec::new();
        };
        let buffered = feed.pending.take().unwrap_or_default();
        buffered
            .iter()
            .filter(|batch| batch.ordinal >= published_before)
            .filter_map(|batch| feed.increment(batch))
            .collect()
    }

    /// Maps one committed batch to every active feed's increments and
    /// buffers it for pending ones.
    ///
    /// # Errors
    /// Returns an error when a pending feed's buffer is full; the session
    /// must disconnect rather than publish a feed with a hole in it.
    pub(crate) fn on_batch(&mut self, batch: &CommittedBatch) -> Result<Vec<FeedMessage>, String> {
        let mut messages = Vec::new();
        for feed in &mut self.feeds {
            if !batch
                .public
                .iter()
                .any(|update| update.listing_key == feed.listing_key)
            {
                continue;
            }
            match &mut feed.pending {
                Some(buffer) => {
                    if buffer.len() >= MAX_BUFFERED_BATCHES {
                        return Err(format!(
                            "market data request {} buffered more than {MAX_BUFFERED_BATCHES} batches",
                            feed.request_id
                        ));
                    }
                    buffer.push(batch.clone());
                }
                None => messages.extend(feed.increment(batch)),
            }
        }
        Ok(messages)
    }

    /// Orders one feed's messages: a message never leaves before the
    /// previous one of the same feed, whatever the path's jitter.
    pub(crate) fn send_time(&mut self, request_id: &str, proposed_us: u64) -> u64 {
        self.feeds
            .iter_mut()
            .find(|feed| feed.request_id == request_id)
            .map_or(proposed_us, |feed| {
                feed.last_send_us = feed.last_send_us.max(proposed_us);
                feed.last_send_us
            })
    }
}

impl Feed {
    fn increment(&mut self, batch: &CommittedBatch) -> Option<FeedMessage> {
        let update = batch
            .public
            .iter()
            .find(|update| update.listing_key == self.listing_key)?;
        let entries = self.entries(update);
        if entries.is_empty() {
            return None;
        }
        let message = market_incremental(
            &self.request_id,
            self.listing_key,
            self.next_report,
            &entries,
        );
        self.next_report = self
            .next_report
            .saturating_add(u64::try_from(entries.len()).unwrap_or(u64::MAX));
        Some(FeedMessage {
            venue_id: self.listing_key.venue_id,
            available_us: batch.durable_us,
            request_id: self.request_id.clone(),
            message,
        })
    }

    fn entries(&self, update: &PublicListingUpdate) -> Vec<MarketDataIncrement> {
        let mut entries = Vec::new();
        if self.trades {
            entries.extend(
                update
                    .trades
                    .iter()
                    .map(|trade| MarketDataIncrement::Trade {
                        price: trade.price,
                        quantity: trade.quantity,
                    }),
            );
        }
        for change in &update.levels {
            let wanted = match change.side {
                Side::Buy => self.bids,
                Side::Sell => self.offers,
            };
            if !wanted {
                continue;
            }
            entries.push(MarketDataIncrement::Level {
                action: if change.quantity.get() == 0 {
                    MarketDataUpdateAction::Delete
                } else if change.previous.get() == 0 {
                    MarketDataUpdateAction::New
                } else {
                    MarketDataUpdateAction::Change
                },
                side: change.side,
                price: change.price,
                // Absolute resulting quantity: zero for a removed level.
                quantity: change.quantity,
            });
        }
        entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::distributor::Committed;
    use bunting_admission_sequencer::Endpoint;
    use bunting_application::LevelChange;
    use bunting_market_types::{InstrumentId, PriceTicks, QuantityLots};
    use std::sync::Arc;

    fn listing(venue: u128) -> ListingKey {
        ListingKey::new(VenueId::new(venue), InstrumentId::new(1))
    }

    fn batch(
        ordinal: u64,
        venue: u128,
        price: i64,
        previous: i64,
        quantity: i64,
    ) -> CommittedBatch {
        Arc::new(Committed {
            durable_us: ordinal.saturating_mul(10),
            source: Endpoint::Venue(VenueId::new(venue)),
            events: Vec::new(),
            ordinal,
            public: vec![PublicListingUpdate {
                listing_key: listing(venue),
                trades: Vec::new(),
                levels: vec![LevelChange {
                    side: Side::Sell,
                    price: PriceTicks::new(price),
                    previous: QuantityLots::new(previous),
                    quantity: QuantityLots::new(quantity),
                }],
            }],
        })
    }

    const BOOK: &[MarketDataEntryType] = &[MarketDataEntryType::Bid, MarketDataEntryType::Offer];

    #[test]
    fn a_feed_resumes_exactly_after_its_snapshot() -> Result<(), String> {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK), None);
        // Committed before and after the snapshot, drained before its reply.
        assert!(feeds.on_batch(&batch(0, 1, 100, 0, 5))?.is_empty());
        assert!(feeds.on_batch(&batch(1, 1, 101, 0, 3))?.is_empty());
        let resumed = feeds.activate("a", 1);
        assert_eq!(resumed.len(), 1);
        assert_eq!(resumed[0].message.value(270), Some("101"));
        assert_eq!(resumed[0].message.value(279), Some("0"));
        assert_eq!(resumed[0].message.value(83), Some("1"));
        let next = feeds.on_batch(&batch(2, 1, 101, 3, 0))?;
        assert_eq!(next[0].message.value(279), Some("2"));
        assert_eq!(next[0].message.value(271), Some("0"));
        assert_eq!(next[0].message.value(83), Some("2"));
        Ok(())
    }

    #[test]
    fn feeds_see_only_their_venue_and_requested_sides() -> Result<(), String> {
        let mut feeds = PublicFeeds::default();
        assert_eq!(
            feeds.subscribe("bids", listing(1), &[MarketDataEntryType::Bid]),
            None
        );
        assert_eq!(feeds.subscribe("venue2", listing(2), BOOK), None);
        feeds.activate("bids", 0);
        feeds.activate("venue2", 0);
        // An ask change on venue 1: neither feed wants it.
        assert!(feeds.on_batch(&batch(0, 1, 100, 0, 5))?.is_empty());
        let other = feeds.on_batch(&batch(1, 2, 100, 0, 5))?;
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].venue_id, VenueId::new(2));
        Ok(())
    }

    #[test]
    fn duplicate_ids_limits_and_unsubscribe() {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK), None);
        assert!(feeds.subscribe("a", listing(2), BOOK).is_some());
        assert!(feeds.subscribe("none", listing(1), &[]).is_some());
        for index in 1..MAX_FEEDS_PER_SESSION {
            assert_eq!(feeds.subscribe(&index.to_string(), listing(1), BOOK), None);
        }
        assert!(feeds.subscribe("over", listing(1), BOOK).is_some());
        assert!(feeds.unsubscribe("a"));
        assert!(!feeds.unsubscribe("a"));
        assert_eq!(feeds.subscribe("over", listing(1), BOOK), None);
    }

    #[test]
    fn a_full_pending_buffer_is_an_error() -> Result<(), String> {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK), None);
        for ordinal in 0..u64::try_from(MAX_BUFFERED_BATCHES).map_err(|e| e.to_string())? {
            feeds.on_batch(&batch(ordinal, 1, 100, 0, 1))?;
        }
        assert!(feeds.on_batch(&batch(u64::MAX, 1, 100, 0, 1)).is_err());
        Ok(())
    }

    #[test]
    fn send_times_never_go_backwards_within_a_feed() {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK), None);
        assert_eq!(feeds.send_time("a", 50), 50);
        assert_eq!(feeds.send_time("a", 40), 50);
        assert_eq!(feeds.send_time("a", 60), 60);
    }
}
