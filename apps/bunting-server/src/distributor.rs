//! Bounded fan-out of committed event batches to participant sessions.
//!
//! Every successful origin commit publishes its canonical events exactly once,
//! after durability, through [`PublishingOrigin`]. Each FIX session subscribes
//! and maps only its own participant's facts, so a resting maker receives its
//! fills no matter which connection, administrator or built-in agent caused
//! them. Commits (and therefore publishes) happen only on the admission
//! sequencer thread, so every subscriber observes batches in commit order.

use crate::admission::VenueClock;
use crate::consolidated::ConsolidatedTape;
use crate::storage::NativeOrigin;
use crate::wake::Waker;
use bunting_admission_sequencer::Endpoint;
use bunting_application::competition::{DealChanges, deal_changes};
use bunting_application::{
    DisplayedOrder, PublicDepth, PublicListingUpdate, diff_levels, diff_orders, displayed_orders,
    order_references, project_public_event, trade_brokers,
};
use bunting_engine::RunState;
use bunting_market_events::{EventEnvelope, EventPayload};
use bunting_market_types::{CommandId, ListingKey, OrderId, RunId};
use bunting_origin_store::{
    AdmissionRecord, CommandResult, Executed, JournalInput, OriginError, OriginStore,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

/// Committed batches a session may fall behind before it is disconnected.
pub(crate) const MAX_PENDING_BATCHES: usize = 4_096;

/// Latest committed batches kept for sessions resuming a private stream
/// (BNP, ADR 0040). A cursor older than this window resumes with a gap.
pub(crate) const RETAINED_BATCHES: usize = 16_384;

/// One committed command's canonical events, its venue commit time and
/// where it was applied, shared by every subscriber.
pub(crate) struct Committed {
    /// Venue clock (microseconds) when the commit became durable; outbound
    /// delays are measured from here (ADR 0035).
    pub(crate) durable_us: u64,
    /// Where the command was applied; its reports travel from here to each
    /// team over the latency map.
    pub(crate) source: Endpoint,
    pub(crate) events: Vec<EventEnvelope>,
    /// Position in the venue's publish order, from zero. A reply stamped
    /// with [`EventDistributor::published`] reflects exactly the batches
    /// before it.
    pub(crate) ordinal: u64,
    /// Anonymous public trades and depth changes of each listing this
    /// commit touched, computed from the state right after it committed.
    pub(crate) public: Vec<PublicListingUpdate>,
    /// Listings whose public projection failed after this commit became
    /// durable (ADR 0053 §2). Empty on the normal path. A feed of one of
    /// these listings must skip a report sequence number for this batch,
    /// so its subscribers see a gap and resnapshot instead of trusting a
    /// book that is silently missing this commit.
    pub(crate) public_gaps: Vec<ListingKey>,
    /// Public order-by-order reference of each order this commit rested or
    /// reduced that still rests, sorted by order: its owner's reports carry
    /// it (`bunting_application::order_references`).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the BNP host is native only")
    )]
    pub(crate) references: Vec<(OrderId, u64)>,
    /// Tenders and OTC negotiations this commit changed, as they stand
    /// after it; their parties' streams carry them (ADR 0043).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(dead_code, reason = "the BNP host is native only")
    )]
    pub(crate) deals: DealChanges,
}

/// Where a committed batch was applied: the admitted destination when the
/// command came through the sequencer, otherwise (built-in agents, operator
/// commands) the venue of the first event naming a listing, else the hub.
fn batch_source(admission: Option<&AdmissionRecord>, events: &[EventEnvelope]) -> Endpoint {
    if let Some(admission) = admission {
        return admission.destination;
    }
    events
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::OrderReceived { listing_key, .. }
            | EventPayload::OrderRested { listing_key, .. }
            | EventPayload::OrderCanceled { listing_key, .. }
            | EventPayload::TradeExecuted { listing_key, .. } => {
                listing_key.map(|listing| Endpoint::Venue(listing.venue_id))
            }
            _ => None,
        })
        .unwrap_or(Endpoint::Hub)
}

pub(crate) type CommittedBatch = Arc<Committed>;

struct Subscriber<T> {
    sender: SyncSender<Arc<T>>,
    overflowed: Arc<AtomicBool>,
    waker: Option<Waker>,
}

/// Registry of bounded per-subscriber queues of shared items.
pub(crate) struct Fanout<T> {
    capacity: usize,
    next_id: AtomicU64,
    subscribers: Mutex<BTreeMap<u64, Subscriber<T>>>,
}

