#![forbid(unsafe_code)]
//! Authoritative command, event, projection, and snapshot persistence boundary.

use bunting_ledger::{AccountProjection, HoldingProjection};
use bunting_market_events::{EventEnvelope, Side};
use bunting_market_types::{
    CommandId, EventSequence, InstrumentId, OrderId, ParticipantId, PriceBounds, PriceTicks,
    QuantityLots, RunId,
};
use bunting_risk_engine::RiskLimits;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Persisted lifecycle state for an owned order.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnedOrderState {
    /// The order has resting quantity.
    Active,
    /// The order filled completely.
    Filled,
    /// The order was canceled.
    Canceled,
}

/// Authoritative private ownership record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OwnedOrder {
    /// External Bunting identifier.
    pub order_id: OrderId,
    /// Sequential upstream identifier.
    pub upstream_order_id: u64,
    /// Owning participant.
    pub participant_id: ParticipantId,
    /// Traded instrument.
    pub instrument_id: InstrumentId,
    /// Order side.
    pub side: Side,
    /// Reservation limit price.
    pub limit_price: PriceTicks,
    /// Original order quantity.
    pub original_quantity: QuantityLots,
    /// Current unfilled quantity.
    pub remaining_quantity: QuantityLots,
    /// Current lifecycle state.
    pub state: OwnedOrderState,
}

/// Persisted participant risk configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParticipantConfig {
    /// Participant identifier.
    pub participant_id: ParticipantId,
    /// Whether new commands are enabled.
    pub enabled: bool,
    /// Exact participant limits.
    pub limits: RiskLimits,
}

/// Authoritative snapshot metadata and package.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SnapshotRecord {
    /// Instrument represented by the package.
    pub instrument_id: InstrumentId,
    /// Committed event sequence represented by the package.
    pub represented_sequence: EventSequence,
    /// Upstream SHA-256 checksum.
    pub checksum: String,
    /// Exact upstream package JSON.
    pub package_json: String,
}

/// Complete authoritative recovery projection for one run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RunState {
    /// Run identifier.
    pub run_id: RunId,
    /// Current committed event-stream version.
    pub version: EventSequence,
    /// Initial supported instrument.
    pub instrument_id: InstrumentId,
    /// Upstream book symbol.
    pub symbol: String,
    /// Inclusive price limits.
    pub price_bounds: PriceBounds,
    /// Participant admission configuration.
    pub participants: Vec<ParticipantConfig>,
    /// Exact account projection.
    pub accounts: AccountProjection,
    /// Exact holding projection.
    pub holdings: HoldingProjection,
    /// Private ownership projection.
    pub ownership: Vec<OwnedOrder>,
    /// Latest authoritative upstream package.
    pub snapshot: SnapshotRecord,
}

/// Stable persisted command response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandResult {
    /// Whether matching accepted the command.
    pub accepted: bool,
    /// Stable rejection code when rejected.
    pub reject_code: Option<String>,
    /// Highest committed event sequence represented.
    pub committed_sequence: EventSequence,
    /// External order identifier when applicable.
    pub order_id: Option<OrderId>,
    /// Snapshot checksum representing the committed state.
    pub snapshot_checksum: Option<String>,
}

/// One atomic expected-version commit request.
#[derive(Clone, Debug)]
pub struct CommitRequest {
    /// Run identifier.
    pub run_id: RunId,
    /// Command identifier.
    pub command_id: CommandId,
    /// Canonical request fingerprint.
    pub fingerprint: String,
    /// Required origin version.
    pub expected_version: EventSequence,
    /// Deterministically sequenced event batch.
    pub events: Vec<EventEnvelope>,
    /// Stable idempotency result.
    pub result: CommandResult,
    /// Complete candidate recovery projection.
    pub candidate: RunState,
}

/// Outcome of an atomic commit attempt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommitOutcome {
    /// The candidate state committed.
    Committed(CommandResult),
    /// The same canonical command was already committed.
    Duplicate(CommandResult),
}

