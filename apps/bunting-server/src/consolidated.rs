//! The consolidated tape (ADR 0036): a processor at the hub's location on
//! the latency map that hears every venue's public changes, each after its
//! venue-to-hub path plus a processing delay, and publishes one
//! consolidated stream per instrument: every venue's best bid and offer and
//! every venue's trades, each entry naming its venue.
//!
//! Like a real securities information processor it is slower than the direct
//! venue feeds for anyone near a venue, which is what makes watching only the
//! consolidated view a risk. Its report sequence (FIX 83) is per instrument,
//! the same for every subscriber.

use crate::distributor::{Fanout, Subscription};
use crate::wake::Waker;
use bunting_application::PublicListingUpdate;
use bunting_market_events::Side;
use bunting_market_types::{InstrumentId, ListingKey, PriceTicks, QuantityLots, VenueId};
use simfix_mapping::{MarketDataIncrement, MarketDataUpdateAction};
use std::collections::BTreeMap;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

type Quote = Option<(PriceTicks, QuantityLots)>;

/// One venue's best bid or offer as the processor sees it.
pub(crate) type VenueQuote = (ListingKey, Side, PriceTicks, QuantityLots);

/// `L(v, hub)` for a venue: how long its changes take to reach the
/// processor.
pub(crate) type DelayToHub = Box<dyn Fn(VenueId) -> Result<u64, String> + Send + Sync>;

/// One consolidated change, published when the processor applies it.
pub(crate) struct TapeRecord {
    pub(crate) instrument_id: InstrumentId,
    /// Venue time the processor applied it; subscribers receive it
    /// `L(hub, p)` later.
    pub(crate) hub_us: u64,
    /// Report sequence of the first entry; entries are numbered on from it.
    pub(crate) first_report: u64,
    pub(crate) entries: Vec<(ListingKey, MarketDataIncrement)>,
}

/// A venue's change on its way to the processor.
struct InFlight {
    listing_key: ListingKey,
    trades: Vec<(PriceTicks, QuantityLots)>,
    best_bid: Quote,
    best_ask: Quote,
}

#[derive(Default)]
struct TapeState {
    quotes: BTreeMap<ListingKey, (Quote, Quote)>,
    next_report: BTreeMap<InstrumentId, u64>,
    /// Ordered by arrival at the hub, then commit order.
    in_flight: BTreeMap<(u64, u64, usize), InFlight>,
    /// Set once the processor lost a change; the tape then stays down.
    failed: Option<String>,
}

pub(crate) struct ConsolidatedTape {
    processing_us: u64,
    capacity: usize,
    delay_to_hub: DelayToHub,
    state: Mutex<TapeState>,
    /// Signalled when a change starts its way to the hub.
    heard: Condvar,
    fanout: Fanout<TapeRecord>,
}

impl ConsolidatedTape {
    /// `capacity` bounds both the changes in flight to the hub and each
    /// subscriber's queue.
    pub(crate) fn new(processing_us: u64, capacity: usize, delay_to_hub: DelayToHub) -> Self {
        Self {
            processing_us,
            capacity: capacity.max(1),
            delay_to_hub,
            state: Mutex::new(TapeState::default()),
            heard: Condvar::new(),
            fanout: Fanout::new(capacity),
        }
    }

