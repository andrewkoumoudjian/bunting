//! BNP public feeds over the protocol-neutral feed publisher
//! (`public_feed.rs`): price-level and order-by-order direct venue feeds,
//! and an instrument's consolidated tape (ADR 0036 §5–§7).
//!
//! The publisher hands out increments as listing-tagged entries; this module
//! remembers what each subscription is and encodes its increments as
//! `MarketUpdate`, `OrderUpdate` or `TapeUpdate`. Timing is the publisher's
//! (a venue feed leaves its venue after commit, the tape leaves the
//! processor at the hub), so BNP and FIX subscribers see the same feed.

use crate::admission::JobWork;
use crate::consolidated::ConsolidatedTape;
use crate::distributor::{CommittedBatch, PublishingOrigin};
use crate::public_feed::{FeedMessage, PublicFeeds};
use crate::wake::Waker;
use bnp_wire::{
    BookOrder, Entry, EntryKind, FeedFlags, Level, Listing, OrderAction, OrderEntry, ServerMessage,
    TapeEntry, TapeQuote,
};
use bunting_application::bnp::listing_key;
use bunting_application::{
    ApplicationService, MarketDataEntryType, displayed_orders, project_market,
};
use bunting_market_events::Side;
use bunting_market_types::{InstrumentId, ListingKey, ParticipantId, RunId};
use simfix_mapping::{MarketDataIncrement, MarketDataUpdateAction};
use std::collections::BTreeMap;

/// One increment as the publisher produced it, before BNP encoding.
pub(crate) struct RawIncrement {
    first_report: u64,
    entries: Vec<(ListingKey, MarketDataIncrement)>,
}