/// Typed origin failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OriginError {
    /// The requested run does not exist.
    UnknownRun,
    /// The command identifier was reused for different bytes.
    IdempotencyConflict,
    /// The current origin version differs from the expected version.
    VersionConflict { current: EventSequence },
    /// The event batch or candidate projection is inconsistent.
    InvalidCommit,
    /// The in-memory test adapter was poisoned by a panic.
    Unavailable,
}

impl fmt::Display for OriginError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for OriginError {}

/// Domain-facing authoritative persistence contract.
pub trait OriginStore {
    /// Loads one complete recovery projection.
    ///
    /// # Errors
    /// Returns [`OriginError`] when the run is missing or storage is unavailable.
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError>;

    /// Looks up an idempotency result and its canonical fingerprint.
    ///
    /// # Errors
    /// Returns [`OriginError`] when storage cannot complete the lookup.
    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError>;

    /// Atomically commits one expected-version command transaction.
    ///
    /// # Errors
    /// Returns [`OriginError`] for conflicts, invalid candidates, or storage failures.
    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError>;
}

#[derive(Debug, Default)]
struct MemoryState {
    runs: BTreeMap<RunId, RunState>,
    commands: BTreeMap<(RunId, CommandId), (String, CommandResult)>,
    events: BTreeMap<RunId, Vec<EventEnvelope>>,
}

/// Deterministic in-memory origin adapter for native tests.
#[derive(Clone, Debug, Default)]
pub struct InMemoryOrigin {
    inner: Arc<Mutex<MemoryState>>,
}

impl InMemoryOrigin {
    /// Creates an empty origin.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts a test run before command processing begins.
    ///
    /// # Errors
    /// Returns [`OriginError::Unavailable`] when the adapter lock is poisoned.
    pub fn insert_run(&self, run: RunState) -> Result<(), OriginError> {
        let mut state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        state.runs.insert(run.run_id, run);
        Ok(())
    }

    /// Returns committed events for deterministic assertions.
    ///
    /// # Errors
    /// Returns [`OriginError::Unavailable`] when the adapter lock is poisoned.
    pub fn events(&self, run_id: RunId) -> Result<Vec<EventEnvelope>, OriginError> {
        let state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(state.events.get(&run_id).cloned().unwrap_or_default())
    }
}