impl<T> Fanout<T> {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_id: AtomicU64::new(0),
            subscribers: Mutex::new(BTreeMap::new()),
        }
    }

    /// Registers a subscriber; items sent from now on are queued for it.
    pub(crate) fn subscribe(&self, waker: Option<Waker>) -> Result<Subscription<'_, T>, String> {
        let (sender, receiver) = sync_channel(self.capacity);
        let overflowed = Arc::new(AtomicBool::new(false));
        let id = self.next_id.fetch_add(1, Ordering::AcqRel);
        self.subscribers
            .lock()
            .map_err(|_| "event distributor is unavailable".to_owned())?
            .insert(
                id,
                Subscriber {
                    sender,
                    overflowed: overflowed.clone(),
                    waker,
                },
            );
        Ok(Subscription {
            fanout: self,
            id,
            receiver,
            overflowed,
        })
    }

    /// Queues one item for every subscriber without blocking the sender. A
    /// full queue marks that subscriber overflowed and drops it; it then
    /// disconnects instead of silently skipping items.
    pub(crate) fn send(&self, item: &Arc<T>) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        subscribers.retain(|_, subscriber| {
            let keep = match subscriber.sender.try_send(item.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    subscriber.overflowed.store(true, Ordering::Release);
                    false
                }
                Err(TrySendError::Disconnected(_)) => false,
            };
            // Wake the subscriber for the item, or to notice the overflow.
            if let Some(waker) = &subscriber.waker {
                waker.wake();
            }
            keep
        });
    }

    /// Disconnects every subscriber: each one's next drain fails, so its
    /// session reconnects instead of reading a stream with a hole in it.
    pub(crate) fn close_all(&self) {
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        for subscriber in std::mem::take(&mut *subscribers).into_values() {
            subscriber.overflowed.store(true, Ordering::Release);
            if let Some(waker) = &subscriber.waker {
                waker.wake();
            }
        }
    }

    #[cfg(test)]
    fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map_or(0, |subscribers| subscribers.len())
    }
}

/// Committed batches, in commit order, to every session and host.
pub(crate) struct EventDistributor {
    fanout: Fanout<Committed>,
    published: AtomicU64,
    /// The latest batches, oldest first. Publishing holds this lock while it
    /// sends, so a resuming subscription sees each batch exactly once,
    /// either here or on its queue.
    retained: Mutex<VecDeque<CommittedBatch>>,
    retain: usize,
}

/// Where a resumed private stream starts.
pub(crate) enum Resume {
    /// Every retained batch with events after the cursor, oldest first;
    /// later batches arrive on the subscription.
    Replay(Vec<CommittedBatch>),
    /// The cursor is older than the retention window (or not a position
    /// of this run): the stream starts live.
    Gap,
}

impl EventDistributor {
    pub(crate) fn new(capacity: usize) -> Self {
        Self::with_retention(capacity, RETAINED_BATCHES)
    }

    pub(crate) fn with_retention(capacity: usize, retain: usize) -> Self {
        Self {
            fanout: Fanout::new(capacity),
            published: AtomicU64::new(0),
            retained: Mutex::new(VecDeque::new()),
            retain,
        }
    }

    /// Batches published so far: the ordinal the next batch will carry.
    pub(crate) fn published(&self) -> u64 {
        self.published.load(Ordering::Acquire)
    }

    /// Registers a session; batches committed from now on are queued for it.
    pub(crate) fn subscribe(&self, waker: Option<Waker>) -> Result<Subscription<'_>, String> {
        self.fanout.subscribe(waker)
    }

    /// [`Self::subscribe`] for a session resuming after event sequence
    /// `after`. Atomically with the registration it returns the retained
    /// batches the session missed, or [`Resume::Gap`] when the window no
    /// longer covers the cursor. `current` reads the run's committed event
    /// sequence; it decides coverage when nothing is retained yet (for
    /// example right after a restart).
    pub(crate) fn subscribe_resuming(
        &self,
        waker: Option<Waker>,
        after: u64,
        current: impl FnOnce() -> Option<u64>,
    ) -> Result<(Subscription<'_>, Resume), String> {
        let retained = self
            .retained
            .lock()
            .map_err(|_| "event distributor is unavailable".to_owned())?;
        let subscription = self.fanout.subscribe(waker)?;
        let first = |batch: &CommittedBatch| batch.events.first().map(|event| event.sequence.get());
        let last = retained
            .back()
            .and_then(|batch| batch.events.last())
            .map(|event| event.sequence.get());
        let covered = match retained.front().and_then(first) {
            // Every event after the cursor is retained, and the cursor is
            // a position this venue has published.
            Some(oldest) => after.saturating_add(1) >= oldest && Some(after) <= last,
            None => current() == Some(after),
        };
        let resume = if covered {
            Resume::Replay(
                retained
                    .iter()
                    .filter(|batch| {
                        batch
                            .events
                            .last()
                            .is_some_and(|event| event.sequence.get() > after)
                    })
                    .cloned()
                    .collect(),
            )
        } else {
            Resume::Gap
        };
        Ok((subscription, resume))
    }

    /// Queues one committed batch for every subscriber without blocking the
    /// committer.
    fn publish(
        &self,
        events: &[EventEnvelope],
        committed_us: u64,
        source: Endpoint,
        public: Vec<PublicListingUpdate>,
        public_gaps: Vec<ListingKey>,
        (references, deals): (Vec<(OrderId, u64)>, DealChanges),
    ) {
        if events.is_empty() {
            return;
        }
        let batch: CommittedBatch = Arc::new(Committed {
            durable_us: committed_us,
            source,
            events: events.to_vec(),
            ordinal: self.published.fetch_add(1, Ordering::AcqRel),
            public,
            public_gaps,
            references,
            deals,
        });
        let Ok(mut retained) = self.retained.lock() else {
            return;
        };
        self.fanout.send(&batch);
        if self.retain > 0 {
            if retained.len() >= self.retain {
                retained.pop_front();
            }
            retained.push_back(batch);
        }
    }

    #[cfg(test)]
    fn subscriber_count(&self) -> usize {
        self.fanout.subscriber_count()
    }
}