    /// Registers a session; records applied from now on are queued for it.
    pub(crate) fn subscribe(
        &self,
        waker: Option<Waker>,
    ) -> Result<Subscription<'_, TapeRecord>, String> {
        if let Some(reason) = &self.lock()?.failed {
            return Err(reason.clone());
        }
        self.fanout.subscribe(waker)
    }

    /// Seeds each venue's quotes the first time a run commits through this
    /// venue process (a restarted venue already has resting orders). Quotes
    /// the processor already knows are kept.
    pub(crate) fn prime(&self, quotes: impl IntoIterator<Item = (ListingKey, (Quote, Quote))>) {
        if let Ok(mut state) = self.state.lock() {
            for (key, quote) in quotes {
                state.quotes.entry(key).or_insert(quote);
            }
        }
    }

    /// The processor's current view of one instrument, after applying every
    /// change that has reached it by `now_us`: the last report sequence and
    /// each venue's quotes.
    pub(crate) fn snapshot(
        &self,
        instrument_id: InstrumentId,
        now_us: u64,
    ) -> Result<(u64, Vec<VenueQuote>), String> {
        let mut state = self.lock()?;
        self.release_locked(&mut state, now_us);
        if let Some(reason) = &state.failed {
            return Err(reason.clone());
        }
        let last = state
            .next_report
            .get(&instrument_id)
            .map_or(0, |next| next.saturating_sub(1));
        let mut quotes = Vec::new();
        for (key, (bid, ask)) in &state.quotes {
            if key.instrument_id != instrument_id {
                continue;
            }
            for (side, quote) in [(Side::Buy, bid), (Side::Sell, ask)] {
                if let Some((price, quantity)) = quote {
                    quotes.push((*key, side, *price, *quantity));
                }
            }
        }
        Ok((last, quotes))
    }

    /// Runs the processor: applies and publishes each change once it has
    /// reached the hub. Returns only if the state lock is poisoned.
    pub(crate) fn run(&self, now_us: impl Fn() -> u64) -> Result<(), String> {
        let mut state = self.lock()?;
        loop {
            let wait = self.release_locked(&mut state, now_us()).map_or(
                Duration::from_millis(100),
                |due| {
                    Duration::from_micros(due.saturating_sub(now_us()))
                        .clamp(Duration::from_micros(50), Duration::from_millis(100))
                },
            );
            state = self
                .heard
                .wait_timeout(state, wait)
                .map_err(|_| "consolidated tape is unavailable".to_owned())?
                .0;
        }
    }

    /// Starts each public change of one committed batch on its way to the
    /// hub. Called on the committer right after the commit is durable, so it
    /// cannot fail the commit: if the bound is exceeded the tape goes down
    /// (every subscriber is disconnected) rather than publish a hole.
    pub(crate) fn hear(&self, ordinal: u64, durable_us: u64, public: &[PublicListingUpdate]) {
        if public.is_empty() {
            return;
        }
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.failed.is_some() {
            return;
        }
        for (index, update) in public.iter().enumerate() {
            let delay = match (self.delay_to_hub)(update.listing_key.venue_id) {
                Ok(delay) => delay,
                Err(error) => {
                    self.fail(
                        &mut state,
                        format!("consolidated tape path failed: {error}"),
                    );
                    return;
                }
            };
            if state.in_flight.len() >= self.capacity {
                let reason = format!(
                    "consolidated tape held more than {} changes in flight and is down",
                    self.capacity
                );
                self.fail(&mut state, reason);
                return;
            }
            let arrives = durable_us
                .saturating_add(delay)
                .saturating_add(self.processing_us);
            state.in_flight.insert(
                (arrives, ordinal, index),
                InFlight {
                    listing_key: update.listing_key,
                    trades: update
                        .trades
                        .iter()
                        .map(|trade| (trade.price, trade.quantity))
                        .collect(),
                    best_bid: update.best_bid,
                    best_ask: update.best_ask,
                },
            );
        }
        drop(state);
        self.heard.notify_all();
    }

    fn fail(&self, state: &mut TapeState, reason: String) {
        state.in_flight.clear();
        state.failed = Some(reason);
        self.fanout.close_all();
    }

    /// Applies and publishes every change due by `now_us`; returns when the
    /// next one is due.
    fn release_locked(&self, state: &mut TapeState, now_us: u64) -> Option<u64> {
        while let Some(entry) = state.in_flight.first_entry() {
            let hub_us = entry.key().0;
            if hub_us > now_us {
                return Some(hub_us);
            }
            let change = entry.remove();
            let key = change.listing_key;
            let (old_bid, old_ask) = state.quotes.get(&key).copied().unwrap_or((None, None));
            let mut entries: Vec<_> = change
                .trades
                .iter()
                .map(|&(price, quantity)| {
                    (
                        key,
                        MarketDataIncrement::Trade {
                            price,
                            quantity,
                            reference: None,
                        },
                    )
                })
                .collect();
            for (side, old, new) in [
                (Side::Buy, old_bid, change.best_bid),
                (Side::Sell, old_ask, change.best_ask),
            ] {
                if old == new {
                    continue;
                }
                let (action, (price, quantity)) = match (old, new) {
                    (_, None) => (
                        MarketDataUpdateAction::Delete,
                        (
                            old.map_or(PriceTicks::new(0), |(price, _)| price),
                            QuantityLots::new(0),
                        ),
                    ),
                    (None, Some(quote)) => (MarketDataUpdateAction::New, quote),
                    (Some(_), Some(quote)) => (MarketDataUpdateAction::Change, quote),
                };
                entries.push((
                    key,
                    MarketDataIncrement::Level {
                        action,
                        side,
                        price,
                        quantity,
                    },
                ));
            }
            state.quotes.insert(key, (change.best_bid, change.best_ask));
            if entries.is_empty() {
                continue;
            }
            let next = state.next_report.entry(key.instrument_id).or_insert(1);
            let first_report = *next;
            *next = next.saturating_add(u64::try_from(entries.len()).unwrap_or(u64::MAX));
            self.fanout.send(&Arc::new(TapeRecord {
                instrument_id: key.instrument_id,
                hub_us,
                first_report,
                entries,
            }));
        }
        None
    }

    fn lock(&self) -> Result<MutexGuard<'_, TapeState>, String> {
        self.state
            .lock()
            .map_err(|_| "consolidated tape is unavailable".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_application::PublicTrade;
    use bunting_market_types::{EventSequence, LogicalTimeNs};

    fn key(venue: u128) -> ListingKey {
        ListingKey::new(VenueId::new(venue), InstrumentId::new(1))
    }

    fn level(price: i64, quantity: i64) -> (PriceTicks, QuantityLots) {
        (PriceTicks::new(price), QuantityLots::new(quantity))
    }

    fn update(venue: u128, ask: Quote, trade: bool) -> Vec<PublicListingUpdate> {
        vec![PublicListingUpdate {
            listing_key: key(venue),
            trades: if trade {
                vec![PublicTrade {
                    sequence: EventSequence::new(1),
                    logical_time: LogicalTimeNs::new(0),
                    instrument_id: InstrumentId::new(1),
                    listing_key: key(venue),
                    price: PriceTicks::new(100),
                    quantity: QuantityLots::new(1),
                    maker_reference: None,
                }]
            } else {
                Vec::new()
            },
            levels: Vec::new(),
            best_bid: None,
            best_ask: ask,
            orders: Vec::new(),
        }]
    }

    /// Venue 1 is 40 ms from the hub, venue 2 is 1 ms away.
    fn tape(processing_us: u64, capacity: usize) -> ConsolidatedTape {
        ConsolidatedTape::new(
            processing_us,
            capacity,
            Box::new(|venue| {
                Ok(if venue == VenueId::new(1) {
                    40_000
                } else {
                    1_000
                })
            }),
        )
    }

    #[test]
    fn changes_reach_the_tape_in_hub_arrival_order_with_one_sequence() -> Result<(), String> {
        let tape = tape(500, 64);
        let records = tape.subscribe(None)?;
        // Venue 1 commits first but is far from the hub.
        tape.hear(0, 0, &update(1, Some(level(101, 5)), false));
        tape.hear(1, 2_000, &update(2, Some(level(102, 3)), true));
        let (last, quotes) = tape.snapshot(InstrumentId::new(1), 3_000)?;
        assert_eq!(last, 0);
        assert!(quotes.is_empty());
        let (last, quotes) = tape.snapshot(InstrumentId::new(1), 3_500)?;
        assert_eq!(last, 2, "venue 2's trade and new offer");
        assert_eq!(quotes.len(), 1);
        let (last, quotes) = tape.snapshot(InstrumentId::new(1), 40_500)?;
        assert_eq!(last, 3);
        assert_eq!(quotes.len(), 2);
        let published = records.drain()?;
        assert_eq!(
            published
                .iter()
                .map(|record| (record.hub_us, record.first_report))
                .collect::<Vec<_>>(),
            vec![(3_500, 1), (40_500, 3)]
        );
        assert!(matches!(
            published[0].entries[0].1,
            MarketDataIncrement::Trade { .. }
        ));
        Ok(())
    }

    #[test]
    fn an_emptied_side_is_a_delete_and_unchanged_quotes_publish_nothing() -> Result<(), String> {
        let tape = tape(0, 64);
        let records = tape.subscribe(None)?;
        tape.hear(0, 0, &update(2, Some(level(101, 5)), false));
        tape.hear(1, 10, &update(2, Some(level(101, 5)), false));
        tape.hear(2, 20, &update(2, None, false));
        tape.snapshot(InstrumentId::new(1), 10_000)?;
        let published = records.drain()?;
        assert_eq!(published.len(), 2);
        assert!(matches!(
            published[1].entries[0].1,
            MarketDataIncrement::Level {
                action: MarketDataUpdateAction::Delete,
                price,
                quantity,
                ..
            } if price == PriceTicks::new(101) && quantity == QuantityLots::new(0)
        ));
        Ok(())
    }

    #[test]
    fn primed_quotes_are_the_base_of_the_first_change() -> Result<(), String> {
        let tape = tape(0, 64);
        tape.prime([(key(2), (None, Some(level(101, 5))))]);
        let records = tape.subscribe(None)?;
        let (last, quotes) = tape.snapshot(InstrumentId::new(1), 0)?;
        assert_eq!((last, quotes.len()), (0, 1));
        tape.hear(0, 0, &update(2, Some(level(101, 7)), false));
        tape.snapshot(InstrumentId::new(1), 1_000)?;
        assert!(matches!(
            records.drain()?[0].entries[0].1,
            MarketDataIncrement::Level {
                action: MarketDataUpdateAction::Change,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn exceeding_the_in_flight_bound_takes_the_tape_down() -> Result<(), String> {
        let tape = tape(0, 2);
        let records = tape.subscribe(None)?;
        tape.hear(0, 0, &update(1, Some(level(101, 1)), false));
        tape.hear(1, 0, &update(1, Some(level(102, 1)), false));
        assert!(records.drain()?.is_empty());
        tape.hear(2, 0, &update(1, Some(level(103, 1)), false));
        assert!(records.drain().is_err(), "subscribers are disconnected");
        assert!(tape.subscribe(None).is_err());
        assert!(tape.snapshot(InstrumentId::new(1), 0).is_err());
        Ok(())
    }
}