fn raw(
    _request_id: &str,
    first_report: u64,
    entries: &[(ListingKey, MarketDataIncrement)],
) -> RawIncrement {
    RawIncrement {
        first_report,
        entries: entries.to_vec(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Levels(Listing),
    Orders(Listing),
    Tape(u128),
}

/// One BNP session's public subscriptions.
pub(crate) struct BnpFeeds<'a> {
    public: PublicFeeds<'a, RawIncrement>,
    kinds: BTreeMap<u32, Kind>,
}

impl<'a> BnpFeeds<'a> {
    pub(crate) fn new() -> Self {
        Self {
            public: PublicFeeds::new(raw),
            kinds: BTreeMap::new(),
        }
    }

    /// Registers a direct venue feed before its request is admitted; returns
    /// why it is refused, if it is. With [`FeedFlags::ORDERS`] the feed is
    /// order by order and the venue starts computing order-level views.
    pub(crate) fn subscribe_venue(
        &mut self,
        request_id: u32,
        listing: Listing,
        flags: FeedFlags,
        origin: &PublishingOrigin,
    ) -> Option<String> {
        let mut entry_types = Vec::new();
        for (flag, entry) in [
            (FeedFlags::BIDS, MarketDataEntryType::Bid),
            (FeedFlags::OFFERS, MarketDataEntryType::Offer),
            (FeedFlags::TRADES, MarketDataEntryType::Trade),
        ] {
            if flags.contains(flag) {
                entry_types.push(entry);
            }
        }
        let orders = flags.contains(FeedFlags::ORDERS);
        if orders {
            origin.enable_order_feeds();
        }
        let refused = self.public.subscribe(
            &request_id.to_string(),
            listing_key(listing),
            &entry_types,
            !orders,
        );
        if refused.is_none() {
            self.kinds.insert(
                request_id,
                if orders {
                    Kind::Orders(listing)
                } else {
                    Kind::Levels(listing)
                },
            );
        }
        refused
    }

    /// Registers a consolidated-tape feed of one instrument before its
    /// request is admitted.
    pub(crate) fn subscribe_tape(
        &mut self,
        request_id: u32,
        instrument_id: u128,
        tape: &'a ConsolidatedTape,
        waker: &Waker,
    ) -> Option<String> {
        let refused = self.public.subscribe_consolidated(
            &request_id.to_string(),
            InstrumentId::new(instrument_id),
            tape,
            waker,
        );
        if refused.is_none() {
            self.kinds.insert(request_id, Kind::Tape(instrument_id));
        }
        refused
    }

    pub(crate) fn unsubscribe(&mut self, request_id: &str) -> bool {
        if let Ok(id) = request_id.parse() {
            self.kinds.remove(&id);
        }
        self.public.unsubscribe(request_id)
    }

    pub(crate) fn send_time(&mut self, request_id: &str, proposed_us: u64) -> u64 {
        self.public.send_time(request_id, proposed_us)
    }

    /// The snapshot reply arrived: returns the buffered increments that
    /// follow it. A tape feed continues after the snapshot's last report.
    pub(crate) fn activate(
        &mut self,
        request_id: &str,
        published_before: u64,
        snapshot: &[ServerMessage],
    ) -> Vec<FeedMessage<ServerMessage>> {
        let last = snapshot
            .iter()
            .find_map(|message| match message {
                ServerMessage::TapeSnapshot { last_report, .. } => Some(*last_report),
                _ => None,
            })
            .unwrap_or(0);
        let increments = self
            .public
            .activate_after(request_id, published_before, last);
        self.encode_all(increments)
    }

    /// One committed batch's increments for every venue feed.
    pub(crate) fn on_batch(
        &mut self,
        batch: &CommittedBatch,
    ) -> Result<Vec<FeedMessage<ServerMessage>>, String> {
        let increments = self.public.on_batch(batch)?;
        Ok(self.encode_all(increments))
    }

    /// Every tape record applied since the last call, for every tape feed.
    pub(crate) fn drain_tape(&mut self) -> Result<Vec<FeedMessage<ServerMessage>>, String> {
        let increments = self.public.drain_tape()?;
        Ok(self.encode_all(increments))
    }

    fn encode_all(
        &self,
        increments: Vec<FeedMessage<RawIncrement>>,
    ) -> Vec<FeedMessage<ServerMessage>> {
        increments
            .into_iter()
            .filter_map(|increment| {
                let request_id: u32 = increment.request_id.parse().ok()?;
                let kind = *self.kinds.get(&request_id)?;
                Some(FeedMessage {
                    source: increment.source,
                    available_us: increment.available_us,
                    message: encode(request_id, kind, &increment.message),
                    request_id: increment.request_id,
                })
            })
            .collect()
    }
}

fn broker(participant: Option<ParticipantId>) -> Option<u128> {
    participant.map(ParticipantId::get)
}

const fn wire_side(side: Side) -> bnp_wire::Side {
    match side {
        Side::Buy => bnp_wire::Side::Buy,
        Side::Sell => bnp_wire::Side::Sell,
    }
}

fn encode(request_id: u32, kind: Kind, increment: &RawIncrement) -> ServerMessage {
    let first_report = increment.first_report;
    let entries = increment.entries.iter().map(|(key, entry)| (*key, *entry));
    match kind {
        Kind::Levels(listing) => ServerMessage::MarketUpdate {
            request_id,
            listing,
            first_report,
            entries: entries
                .filter_map(|(_, entry)| level_entry(entry))
                .collect(),
        },
        Kind::Orders(listing) => ServerMessage::OrderUpdate {
            request_id,
            listing,
            first_report,
            entries: entries
                .filter_map(|(_, entry)| order_entry(entry))
                .collect(),
        },
        Kind::Tape(instrument_id) => ServerMessage::TapeUpdate {
            request_id,
            instrument_id,
            first_report,
            entries: entries
                .filter_map(|(key, entry)| {
                    level_entry(entry).map(|entry| TapeEntry {
                        venue_id: key.venue_id.get(),
                        kind: entry.kind,
                        price: entry.price,
                        quantity: entry.quantity,
                    })
                })
                .collect(),
        },
    }
}

/// A trade or price level; for a level, the visible quantity after the
/// change (zero when it is gone).
fn level_entry(entry: MarketDataIncrement) -> Option<Entry> {
    match entry {
        MarketDataIncrement::Trade {
            price, quantity, ..
        } => Some(Entry {
            kind: EntryKind::Trade,
            price: price.get(),
            quantity: quantity.get(),
        }),
        MarketDataIncrement::Level {
            side,
            price,
            quantity,
            ..
        } => Some(Entry {
            kind: match side {
                Side::Buy => EntryKind::Bid,
                Side::Sell => EntryKind::Ask,
            },
            price: price.get(),
            quantity: quantity.get(),
        }),
        MarketDataIncrement::Order { .. } => None,
    }
}

fn order_entry(entry: MarketDataIncrement) -> Option<OrderEntry> {
    match entry {
        MarketDataIncrement::Trade {
            price,
            quantity,
            reference,
            buyer,
            seller,
        } => Some(OrderEntry::Trade {
            price: price.get(),
            quantity: quantity.get(),
            resting_reference: reference,
            buyer: broker(buyer),
            seller: broker(seller),
        }),
        MarketDataIncrement::Order {
            action,
            side,
            reference,
            price,
            quantity,
            broker: owner,
        } => Some(OrderEntry::Order {
            action: match action {
                MarketDataUpdateAction::New => OrderAction::Add,
                MarketDataUpdateAction::Change => OrderAction::Change,
                MarketDataUpdateAction::Delete => OrderAction::Delete,
            },
            side: wire_side(side),
            reference,
            price: price.get(),
            quantity: quantity.get(),
            broker: broker(owner),
        }),
        MarketDataIncrement::Level { .. } => None,
    }
}

fn unknown_listing(request_type: u8, request_id: u32) -> ServerMessage {
    ServerMessage::Reject {
        request_type,
        reference: u64::from(request_id),
        reason: "unknown listing".to_owned(),
    }
}

/// A full or depth-limited price-level snapshot of one listing, taken at its
/// venue. `next_report` is 1 for a feed's starting snapshot and 0 for a
/// one-shot.
pub(crate) fn level_snapshot_job(
    request_id: u32,
    listing: Listing,
    depth: usize,
    next_report: u64,
    run_id: RunId,
    request_type: u8,
) -> JobWork<ServerMessage> {
    Box::new(move |context| {
        let projection = ApplicationService::new(context.origin)
            .read(run_id, |state| project_market(state, listing_key(listing)))
            .map_err(|error| format!("run read failed: {error}"))?;
        let levels = |levels: &[(i64, i64)]| {
            levels
                .iter()
                .take(depth)
                .map(|&(price, quantity)| Level { price, quantity })
                .collect()
        };
        Ok(vec![match projection {
            Ok(projection) => ServerMessage::MarketSnapshot {
                request_id,
                listing,
                next_report,
                bids: levels(&projection.bids),
                asks: levels(&projection.asks),
            },
            Err(_) => unknown_listing(request_type, request_id),
        }])
    })
}

/// Encoded bytes of an `OrderSnapshot` before its orders, plus the frame's
/// length prefix, rounded up.
const ORDER_SNAPSHOT_HEADER_BYTES: usize = 64;
/// Largest encoded `BookOrder`.
const BOOK_ORDER_BYTES: usize = 46;

/// Orders per `OrderSnapshot` part that fit in a frame of `max_frame_bytes`.
pub(crate) fn orders_per_part(max_frame_bytes: usize) -> usize {
    (max_frame_bytes.saturating_sub(ORDER_SNAPSHOT_HEADER_BYTES) / BOOK_ORDER_BYTES)
        .clamp(1, bnp_wire::MAX_GROUP_ENTRIES)
}

/// An order-by-order feed's starting book, taken at the listing's venue: every
/// displayed order with its public reference and queue position, split into
/// parts of at most `per_part` orders.
pub(crate) fn order_snapshot_job(
    request_id: u32,
    listing: Listing,
    per_part: usize,
    run_id: RunId,
    request_type: u8,
) -> JobWork<ServerMessage> {
    Box::new(move |context| {
        let orders = ApplicationService::new(context.origin)
            .read(run_id, |state| {
                displayed_orders(state, listing_key(listing))
            })
            .map_err(|error| format!("run read failed: {error}"))?;
        let Ok(orders) = orders else {
            return Ok(vec![unknown_listing(request_type, request_id)]);
        };
        let mut position = [0_u32; 2];
        let orders: Vec<BookOrder> = orders
            .iter()
            .map(|order| {
                let place = &mut position[usize::from(order.side == Side::Sell)];
                *place = place.saturating_add(1);
                BookOrder {
                    side: wire_side(order.side),
                    reference: order.reference,
                    price: order.price.get(),
                    quantity: order.quantity.get(),
                    broker: broker(order.broker),
                    queue_position: *place,
                }
            })
            .collect();
        let parts: Vec<&[BookOrder]> = if orders.is_empty() {
            vec![&[]]
        } else {
            orders.chunks(per_part.max(1)).collect()
        };
        let count = parts.len();
        Ok(parts
            .into_iter()
            .enumerate()
            .map(|(index, part)| ServerMessage::OrderSnapshot {
                request_id,
                listing,
                next_report: 1,
                complete: index + 1 == count,
                orders: part.to_vec(),
            })
            .collect())
    })
}

/// The consolidated tape's current view of one instrument, read from the
/// processor at the hub.
pub(crate) fn tape_snapshot_job(request_id: u32, instrument_id: u128) -> JobWork<ServerMessage> {
    Box::new(move |context| {
        let (last_report, quotes) = context
            .origin
            .tape()
            .snapshot(InstrumentId::new(instrument_id), context.clock.now_us())?;
        Ok(vec![ServerMessage::TapeSnapshot {
            request_id,
            instrument_id,
            last_report,
            quotes: quotes
                .into_iter()
                .map(|(key, side, price, quantity)| TapeQuote {
                    venue_id: key.venue_id.get(),
                    side: wire_side(side),
                    price: price.get(),
                    quantity: quantity.get(),
                })
                .collect(),
        }])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_application::bnp::wire_listing;
    use bunting_market_types::{PriceTicks, QuantityLots, VenueId};

    fn key(venue: u128) -> ListingKey {
        ListingKey::new(VenueId::new(venue), InstrumentId::new(3))
    }

    #[test]
    fn increments_are_encoded_by_what_the_subscription_is() {
        let trade = MarketDataIncrement::Trade {
            price: PriceTicks::new(100),
            quantity: QuantityLots::new(2),
            reference: Some(9),
            buyer: Some(ParticipantId::new(4)),
            seller: None,
        };
        let order = MarketDataIncrement::Order {
            action: MarketDataUpdateAction::Change,
            side: Side::Sell,
            reference: 9,
            price: PriceTicks::new(100),
            quantity: QuantityLots::new(1),
            broker: Some(ParticipantId::new(5)),
        };
        let bid_gone = MarketDataIncrement::Level {
            action: MarketDataUpdateAction::Delete,
            side: Side::Buy,
            price: PriceTicks::new(99),
            quantity: QuantityLots::new(0),
        };
        let listing = wire_listing(key(1));
        let increment = RawIncrement {
            first_report: 7,
            entries: vec![(key(1), trade), (key(1), order)],
        };
        assert_eq!(
            encode(1, Kind::Orders(listing), &increment),
            ServerMessage::OrderUpdate {
                request_id: 1,
                listing,
                first_report: 7,
                entries: vec![
                    OrderEntry::Trade {
                        price: 100,
                        quantity: 2,
                        resting_reference: Some(9),
                        buyer: Some(4),
                        seller: None,
                    },
                    OrderEntry::Order {
                        action: OrderAction::Change,
                        side: bnp_wire::Side::Sell,
                        reference: 9,
                        price: 100,
                        quantity: 1,
                        broker: Some(5),
                    },
                ],
            }
        );
        let tape = RawIncrement {
            first_report: 12,
            entries: vec![(key(2), bid_gone)],
        };
        assert_eq!(
            encode(2, Kind::Tape(3), &tape),
            ServerMessage::TapeUpdate {
                request_id: 2,
                instrument_id: 3,
                first_report: 12,
                entries: vec![TapeEntry {
                    venue_id: 2,
                    kind: EntryKind::Bid,
                    price: 99,
                    quantity: 0,
                }],
            }
        );
    }

    #[test]
    fn a_snapshot_part_always_fits_its_frame() -> Result<(), bnp_wire::WireError> {
        for max_frame_bytes in [1_024, 16_384, bnp_wire::MAX_FRAME_BYTES] {
            let per_part = orders_per_part(max_frame_bytes);
            let order = BookOrder {
                side: bnp_wire::Side::Buy,
                reference: u64::MAX,
                price: i64::MIN,
                quantity: i64::MAX,
                broker: Some(u128::MAX),
                queue_position: u32::MAX,
            };
            let mut frame = Vec::new();
            bnp_wire::encode_server(
                &ServerMessage::OrderSnapshot {
                    request_id: u32::MAX,
                    listing: Listing {
                        venue_id: u128::MAX,
                        instrument_id: u128::MAX,
                    },
                    next_report: 1,
                    complete: true,
                    orders: vec![order; per_part],
                },
                &mut frame,
            )?;
            assert!(
                frame.len() <= max_frame_bytes,
                "{max_frame_bytes}: {}",
                frame.len()
            );
        }
        Ok(())
    }
}