/// One subscriber's ordered view of a fanout.
pub(crate) struct Subscription<'a, T = Committed> {
    fanout: &'a Fanout<T>,
    id: u64,
    receiver: Receiver<Arc<T>>,
    overflowed: Arc<AtomicBool>,
}

impl<T> Subscription<'_, T> {
    /// Returns every item queued so far, in order, without blocking.
    ///
    /// # Errors
    /// Returns an error once the queue has overflowed; the caller must
    /// disconnect so the participant recovers state rather than miss an item.
    pub(crate) fn drain(&self) -> Result<Vec<Arc<T>>, String> {
        if self.overflowed.load(Ordering::Acquire) {
            return Err(format!(
                "stream closed: more than {} pending items or the source failed; reconnect to recover",
                self.fanout.capacity
            ));
        }
        let mut items = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(item) => items.push(item),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(items),
            }
        }
    }
}

impl<T> Drop for Subscription<'_, T> {
    fn drop(&mut self) {
        if let Ok(mut subscribers) = self.fanout.subscribers.lock() {
            subscribers.remove(&self.id);
        }
    }
}

/// The venue's origin: commits through the durable store, then publishes the
/// committed events. Every mutating path (FIX, administration, built-in
/// agents) commits through this type, so none can skip publication.
pub(crate) struct PublishingOrigin {
    inner: NativeOrigin,
    distributor: EventDistributor,
    clock: VenueClock,
    /// Last published visible depth of every listing, per run: the base the
    /// next commit's public depth changes are measured against.
    books: Mutex<BTreeMap<RunId, BTreeMap<ListingKey, PublicDepth>>>,
    /// The consolidated tape's processor, fed each commit's public changes.
    tape: ConsolidatedTape,
    /// Set once any session asks for an order-by-order feed; until then no
    /// commit pays for order-level views.
    orders_enabled: AtomicBool,
    /// Last published displayed orders of every listing, per run, once
    /// order-by-order feeds are enabled.
    orders: Mutex<BTreeMap<RunId, BTreeMap<ListingKey, Vec<DisplayedOrder>>>>,
    /// Post-commit projection failures so far (public updates and the
    /// private order-reference/deal read). Each is also logged.
    projection_failures: AtomicU64,
    /// Test-only fault injection: the next public projection fails.
    #[cfg(test)]
    fail_public_projection: AtomicBool,
}

impl PublishingOrigin {
    pub(crate) fn new(
        inner: NativeOrigin,
        capacity: usize,
        clock: VenueClock,
        tape: ConsolidatedTape,
    ) -> Self {
        Self {
            inner,
            distributor: EventDistributor::new(capacity),
            clock,
            books: Mutex::new(BTreeMap::new()),
            tape,
            orders_enabled: AtomicBool::new(false),
            orders: Mutex::new(BTreeMap::new()),
            projection_failures: AtomicU64::new(0),
            #[cfg(test)]
            fail_public_projection: AtomicBool::new(false),
        }
    }

    #[cfg(test)]
    pub(crate) fn projection_failures(&self) -> u64 {
        self.projection_failures.load(Ordering::Acquire)
    }

    /// Forces the next commit's public projection to fail (tests only).
    #[cfg(test)]
    pub(crate) fn fail_next_public_projection(&self) {
        self.fail_public_projection.store(true, Ordering::Release);
    }

    /// Starts order-by-order views: the next commit records every listing's
    /// displayed orders before it executes and publishes order changes from
    /// then on. Idempotent.
    pub(crate) fn enable_order_feeds(&self) {
        self.orders_enabled.store(true, Ordering::Release);
    }

    /// Records every listing's displayed orders the first time a run commits
    /// with order-by-order feeds enabled.
    fn prime_orders(&self, run_id: RunId) -> Result<(), OriginError> {
        if !self.orders_enabled.load(Ordering::Acquire) {
            return Ok(());
        }
        let mut orders = self.orders.lock().map_err(|_| OriginError::Unavailable)?;
        if orders.contains_key(&run_id) {
            return Ok(());
        }
        let views = self.inner.read_run(run_id, |state| {
            state
                .listings()
                .keys()
                .filter_map(|&key| displayed_orders(state, key).ok().map(|view| (key, view)))
                .collect::<BTreeMap<_, _>>()
        })?;
        orders.insert(run_id, views);
        Ok(())
    }

    /// Records every listing's visible depth the first time a run commits
    /// through this origin, so the first public changes have a base.
    fn prime_books(&self, run_id: RunId) -> Result<(), OriginError> {
        let mut books = self.books.lock().map_err(|_| OriginError::Unavailable)?;
        if books.contains_key(&run_id) {
            return Ok(());
        }
        let depths = self.inner.read_run(run_id, |state| {
            state
                .listings()
                .keys()
                .filter_map(|&key| state.visible_levels(key).ok().map(|depth| (key, depth)))
                .collect::<BTreeMap<_, _>>()
        })?;
        self.tape.prime(
            depths
                .iter()
                .map(|(&key, (bids, asks))| (key, (bids.first().copied(), asks.first().copied()))),
        );
        books.insert(run_id, depths);
        Ok(())
    }

