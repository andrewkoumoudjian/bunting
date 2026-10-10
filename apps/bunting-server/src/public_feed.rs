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
//!
//! With 266=N the direct feed is order-by-order (L3): every displayed
//! order by its anonymous public reference (278) instead of price levels,
//! and each trade names the resting order it executed against.
//!
//! A subscription naming exchange 0 (207=0) is the consolidated tape of one
//! instrument instead (ADR 0036): every venue's best bid and offer and every
//! venue's trades, sent from the processor at the hub. Its snapshot states
//! the tape's last report sequence (83); the feed continues with the next.
//!
//! The feed logic is protocol-neutral; each session supplies how one
//! increment is encoded (FIX `X`, or a BNP market update).

use crate::consolidated::{ConsolidatedTape, TapeRecord};
use crate::distributor::{CommittedBatch, Subscription};
use crate::wake::Waker;
use bunting_admission_sequencer::Endpoint;
use bunting_application::{MarketDataEntryType, OrderAction, PublicListingUpdate};
use bunting_market_events::Side;
use bunting_market_types::{InstrumentId, ListingKey};
use simfix_mapping::{MarketDataIncrement, MarketDataUpdateAction, market_incremental};
use simfix_wire::FixMessage;
use std::sync::Arc;

/// Concurrent subscriptions one session may hold.
pub(crate) const MAX_FEEDS_PER_SESSION: usize = 32;
/// Batches buffered per feed while its snapshot is in flight.
pub(crate) const MAX_BUFFERED_BATCHES: usize = crate::distributor::MAX_PENDING_BATCHES;

/// Encodes one increment: request ID, the report sequence of its first
/// entry, and its entries with their listings.
pub(crate) type IncrementEncoder<M> = fn(&str, u64, &[(ListingKey, MarketDataIncrement)]) -> M;

/// One message ready to leave its source (a venue, or the tape's processor
/// at the hub) and the venue time it became available there.
pub(crate) struct FeedMessage<M = FixMessage> {
    pub(crate) source: Endpoint,
    pub(crate) available_us: u64,
    pub(crate) request_id: String,
    pub(crate) message: M,
}

/// What a direct feed shows of the book.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Detail {
    /// Aggregated price levels (L2).
    Levels,
    /// Every displayed order by public reference (L3, FIX 266=N).
    Orders,
}

struct Feed {
    request_id: String,
    listing_key: ListingKey,
    bids: bool,
    offers: bool,
    trades: bool,
    detail: Detail,
    /// Report sequence (FIX 83) of this feed's next entry.
    next_report: u64,
    /// `Some` until the snapshot's reply arrives.
    pending: Option<Vec<CommittedBatch>>,
    /// Latest send time handed out, so jitter never reorders one feed.
    last_send_us: u64,
}

/// One session's subscription to an instrument's consolidated tape.
struct TapeFeed {
    request_id: String,
    instrument_id: InstrumentId,
    /// `Some` until the snapshot's reply arrives.
    pending: Option<Vec<Arc<TapeRecord>>>,
    last_send_us: u64,
}

pub(crate) struct PublicFeeds<'a, M = FixMessage> {
    feeds: Vec<Feed>,
    tape_feeds: Vec<TapeFeed>,
    /// Held while any consolidated feed exists.
    tape: Option<Subscription<'a, TapeRecord>>,
    encode: IncrementEncoder<M>,
}

impl Default for PublicFeeds<'_, FixMessage> {
    fn default() -> Self {
        Self::new(market_incremental)
    }
}

impl PublicFeeds<'_, FixMessage> {
    /// [`PublicFeeds::activate_after`] for a FIX snapshot: a consolidated
    /// feed continues after the snapshot's last report sequence (83).
    pub(crate) fn activate(
        &mut self,
        request_id: &str,
        published_before: u64,
        snapshot: &[FixMessage],
    ) -> Vec<FeedMessage> {
        let last = snapshot
            .first()
            .and_then(|message| message.value(83))
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(0);
        self.activate_after(request_id, published_before, last)
    }
}

impl<'a, M> PublicFeeds<'a, M> {
    pub(crate) const fn new(encode: IncrementEncoder<M>) -> Self {
        Self {
            feeds: Vec::new(),
            tape_feeds: Vec::new(),
            tape: None,
            encode,
        }
    }

