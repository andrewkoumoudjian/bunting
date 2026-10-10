//! Bounded fan-out of committed event batches to participant sessions.
//!
//! Every successful origin commit publishes its canonical events exactly once,
//! after durability, through [`PublishingOrigin`]. Each FIX session subscribes
//! and maps only its own participant's facts, so a resting maker receives its
//! fills no matter which connection, administrator or built-in agent caused
//! them. Commits (and therefore publishes) happen under the authoritative
//! writer, so every subscriber observes batches in commit order.

use crate::admission::VenueClock;
use crate::storage::NativeOrigin;
use crate::wake::Waker;
use bunting_admission_sequencer::Endpoint;
use bunting_engine::RunState;
use bunting_market_events::{EventEnvelope, EventPayload};
use bunting_market_types::{CommandId, RunId};
use bunting_origin_store::{
    AdmissionRecord, CommandResult, Executed, JournalInput, OriginError, OriginStore,
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

/// Committed batches a session may fall behind before it is disconnected.
pub(crate) const MAX_PENDING_BATCHES: usize = 4_096;

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

struct Subscriber {
    sender: SyncSender<CommittedBatch>,
    overflowed: Arc<AtomicBool>,
    waker: Option<Waker>,
}

/// Registry of bounded per-session queues.
pub(crate) struct EventDistributor {
    capacity: usize,
    next_id: AtomicU64,
    subscribers: Mutex<BTreeMap<u64, Subscriber>>,
}

impl EventDistributor {
    pub(crate) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            next_id: AtomicU64::new(0),
            subscribers: Mutex::new(BTreeMap::new()),
        }
    }

    /// Registers a session; batches committed from now on are queued for it.
    pub(crate) fn subscribe(&self, waker: Option<Waker>) -> Result<Subscription<'_>, String> {
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
            distributor: self,
            id,
            receiver,
            overflowed,
        })
    }

    /// Queues one committed batch for every subscriber without blocking the
    /// writer. A full queue marks that subscriber overflowed and drops it; the
    /// session then disconnects instead of silently skipping reports.
    fn publish(&self, events: &[EventEnvelope], committed_us: u64, source: Endpoint) {
        if events.is_empty() {
            return;
        }
        let batch: CommittedBatch = Arc::new(Committed {
            durable_us: committed_us,
            source,
            events: events.to_vec(),
        });
        let Ok(mut subscribers) = self.subscribers.lock() else {
            return;
        };
        subscribers.retain(|_, subscriber| {
            let keep = match subscriber.sender.try_send(batch.clone()) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => {
                    subscriber.overflowed.store(true, Ordering::Release);
                    false
                }
                Err(TrySendError::Disconnected(_)) => false,
            };
            // Wake the session for the batch, or to notice the overflow.
            if let Some(waker) = &subscriber.waker {
                waker.wake();
            }
            keep
        });
    }

    #[cfg(test)]
    fn subscriber_count(&self) -> usize {
        self.subscribers
            .lock()
            .map_or(0, |subscribers| subscribers.len())
    }
}

/// One session's ordered view of committed batches.
pub(crate) struct Subscription<'a> {
    distributor: &'a EventDistributor,
    id: u64,
    receiver: Receiver<CommittedBatch>,
    overflowed: Arc<AtomicBool>,
}

impl Subscription<'_> {
    /// Returns every batch queued so far, in commit order, without blocking.
    ///
    /// # Errors
    /// Returns an error once the queue has overflowed; the caller must
    /// disconnect so the participant recovers state rather than miss a report.
    pub(crate) fn drain(&self) -> Result<Vec<CommittedBatch>, String> {
        if self.overflowed.load(Ordering::Acquire) {
            return Err(format!(
                "committed report queue overflow: more than {} pending batches; reconnect to recover",
                self.distributor.capacity
            ));
        }
        let mut batches = Vec::new();
        loop {
            match self.receiver.try_recv() {
                Ok(batch) => batches.push(batch),
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return Ok(batches),
            }
        }
    }
}

impl Drop for Subscription<'_> {
    fn drop(&mut self) {
        if let Ok(mut subscribers) = self.distributor.subscribers.lock() {
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
}

impl PublishingOrigin {
    pub(crate) fn new(inner: NativeOrigin, capacity: usize, clock: VenueClock) -> Self {
        Self {
            inner,
            distributor: EventDistributor::new(capacity),
            clock,
        }
    }

    pub(crate) const fn inner(&self) -> &NativeOrigin {
        &self.inner
    }

    pub(crate) const fn distributor(&self) -> &EventDistributor {
        &self.distributor
    }
}

impl OriginStore for PublishingOrigin {
    fn execute_admitted(
        &self,
        input: &JournalInput,
        admission: Option<&AdmissionRecord>,
    ) -> Result<Executed, OriginError> {
        let executed = self.inner.execute_admitted(input, admission)?;
        // A duplicate's events were published when it first committed.
        if !executed.duplicate {
            self.distributor.publish(
                &executed.events,
                self.clock.now_us(),
                batch_source(admission, &executed.events),
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
        distributor.publish(&[event(1)], 1, Endpoint::Hub);
        let second = distributor.subscribe(None)?;
        distributor.publish(&[event(2), event(3)], 2, Endpoint::Hub);
        distributor.publish(&[], 3, Endpoint::Hub);
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
            distributor.publish(&[event(sequence)], sequence, Endpoint::Hub);
        }
        assert_eq!(fast.drain()?.len(), 2);
        distributor.publish(&[event(3)], 3, Endpoint::Hub);
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
}