    /// The public, anonymous view of one commit: trades and depth changes of
    /// every listing its events touched. A batch that names no listing on
    /// some book event (or carries a simulation event that may move books)
    /// re-reads every listing of the run.
    fn public_updates(
        &self,
        run_id: RunId,
        events: &[EventEnvelope],
    ) -> Result<Vec<PublicListingUpdate>, OriginError> {
        #[cfg(test)]
        if self.fail_public_projection.swap(false, Ordering::AcqRel) {
            return Err(OriginError::Unavailable);
        }
        let mut books = self.books.lock().map_err(|_| OriginError::Unavailable)?;
        // An unprimed run has no base to diff against: that is a failed
        // projection, not an empty one. Returning `Ok` here would hide
        // this commit's public updates exactly like a read failure.
        let Some(known) = books.get_mut(&run_id) else {
            return Err(OriginError::Unavailable);
        };
        let mut touched = BTreeSet::new();
        let mut everything = false;
        for event in events {
            match &event.payload {
                EventPayload::OrderRested { listing_key, .. }
                | EventPayload::OrderCanceled { listing_key, .. }
                | EventPayload::TradeExecuted { listing_key, .. } => match listing_key {
                    Some(key) => {
                        touched.insert(*key);
                    }
                    None => everything = true,
                },
                EventPayload::Simulation(_) => everything = true,
                _ => {}
            }
        }
        if everything {
            touched.extend(known.keys().copied());
        }
        let mut order_views = self.orders.lock().map_err(|_| OriginError::Unavailable)?;
        let mut order_views = order_views.get_mut(&run_id);
        let with_orders = order_views.is_some();
        let after = self.inner.read_run(run_id, |state| {
            touched
                .iter()
                .filter_map(|&key| {
                    let depth = state.visible_levels(key).ok()?;
                    let orders = if with_orders {
                        displayed_orders(state, key).ok()
                    } else {
                        None
                    };
                    // Broker identifiers of this listing's trades, in order.
                    let brokers: Vec<_> = events
                        .iter()
                        .filter(|event| project_public_event(event, key).is_some())
                        .map(|event| trade_brokers(state, event))
                        .collect();
                    Some((key, depth, orders, brokers))
                })
                .collect::<Vec<_>>()
        })?;
        let mut updates = Vec::new();
        for (key, depth, displayed, brokers) in after {
            let levels = known.get(&key).map_or_else(
                || diff_levels(&(Vec::new(), Vec::new()), &depth),
                |before| diff_levels(before, &depth),
            );
            let mut trades: Vec<_> = events
                .iter()
                .filter_map(|event| project_public_event(event, key))
                .collect();
            for (trade, (buyer, seller)) in trades.iter_mut().zip(brokers) {
                trade.buyer_broker = buyer;
                trade.seller_broker = seller;
            }
            known.insert(key, depth);
            let mut orders = Vec::new();
            if let (Some(views), Some(displayed)) = (order_views.as_deref_mut(), displayed) {
                let before = views.remove(&key).unwrap_or_default();
                // Each trade names the public reference its resting order
                // had before this commit.
                let makers = events.iter().filter_map(|event| match &event.payload {
                    EventPayload::TradeExecuted {
                        listing_key: Some(executed_at),
                        maker_order_id,
                        ..
                    } if *executed_at == key => Some(*maker_order_id),
                    _ => None,
                });
                for (trade, maker) in trades.iter_mut().zip(makers) {
                    trade.maker_reference = before
                        .iter()
                        .find(|order| order.order_id == maker)
                        .map(|order| order.reference);
                }
                orders = diff_orders(&before, &displayed);
                views.insert(key, displayed);
            }
            if !levels.is_empty() || !trades.is_empty() || !orders.is_empty() {
                updates.push(PublicListingUpdate {
                    listing_key: key,
                    trades,
                    levels,
                    best_bid: known.get(&key).and_then(|depth| depth.0.first().copied()),
                    best_ask: known.get(&key).and_then(|depth| depth.1.first().copied()),
                    orders,
                });
            }
        }
        Ok(updates)
    }

    /// Recovery after a failed public projection (ADR 0053 §2): returns
    /// every listing that may be affected, rebuilds the depth and
    /// displayed-order caches from committed state, and schedules the
    /// consolidated tape's gap for those listings. The failure itself is
    /// counted and logged by the caller; a rebuild that also fails leaves
    /// the caches absent, so the next commit primes from committed state
    /// before it executes rather than diffing against a half-updated base.
    fn recover_public_projection(
        &self,
        run_id: RunId,
        events: &[EventEnvelope],
        ordinal: u64,
        durable_us: u64,
    ) -> Vec<ListingKey> {
        // The failure cannot say which listings it touched, so every
        // listing of the run counts as affected: named by this commit's
        // events, known to either cache, or in the committed run.
        let mut affected = BTreeSet::new();
        for event in events {
            if let EventPayload::OrderReceived { listing_key, .. }
            | EventPayload::OrderRested { listing_key, .. }
            | EventPayload::OrderCanceled { listing_key, .. }
            | EventPayload::TradeExecuted { listing_key, .. } = &event.payload
            {
                affected.extend(listing_key.iter().copied());
            }
        }
        if let Ok(books) = self.books.lock() {
            if let Some(known) = books.get(&run_id) {
                affected.extend(known.keys().copied());
            }
        }
        if let Ok(orders) = self.orders.lock() {
            if let Some(known) = orders.get(&run_id) {
                affected.extend(known.keys().copied());
            }
        }
        if let Ok(listings) = self.inner.read_run(run_id, |state| {
            state.listings().keys().copied().collect::<Vec<_>>()
        }) {
            affected.extend(listings);
        }
        // Drop both caches for the run and re-prime them from the state
        // this commit left behind.
        if let Ok(mut books) = self.books.lock() {
            books.remove(&run_id);
        }
        if let Ok(mut orders) = self.orders.lock() {
            orders.remove(&run_id);
        }
        for (what, result) in [
            ("depth", self.prime_books(run_id)),
            ("order", self.prime_orders(run_id)),
        ] {
            if let Err(error) = result {
                eprintln!(
                    "bunting-server: rebuilding the {what} projection cache for run {run_id} failed: {error}; it stays unprimed until a later commit"
                );
            }
        }
        // The tape's processor needs the rebuilt best bid and offer of
        // each affected listing to pair with its skipped report. Without
        // them its snapshot could stay stale, so it goes down instead
        // (its existing failure mode) rather than publish a hole.
        let rebuilt = self.inner.read_run(run_id, |state| {
            affected
                .iter()
                .filter_map(|&key| match state.visible_levels(key) {
                    Ok((bids, asks)) => Some((key, (bids.first().copied(), asks.first().copied()))),
                    // A listing the run no longer has quotes nothing.
                    Err(_) if !state.listings().contains_key(&key) => Some((key, (None, None))),
                    Err(_) => None,
                })
                .collect::<Vec<_>>()
        });
        match rebuilt {
            Ok(rebuilt) if rebuilt.len() == affected.len() => {
                self.tape.hear_gap(ordinal, durable_us, &rebuilt);
            }
            _ => self.tape.take_down(format!(
                "consolidated tape quotes for run {run_id} could not be rebuilt after a failed public projection and the tape is down"
            )),
        }
        affected.into_iter().collect()
    }