    /// Why a new subscription with this ID cannot start, if it cannot.
    fn refuse(&self, request_id: &str) -> Option<String> {
        if self.feeds.iter().any(|feed| feed.request_id == request_id)
            || self
                .tape_feeds
                .iter()
                .any(|feed| feed.request_id == request_id)
        {
            return Some(format!(
                "market data request {request_id} is already subscribed"
            ));
        }
        if self.feeds.len().saturating_add(self.tape_feeds.len()) >= MAX_FEEDS_PER_SESSION {
            return Some(format!(
                "max market data subscriptions {MAX_FEEDS_PER_SESSION}"
            ));
        }
        None
    }

    /// Registers a pending consolidated-tape subscription before its request
    /// is admitted. The session's tape subscription starts with its first
    /// consolidated feed, so no record applied after the snapshot is missed.
    pub(crate) fn subscribe_consolidated(
        &mut self,
        request_id: &str,
        instrument_id: InstrumentId,
        tape: &'a ConsolidatedTape,
        waker: &Waker,
    ) -> Option<String> {
        if let Some(reason) = self.refuse(request_id) {
            return Some(reason);
        }
        if self.tape.is_none() {
            match tape.subscribe(Some(waker.clone())) {
                Ok(subscription) => self.tape = Some(subscription),
                Err(reason) => return Some(reason),
            }
        }
        self.tape_feeds.push(TapeFeed {
            request_id: request_id.to_owned(),
            instrument_id,
            pending: Some(Vec::new()),
            last_send_us: 0,
        });
        None
    }

    /// Maps every tape record applied since the last call to each active
    /// consolidated feed of its instrument, and buffers it for pending ones.
    ///
    /// # Errors
    /// Returns an error when the tape disconnected this session or a pending
    /// feed's buffer is full.
    pub(crate) fn drain_tape(&mut self) -> Result<Vec<FeedMessage<M>>, String> {
        let Some(tape) = &self.tape else {
            return Ok(Vec::new());
        };
        let mut messages = Vec::new();
        for record in tape.drain()? {
            for feed in &mut self.tape_feeds {
                if feed.instrument_id != record.instrument_id {
                    continue;
                }
                match &mut feed.pending {
                    Some(buffer) => {
                        if buffer.len() >= MAX_BUFFERED_BATCHES {
                            return Err(format!(
                                "market data request {} buffered more than {MAX_BUFFERED_BATCHES} tape records",
                                feed.request_id
                            ));
                        }
                        buffer.push(record.clone());
                    }
                    None => messages.push(feed.message(&record, self.encode)),
                }
            }
        }
        Ok(messages)
    }

    /// Registers a pending subscription before its request is admitted, so
    /// no batch committed after the snapshot can be missed. Returns a
    /// rejection reason for a duplicate request ID, the session limit, or a
    /// request with no supported entry type.
    pub(crate) fn subscribe(
        &mut self,
        request_id: &str,
        listing_key: ListingKey,
        entry_types: &[MarketDataEntryType],
        aggregated: bool,
    ) -> Option<String> {
        if let Some(reason) = self.refuse(request_id) {
            return Some(reason);
        }
        let wants = |wanted| entry_types.contains(&wanted);
        let feed = Feed {
            request_id: request_id.to_owned(),
            listing_key,
            bids: wants(MarketDataEntryType::Bid),
            offers: wants(MarketDataEntryType::Offer),
            trades: wants(MarketDataEntryType::Trade),
            detail: if aggregated {
                Detail::Levels
            } else {
                Detail::Orders
            },
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
        let before = self.feeds.len().saturating_add(self.tape_feeds.len());
        self.feeds.retain(|feed| feed.request_id != request_id);
        self.tape_feeds.retain(|feed| feed.request_id != request_id);
        if self.tape_feeds.is_empty() {
            self.tape = None;
        }
        self.feeds.len().saturating_add(self.tape_feeds.len()) != before
    }

    /// The snapshot reply arrived: a venue feed continues with the first
    /// batch published after it, a consolidated feed with the first record
    /// after the snapshot's last report sequence `last`. Returns the
    /// buffered increments that follow.
    pub(crate) fn activate_after(
        &mut self,
        request_id: &str,
        published_before: u64,
        last: u64,
    ) -> Vec<FeedMessage<M>> {
        let encode = self.encode;
        if let Some(feed) = self
            .tape_feeds
            .iter_mut()
            .find(|feed| feed.request_id == request_id)
        {
            let buffered = feed.pending.take().unwrap_or_default();
            return buffered
                .iter()
                .filter(|record| record.first_report > last)
                .map(|record| feed.message(record, encode))
                .collect();
        }
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
            .filter_map(|batch| feed.increment(batch, encode))
            .collect()
    }

    /// Maps one committed batch to every active feed's increments and
    /// buffers it for pending ones.
    ///
    /// # Errors
    /// Returns an error when a pending feed's buffer is full; the session
    /// must disconnect rather than publish a feed with a hole in it.
    pub(crate) fn on_batch(
        &mut self,
        batch: &CommittedBatch,
    ) -> Result<Vec<FeedMessage<M>>, String> {
        let encode = self.encode;
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
                None => messages.extend(feed.increment(batch, encode)),
            }
        }
        Ok(messages)
    }

    /// Orders one feed's messages: a message never leaves before the
    /// previous one of the same feed, whatever the path's jitter.
    pub(crate) fn send_time(&mut self, request_id: &str, proposed_us: u64) -> u64 {
        let last = self
            .feeds
            .iter_mut()
            .find(|feed| feed.request_id == request_id)
            .map(|feed| &mut feed.last_send_us)
            .or_else(|| {
                self.tape_feeds
                    .iter_mut()
                    .find(|feed| feed.request_id == request_id)
                    .map(|feed| &mut feed.last_send_us)
            });
        last.map_or(proposed_us, |last| {
            *last = (*last).max(proposed_us);
            *last
        })
    }
}