impl OriginStore for InMemoryOrigin {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
        let state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        state
            .runs
            .get(&run_id)
            .cloned()
            .ok_or(OriginError::UnknownRun)
    }

    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError> {
        let state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        Ok(state.commands.get(&(run_id, command_id)).cloned())
    }

    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError> {
        let mut state = self.inner.lock().map_err(|_| OriginError::Unavailable)?;
        if let Some((fingerprint, result)) =
            state.commands.get(&(request.run_id, request.command_id))
        {
            return if fingerprint == &request.fingerprint {
                Ok(CommitOutcome::Duplicate(result.clone()))
            } else {
                Err(OriginError::IdempotencyConflict)
            };
        }
        let current = state
            .runs
            .get(&request.run_id)
            .ok_or(OriginError::UnknownRun)?
            .version;
        if current != request.expected_version {
            return Err(OriginError::VersionConflict { current });
        }
        if request.candidate.run_id != request.run_id
            || request.candidate.version != request.result.committed_sequence
            || request
                .events
                .last()
                .is_none_or(|event| event.sequence != request.candidate.version)
        {
            return Err(OriginError::InvalidCommit);
        }
        state
            .events
            .entry(request.run_id)
            .or_default()
            .extend(request.events);
        state.commands.insert(
            (request.run_id, request.command_id),
            (request.fingerprint, request.result.clone()),
        );
        state.runs.insert(request.run_id, request.candidate);
        Ok(CommitOutcome::Committed(request.result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_ledger::{Account, Holding};
    use bunting_market_events::{EVENT_SCHEMA_VERSION, EventPayload};
    use bunting_market_types::{
        CorrelationId, EventId, LogicalTimeNs, MoneyMinor, PriceTicks, QuantityLots,
    };
    use std::sync::Barrier;
    use std::thread;

    fn run() -> RunState {
        let book = bunting_orderbook_fixture();
        RunState {
            run_id: RunId::new(1),
            version: EventSequence::new(0),
            instrument_id: InstrumentId::new(1),
            symbol: "BNT/USD".to_string(),
            price_bounds: PriceBounds::new(PriceTicks(1), PriceTicks(1_000)).unwrap_or(
                PriceBounds {
                    min: PriceTicks(1),
                    max: PriceTicks(1_000),
                },
            ),
            participants: Vec::new(),
            accounts: vec![(
                ParticipantId::new(1),
                Account {
                    cash: MoneyMinor(100),
                    reserved_cash: MoneyMinor(0),
                },
            )],
            holdings: vec![(
                ParticipantId::new(1),
                InstrumentId::new(1),
                Holding {
                    position: QuantityLots(0),
                    reserved_inventory: QuantityLots(0),
                },
            )],
            ownership: Vec::new(),
            snapshot: SnapshotRecord {
                instrument_id: InstrumentId::new(1),
                represented_sequence: EventSequence::new(0),
                checksum: book.0,
                package_json: book.1,
            },
        }
    }

    fn bunting_orderbook_fixture() -> (String, String) {
        // This adapter test does not validate upstream packages; the transaction
        // layer owns that invariant.
        ("0".repeat(64), "{}".to_string())
    }

    fn request(command: u128) -> CommitRequest {
        let mut candidate = run();
        candidate.version = EventSequence::new(1);
        let event = EventEnvelope {
            schema_version: EVENT_SCHEMA_VERSION,
            run_id: RunId::new(1),
            event_id: EventId::new(command),
            sequence: EventSequence::new(1),
            logical_time: LogicalTimeNs::new(1),
            actor: ParticipantId::new(1),
            command_id: CommandId::new(command),
            correlation_id: CorrelationId::new(command),
            causation_sequence: None,
            payload: EventPayload::KillSwitchActivated,
        };
        CommitRequest {
            run_id: RunId::new(1),
            command_id: CommandId::new(command),
            fingerprint: command.to_string(),
            expected_version: EventSequence::new(0),
            events: vec![event],
            result: CommandResult {
                accepted: true,
                reject_code: None,
                committed_sequence: EventSequence::new(1),
                order_id: None,
                snapshot_checksum: None,
            },
            candidate,
        }
    }

    #[test]
    fn same_expected_version_cannot_commit_twice() {
        let origin = InMemoryOrigin::new();
        assert!(origin.insert_run(run()).is_ok());
        let barrier = Arc::new(Barrier::new(3));
        let first_origin = origin.clone();
        let first_barrier = barrier.clone();
        let first = thread::spawn(move || {
            first_barrier.wait();
            first_origin.commit(request(1))
        });
        let second_origin = origin.clone();
        let second_barrier = barrier.clone();
        let second = thread::spawn(move || {
            second_barrier.wait();
            second_origin.commit(request(2))
        });
        barrier.wait();
        let outcomes = [first.join(), second.join()];
        let committed = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(Ok(CommitOutcome::Committed(_)))))
            .count();
        let conflicted = outcomes
            .iter()
            .filter(|outcome| matches!(outcome, Ok(Err(OriginError::VersionConflict { .. }))))
            .count();
        assert_eq!((committed, conflicted), (1, 1));
    }

    #[test]
    fn persisted_event_and_result_round_trip_exactly() {
        let request = request(9);
        let event_json = serde_json::to_string(&request.events[0]);
        let result_json = serde_json::to_string(&request.result);
        assert!(event_json.is_ok());
        assert!(result_json.is_ok());
        if let (Ok(event_json), Ok(result_json)) = (event_json, result_json) {
            let event = serde_json::from_str::<EventEnvelope>(&event_json);
            let result = serde_json::from_str::<CommandResult>(&result_json);
            assert!(event.is_ok());
            assert!(result.is_ok());
            if let (Ok(event), Ok(result)) = (event, result) {
                assert_eq!(event, request.events[0]);
                assert_eq!(result, request.result);
            }
        }
    }
}