    pub(crate) const fn inner(&self) -> &NativeOrigin {
        &self.inner
    }

    pub(crate) const fn distributor(&self) -> &EventDistributor {
        &self.distributor
    }

    pub(crate) const fn tape(&self) -> &ConsolidatedTape {
        &self.tape
    }
}

impl OriginStore for PublishingOrigin {
    fn execute_admitted(
        &self,
        input: &JournalInput,
        admission: Option<&AdmissionRecord>,
    ) -> Result<Executed, OriginError> {
        // Best effort: an unknown run fails in `execute` below.
        let _ = self.prime_books(input.run_id());
        let _ = self.prime_orders(input.run_id());
        let executed = self.inner.execute_admitted(input, admission)?;
        // A duplicate's events were published when it first committed.
        if !executed.duplicate {
            let durable_us = self.clock.now_us();
            // The commit is durable; a failed public projection must not
            // hide its reports, so it publishes without public updates.
            // But it must not be silent either (ADR 0053 §2): the batch
            // names every listing that may be affected as a public gap,
            // each direct feed of those listings skips a report sequence
            // number, and the consolidated tape skips one of its own, so
            // subscribers resnapshot instead of trusting a stale book.
            let (public, public_gaps) = match self.public_updates(input.run_id(), &executed.events)
            {
                Ok(public) => (public, Vec::new()),
                Err(error) => {
                    self.projection_failures.fetch_add(1, Ordering::AcqRel);
                    eprintln!(
                        "bunting-server: public projection failed after commit for run {}: {error}; affected public feeds will skip a report sequence",
                        input.run_id()
                    );
                    let gaps = self.recover_public_projection(
                        input.run_id(),
                        &executed.events,
                        self.distributor.published(),
                        durable_us,
                    );
                    (Vec::new(), gaps)
                }
            };
            // The same rule for the private order references and deal
            // changes: reports built from the events still go out, but a
            // failed read is counted and logged, never defaulted quietly.
            let private = match self.inner.read_run(input.run_id(), |state| {
                (
                    order_references(state, &executed.events),
                    deal_changes(state, &executed.events),
                )
            }) {
                Ok(private) => private,
                Err(error) => {
                    self.projection_failures.fetch_add(1, Ordering::AcqRel);
                    eprintln!(
                        "bunting-server: private projection (order references and deal changes) failed after commit for run {}: {error}; execution reports still publish without them",
                        input.run_id()
                    );
                    Default::default()
                }
            };
            // On the gap path the tape already heard its skipped report
            // (with rebuilt quotes) during recovery; `public` is empty.
            if public_gaps.is_empty() {
                self.tape
                    .hear(self.distributor.published(), durable_us, &public);
            }
            self.distributor.publish(
                &executed.events,
                durable_us,
                batch_source(admission, &executed.events),
                public,
                public_gaps,
                private,
            );
        }
        Ok(executed)
    }