impl TapeFeed {
    /// Every entry of a record: a consolidated feed always carries quotes
    /// and trades, so its report sequence is the tape's, with no gaps.
    fn message<M>(&self, record: &TapeRecord, encode: IncrementEncoder<M>) -> FeedMessage<M> {
        FeedMessage {
            source: Endpoint::Hub,
            available_us: record.hub_us,
            request_id: self.request_id.clone(),
            message: encode(&self.request_id, record.first_report, &record.entries),
        }
    }
}

impl Feed {
    fn increment<M>(
        &mut self,
        batch: &CommittedBatch,
        encode: IncrementEncoder<M>,
    ) -> Option<FeedMessage<M>> {
        let update = batch
            .public
            .iter()
            .find(|update| update.listing_key == self.listing_key)?;
        let entries = self.entries(update);
        if entries.is_empty() {
            return None;
        }
        let entries: Vec<_> = entries
            .into_iter()
            .map(|entry| (self.listing_key, entry))
            .collect();
        let message = encode(&self.request_id, self.next_report, &entries);
        self.next_report = self
            .next_report
            .saturating_add(u64::try_from(entries.len()).unwrap_or(u64::MAX));
        Some(FeedMessage {
            source: Endpoint::Venue(self.listing_key.venue_id),
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
                        reference: match self.detail {
                            Detail::Orders => trade.maker_reference,
                            Detail::Levels => None,
                        },
                        buyer: trade.buyer_broker,
                        seller: trade.seller_broker,
                    }),
            );
        }
        if self.detail == Detail::Orders {
            for change in &update.orders {
                let wanted = match change.side {
                    Side::Buy => self.bids,
                    Side::Sell => self.offers,
                };
                if wanted {
                    entries.push(MarketDataIncrement::Order {
                        action: match change.action {
                            OrderAction::Added => MarketDataUpdateAction::New,
                            OrderAction::Changed => MarketDataUpdateAction::Change,
                            OrderAction::Deleted => MarketDataUpdateAction::Delete,
                        },
                        side: change.side,
                        reference: change.reference,
                        price: change.price,
                        quantity: change.quantity,
                        broker: change.broker,
                    });
                }
            }
            return entries;
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
    use bunting_application::LevelChange;
    use bunting_market_types::{PriceTicks, QuantityLots, VenueId};

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
                best_bid: None,
                best_ask: None,
                orders: Vec::new(),
            }],
        })
    }

    const BOOK: &[MarketDataEntryType] = &[MarketDataEntryType::Bid, MarketDataEntryType::Offer];

    #[test]
    fn a_feed_resumes_exactly_after_its_snapshot() -> Result<(), String> {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK, true), None);
        // Committed before and after the snapshot, drained before its reply.
        assert!(feeds.on_batch(&batch(0, 1, 100, 0, 5))?.is_empty());
        assert!(feeds.on_batch(&batch(1, 1, 101, 0, 3))?.is_empty());
        let resumed = feeds.activate("a", 1, &[]);
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
            feeds.subscribe("bids", listing(1), &[MarketDataEntryType::Bid], true),
            None
        );
        assert_eq!(feeds.subscribe("venue2", listing(2), BOOK, true), None);
        feeds.activate("bids", 0, &[]);
        feeds.activate("venue2", 0, &[]);
        // An ask change on venue 1: neither feed wants it.
        assert!(feeds.on_batch(&batch(0, 1, 100, 0, 5))?.is_empty());
        let other = feeds.on_batch(&batch(1, 2, 100, 0, 5))?;
        assert_eq!(other.len(), 1);
        assert_eq!(other[0].source, Endpoint::Venue(VenueId::new(2)));
        Ok(())
    }

    #[test]
    fn an_order_by_order_feed_sends_orders_and_trade_references_not_levels() -> Result<(), String> {
        use bunting_application::{OrderChange, PublicTrade};
        use bunting_market_types::{EventSequence, LogicalTimeNs};
        let mut feeds = PublicFeeds::default();
        let all = [
            MarketDataEntryType::Bid,
            MarketDataEntryType::Offer,
            MarketDataEntryType::Trade,
        ];
        assert_eq!(feeds.subscribe("l2", listing(1), &all, true), None);
        assert_eq!(feeds.subscribe("l3", listing(1), &all, false), None);
        feeds.activate("l2", 0, &[]);
        feeds.activate("l3", 0, &[]);
        let mut committed = batch(0, 1, 100, 5, 4);
        let update = &mut Arc::get_mut(&mut committed).ok_or("shared batch")?.public[0];
        update.trades.push(PublicTrade {
            sequence: EventSequence::new(1),
            logical_time: LogicalTimeNs::new(0),
            instrument_id: InstrumentId::new(1),
            listing_key: listing(1),
            price: PriceTicks::new(100),
            quantity: QuantityLots::new(1),
            maker_reference: Some(7),
            buyer_broker: None,
            seller_broker: None,
        });
        update.orders.push(OrderChange {
            action: OrderAction::Changed,
            side: Side::Sell,
            reference: 7,
            price: PriceTicks::new(100),
            quantity: QuantityLots::new(4),
            broker: None,
        });
        let messages = feeds.on_batch(&committed)?;
        let message = |id| {
            messages
                .iter()
                .find(|message| message.request_id == id)
                .map(|message| &message.message)
                .ok_or(format!("no {id} message"))
        };
        let tags = |message: &FixMessage, tag| {
            message
                .fields
                .iter()
                .filter(|field| field.tag == tag)
                .map(|field| field.value.clone())
                .collect::<Vec<_>>()
        };
        assert!(
            tags(message("l2")?, 278).is_empty(),
            "levels carry no reference"
        );
        assert_eq!(tags(message("l2")?, 269), ["2", "1"]);
        assert_eq!(tags(message("l3")?, 278), ["7", "7"]);
        assert_eq!(tags(message("l3")?, 279), ["0", "1"]);
        assert_eq!(tags(message("l3")?, 271), ["1", "4"]);
        Ok(())
    }

    #[test]
    fn duplicate_ids_limits_and_unsubscribe() {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK, true), None);
        assert!(feeds.subscribe("a", listing(2), BOOK, true).is_some());
        assert!(feeds.subscribe("none", listing(1), &[], true).is_some());
        for index in 1..MAX_FEEDS_PER_SESSION {
            assert_eq!(
                feeds.subscribe(&index.to_string(), listing(1), BOOK, true),
                None
            );
        }
        assert!(feeds.subscribe("over", listing(1), BOOK, true).is_some());
        assert!(feeds.unsubscribe("a"));
        assert!(!feeds.unsubscribe("a"));
        assert_eq!(feeds.subscribe("over", listing(1), BOOK, true), None);
    }

    #[test]
    fn a_full_pending_buffer_is_an_error() -> Result<(), String> {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK, true), None);
        for ordinal in 0..u64::try_from(MAX_BUFFERED_BATCHES).map_err(|e| e.to_string())? {
            feeds.on_batch(&batch(ordinal, 1, 100, 0, 1))?;
        }
        assert!(feeds.on_batch(&batch(u64::MAX, 1, 100, 0, 1)).is_err());
        Ok(())
    }

    #[test]
    fn send_times_never_go_backwards_within_a_feed() {
        let mut feeds = PublicFeeds::default();
        assert_eq!(feeds.subscribe("a", listing(1), BOOK, true), None);
        assert_eq!(feeds.send_time("a", 50), 50);
        assert_eq!(feeds.send_time("a", 40), 50);
        assert_eq!(feeds.send_time("a", 60), 60);
    }
}