    fn read_run<T>(
        &self,
        run_id: RunId,
        read: impl FnOnce(&RunState) -> T,
    ) -> Result<T, OriginError> {
        self.inner.read_run(run_id, read)
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        self.inner.find_command(run_id, command_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_admission_sequencer::RttSource;
    use bunting_market_events::CancelReason;
    use bunting_market_types::{
        CorrelationId, EventId, EventSequence, InstrumentId, ListingKey, LogicalTimeNs, OrderId,
        ParticipantId, QuantityLots, VenueId,
    };

    fn event(sequence: u64) -> EventEnvelope {
        EventEnvelope {
            schema_version: 1,
            run_id: RunId::new(1),
            event_id: EventId::new(u128::from(sequence)),
            sequence: EventSequence::new(sequence),
            logical_time: LogicalTimeNs::new(0),
            actor: ParticipantId::new(1),
            command_id: CommandId::new(1),
            correlation_id: CorrelationId::new(1),
            causation_sequence: None,
            payload: EventPayload::KillSwitchActivated,
        }
    }

    #[test]
    fn subscribers_receive_batches_in_order_and_unsubscribe_on_drop() -> Result<(), String> {
        let distributor = EventDistributor::new(8);
        let first = distributor.subscribe(None)?;
        distributor.publish(
            &[event(1)],
            1,
            Endpoint::Hub,
            Vec::new(),
            Vec::new(),
            Default::default(),
        );
        let second = distributor.subscribe(None)?;
        distributor.publish(
            &[event(2), event(3)],
            2,
            Endpoint::Hub,
            Vec::new(),
            Vec::new(),
            Default::default(),
        );
        distributor.publish(
            &[],
            3,
            Endpoint::Hub,
            Vec::new(),
            Vec::new(),
            Default::default(),
        );
        let sequences = |batches: Vec<CommittedBatch>| -> Vec<u64> {
            batches
                .iter()
                .flat_map(|batch| batch.events.iter().map(|event| event.sequence.get()))
                .collect()
        };
        assert_eq!(sequences(first.drain()?), vec![1, 2, 3]);
        assert_eq!(sequences(second.drain()?), vec![2, 3]);
        assert!(first.drain()?.is_empty());
        drop(second);
        assert_eq!(distributor.subscriber_count(), 1);
        Ok(())
    }

    #[test]
    fn a_full_queue_disconnects_the_slow_subscriber_only() -> Result<(), String> {
        let distributor = EventDistributor::new(2);
        let slow = distributor.subscribe(None)?;
        let fast = distributor.subscribe(None)?;
        for sequence in 1..=2 {
            distributor.publish(
                &[event(sequence)],
                sequence,
                Endpoint::Hub,
                Vec::new(),
                Vec::new(),
                Default::default(),
            );
        }
        assert_eq!(fast.drain()?.len(), 2);
        distributor.publish(
            &[event(3)],
            3,
            Endpoint::Hub,
            Vec::new(),
            Vec::new(),
            Default::default(),
        );
        assert!(slow.drain().is_err());
        assert_eq!(fast.drain()?.len(), 1);
        assert_eq!(distributor.subscriber_count(), 1);
        Ok(())
    }

    #[test]
    fn a_batch_travels_from_where_its_command_was_applied() {
        let mut canceled = event(1);
        canceled.payload = EventPayload::OrderCanceled {
            order_id: OrderId::new(1),
            participant_id: ParticipantId::new(1),
            instrument_id: InstrumentId::new(1),
            listing_key: Some(ListingKey {
                venue_id: VenueId::new(2),
                instrument_id: InstrumentId::new(1),
            }),
            remaining: QuantityLots::new(1),
            reason: CancelReason::KillSwitch,
        };
        let admitted = AdmissionRecord {
            received_us: 0,
            measured_one_way_us: None,
            rtt_source: RttSource::None,
            destination: Endpoint::Venue(VenueId::new(3)),
            path_latency_us: 0,
            jitter_position: None,
            release_us: 0,
            arrival_sequence: 0,
        };
        // A sequenced command: its admitted destination, never re-derived.
        assert_eq!(
            batch_source(Some(&admitted), &[canceled.clone()]),
            Endpoint::Venue(VenueId::new(3))
        );
        // Unsequenced (agents, operators): the venue its events name, else
        // the hub; never the acting team, whose path was not crossed.
        assert_eq!(
            batch_source(None, &[event(2), canceled]),
            Endpoint::Venue(VenueId::new(2))
        );
        assert_eq!(batch_source(None, &[event(3)]), Endpoint::Hub);
    }

    #[test]
    fn a_resuming_subscriber_gets_each_missed_batch_once() -> Result<(), String> {
        let distributor = EventDistributor::with_retention(8, 3);
        let sequences = |batches: &[CommittedBatch]| -> Vec<u64> {
            batches
                .iter()
                .flat_map(|batch| batch.events.iter().map(|event| event.sequence.get()))
                .collect()
        };
        // Nothing retained yet: only the run's current position is covered.
        let (_, resume) = distributor.subscribe_resuming(None, 0, || Some(0))?;
        assert!(matches!(resume, Resume::Replay(batches) if batches.is_empty()));
        let (_, resume) = distributor.subscribe_resuming(None, 0, || Some(5))?;
        assert!(matches!(resume, Resume::Gap));
        for sequence in 1..=4 {
            distributor.publish(
                &[event(sequence)],
                sequence,
                Endpoint::Hub,
                Vec::new(),
                Vec::new(),
                Default::default(),
            );
        }
        // Batches 2..=4 are retained; a cursor at 1 replays them all.
        let (live, resume) = distributor.subscribe_resuming(None, 1, || None)?;
        let Resume::Replay(batches) = resume else {
            return Err("cursor 1 is inside the window".to_owned());
        };
        assert_eq!(sequences(&batches), vec![2, 3, 4]);
        distributor.publish(
            &[event(5)],
            5,
            Endpoint::Hub,
            Vec::new(),
            Vec::new(),
            Default::default(),
        );
        assert_eq!(sequences(&live.drain()?), vec![5]);
        // Older than the window, or never published: a gap.
        assert!(matches!(
            distributor.subscribe_resuming(None, 0, || None)?.1,
            Resume::Gap
        ));
        assert!(matches!(
            distributor.subscribe_resuming(None, 9, || None)?.1,
            Resume::Gap
        ));
        Ok(())
    }

    /// A run with one listing, a funded seller (7, holding the instrument)
    /// and a funded buyer (8), ready to trade at venue 1.
    fn trading_origin() -> Result<PublishingOrigin, String> {
        use bunting_engine::{
            InstrumentDefinition, InstrumentKind, ListingDefinition, ParticipantDefinition,
            ScenarioDefinition,
        };
        use bunting_market_types::{
            CurrencyId, IterationId, MoneyMinor, PriceBounds, PriceTicks, ScenarioId,
            ScenarioVersion,
        };
        use bunting_origin_store::InMemoryOrigin;
        use bunting_risk_engine::RiskLimits;
        let participant = |id: u128, positions: i64| {
            ParticipantDefinition::new(
                ParticipantId::new(id),
                true,
                RiskLimits::new(
                    QuantityLots::new(100),
                    QuantityLots::new(1_000),
                    QuantityLots::new(1_000),
                ),
                BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(1_000_000))]),
                BTreeMap::from([(InstrumentId::new(1), QuantityLots::new(positions))]),
            )
        };
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [InstrumentDefinition::new(
                InstrumentId::new(1),
                "BNT",
                CurrencyId::new(1),
                InstrumentKind::Equity,
            )
            .with_opening_mark(PriceTicks::new(100))],
            [ListingDefinition::new(
                ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
                "ONE".to_owned(),
                PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000))
                    .map_err(|error| format!("{error:?}"))?,
            )
            .map_err(|error| format!("{error:?}"))?],
            [participant(7, 1_000), participant(8, 0)],
        )
        .map_err(|error| format!("{error:?}"))?;
        let run = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|error| format!("{error}"))?;
        let inner = NativeOrigin::Memory(InMemoryOrigin::new());
        inner.insert_run(run).map_err(|error| format!("{error}"))?;
        Ok(PublishingOrigin::new(
            inner,
            64,
            VenueClock::start(),
            ConsolidatedTape::new(0, 64, Box::new(|_| Ok(0))),
        ))
    }

    fn submit(
        origin: &PublishingOrigin,
        id: u128,
        actor: u128,
        side: bunting_market_events::Side,
        quantity: i64,
        price: i64,
    ) -> Result<Executed, String> {
        use bunting_market_events::{Command, CommandPayload, OrderKind, SubmitOrder};
        use bunting_market_types::PriceTicks;
        let expected_sequence = origin
            .read_run(RunId::new(1), RunState::sequence)
            .map_err(|error| format!("{error}"))?;
        origin
            .execute(&JournalInput::Command(Command {
                run_id: RunId::new(1),
                command_id: CommandId::new(id),
                correlation_id: CorrelationId::new(id),
                logical_time: LogicalTimeNs::new(
                    u64::try_from(id).map_err(|error| error.to_string())?,
                ),
                expected_sequence,
                actor: ParticipantId::new(actor),
                payload: CommandPayload::SubmitOrder(SubmitOrder {
                    order_id: OrderId::new(id),
                    instrument_id: InstrumentId::new(1),
                    participant_id: ParticipantId::new(actor),
                    side,
                    quantity: QuantityLots::new(quantity),
                    kind: OrderKind::Limit {
                        price: PriceTicks::new(price),
                    },
                    anonymous: false,
                }),
            }))
            .map_err(|error| format!("{error}"))
    }

    type FixFeeds<'a> = crate::public_feed::PublicFeeds<'a, simfix_wire::FixMessage>;

    fn report_sequences(messages: &[crate::public_feed::FeedMessage]) -> Vec<String> {
        messages
            .iter()
            .flat_map(|message| message.message.fields.iter())
            .filter(|field| field.tag == 83)
            .map(|field| field.value.clone())
            .collect()
    }

    /// Applies every batch to the direct feeds; returns the report
    /// sequences (83) of the messages produced.
    fn feed_reports(
        feeds: &mut FixFeeds<'_>,
        batches: &[CommittedBatch],
    ) -> Result<Vec<String>, String> {
        let mut messages = Vec::new();
        for batch in batches {
            messages.extend(feeds.on_batch(batch)?);
        }
        Ok(report_sequences(&messages))
    }

    /// Report sequences of every tape record the processor has applied.
    fn tape_reports(
        feeds: &mut FixFeeds<'_>,
        origin: &PublishingOrigin,
    ) -> Result<Vec<String>, String> {
        origin.tape().snapshot(InstrumentId::new(1), u64::MAX)?;
        Ok(report_sequences(&feeds.drain_tape()?))
    }

    fn committed_depth(
        origin: &PublishingOrigin,
        key: ListingKey,
    ) -> Result<bunting_application::PublicDepth, String> {
        origin
            .read_run(RunId::new(1), |state| state.visible_levels(key))
            .map_err(|error| format!("{error}"))?
            .map_err(|error| format!("{error:?}"))
    }

    fn has_trade(events: &[EventEnvelope], quantity: i64) -> bool {
        events.iter().any(|event| {
            matches!(
                &event.payload,
                EventPayload::TradeExecuted { buyer_id, seller_id, quantity: traded, .. }
                    if *buyer_id == ParticipantId::new(8)
                        && *seller_id == ParticipantId::new(7)
                        && *traded == QuantityLots::new(quantity)
            )
        })
    }

    /// Both sides' private facts are in the batch sessions build
    /// execution reports from: the trade, the maker's reduced order
    /// (3 left) and the taker's completed order.
    fn assert_both_sides_learn_of_the_fill(batch: &Committed) {
        assert!(has_trade(&batch.events, 2));
        assert!(
            batch.events.iter().any(|event| matches!(
                &event.payload,
                EventPayload::OrderReduced { order_id, remaining }
                    if *order_id == OrderId::new(1) && *remaining == QuantityLots::new(3)
            )),
            "the maker learns of its fill"
        );
        assert!(
            batch.events.iter().any(|event| matches!(
                &event.payload,
                EventPayload::OrderCompleted { order_id }
                    if *order_id == OrderId::new(2)
            )),
            "the taker learns of its fill"
        );
    }

    #[test]
    fn a_failed_public_projection_is_a_visible_feed_gap_not_a_silent_skip() -> Result<(), String> {
        use crate::public_feed::PublicFeeds;
        use bunting_application::MarketDataEntryType;
        use bunting_market_events::Side;
        let origin = trading_origin()?;
        let key = ListingKey::new(VenueId::new(1), InstrumentId::new(1));
        let private = origin.distributor().subscribe(None)?;
        let mut feeds = PublicFeeds::default();
        let all = [
            MarketDataEntryType::Bid,
            MarketDataEntryType::Offer,
            MarketDataEntryType::Trade,
        ];
        assert_eq!(feeds.subscribe("feed", key, &all, true), None);
        let waker = crate::wake::Waker::detached();
        assert_eq!(
            feeds.subscribe_consolidated("tape", InstrumentId::new(1), origin.tape(), &waker),
            None
        );
        feeds.activate("feed", 0, &[]);
        feeds.activate("tape", 0, &[]);

        // The maker rests 5 at 100: a normal first increment on both feeds.
        submit(&origin, 1, 7, Side::Sell, 5, 100)?;
        let batches = private.drain()?;
        assert_eq!(feed_reports(&mut feeds, &batches)?, ["1"]);
        assert_eq!(tape_reports(&mut feeds, &origin)?, ["1"]);

        // The taker lifts 2, and this commit's public projection fails.
        origin.fail_next_public_projection();
        let traded = submit(&origin, 2, 8, Side::Buy, 2, 100)?;
        assert!(has_trade(&traded.events, 2), "the commit itself stands");
        let batches = private.drain()?;
        assert_eq!(batches.len(), 1);
        assert_both_sides_learn_of_the_fill(&batches[0]);
        assert!(
            batches[0].public.is_empty(),
            "the projection produced nothing"
        );
        assert!(feed_reports(&mut feeds, &batches)?.is_empty());
        assert!(tape_reports(&mut feeds, &origin)?.is_empty());
        assert_eq!(batches[0].public_gaps, vec![key]);
        assert_eq!(origin.projection_failures(), 1, "the failure is counted");
        // The projection caches were rebuilt from committed state (3 left
        // at 100), not left at the pre-trade depth.
        {
            let books = origin.books.lock().map_err(|_| "books lock".to_owned())?;
            assert_eq!(
                books.get(&RunId::new(1)).and_then(|run| run.get(&key)),
                Some(&committed_depth(&origin, key)?),
                "cache must be rebuilt after a failed projection"
            );
        }

        // The taker lifts the remaining 3 with no failure injected. The
        // direct feed's next increment must skip report 2 and the tape
        // must skip its report 2: a visible gap, never a continuous
        // sequence missing the first trade.
        submit(&origin, 3, 8, Side::Buy, 3, 100)?;
        let batches = private.drain()?;
        assert_eq!(feed_reports(&mut feeds, &batches)?, ["3", "4"]);
        assert_eq!(tape_reports(&mut feeds, &origin)?, ["3", "4"]);
        // The post-failure increment diffs against true committed state:
        // the 100 ask goes from the rebuilt 3 (not the stale 5) to 0.
        let level = batches[0].public[0]
            .levels
            .first()
            .ok_or("expected a level change")?;
        assert_eq!(level.previous, QuantityLots::new(3));
        assert_eq!(level.quantity, QuantityLots::new(0));

        // Resnapshotting after the gap gives the venue's committed book,
        // and increments after it stay correct.
        let mut resnap = PublicFeeds::default();
        assert_eq!(resnap.subscribe("feed", key, &all, true), None);
        let published_before = origin.distributor().published();
        let snapshot = committed_depth(&origin, key)?;
        assert!(snapshot.0.is_empty() && snapshot.1.is_empty());
        resnap.activate("feed", published_before, &[]);
        submit(&origin, 4, 7, Side::Sell, 4, 102)?;
        let batches = private.drain()?;
        assert_eq!(feed_reports(&mut resnap, &batches)?, ["1"]);
        assert_eq!(
            committed_depth(&origin, key)?.1,
            vec![(
                bunting_market_types::PriceTicks::new(102),
                QuantityLots::new(4)
            )],
            "resnapshotted feed tracks the committed book"
        );
        // The tape's resnapshot likewise shows the rebuilt best ask.
        let (_, quotes) = origin.tape().snapshot(InstrumentId::new(1), u64::MAX)?;
        assert_eq!(
            quotes,
            vec![(
                key,
                Side::Sell,
                bunting_market_types::PriceTicks::new(102),
                QuantityLots::new(4)
            )]
        );
        Ok(())
    }
}
