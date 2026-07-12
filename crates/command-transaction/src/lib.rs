#![forbid(unsafe_code)]
//! Pure command orchestration across recovery, risk, matching, and origin commit.

use bunting_ledger::{Ledger, LedgerError, TradeSettlement};
use bunting_market_events::{
    CancelReason, Command, CommandPayload, EVENT_SCHEMA_VERSION, EventEnvelope, EventPayload,
    OrderKind, RejectCode, Side,
};
use bunting_market_types::{EventId, EventSequence, OrderId, PriceTicks, QuantityLots};
use bunting_orderbook::{
    KernelBook, SnapshotPackage, TimeInForce, TradeInfo, sequential_id_from_text,
    to_upstream_order_id, to_upstream_price, to_upstream_quantity, to_upstream_side,
};
use bunting_origin_store::{
    CommandResult, CommitOutcome, CommitRequest, OriginError, OriginStore, OwnedOrder,
    OwnedOrderState, RunState, SnapshotRecord,
};
use bunting_risk_engine::RiskState;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Maximum accepted event facts from one initial command.
pub const MAX_EVENTS_PER_COMMAND: usize = 256;
/// Maximum snapshot depth used by the initial vertical slice.
pub const SNAPSHOT_DEPTH: usize = 10_000;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// Cache entry returned by a snapshot adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CachedSnapshot {
    /// Represented committed sequence.
    pub represented_sequence: EventSequence,
    /// Upstream checksum.
    pub checksum: String,
    /// Upstream package JSON.
    pub package_json: String,
}

/// Typed cache adapter failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotCacheError {
    /// The cache operation failed.
    Unavailable,
}

/// Recoverable immutable snapshot-cache boundary.
pub trait SnapshotCache {
    /// Reads the key identified by authoritative metadata.
    ///
    /// # Errors
    /// Returns [`SnapshotCacheError`] when the cache cannot complete the read.
    fn get(&self, snapshot: &SnapshotRecord) -> Result<Option<CachedSnapshot>, SnapshotCacheError>;

    /// Writes one committed immutable package.
    ///
    /// # Errors
    /// Returns [`SnapshotCacheError`] when the cache cannot complete the write.
    fn put(&self, snapshot: &SnapshotRecord) -> Result<(), SnapshotCacheError>;
}

#[derive(Debug, Default)]
struct MemoryCacheState {
    entries: BTreeMap<(EventSequence, String), CachedSnapshot>,
    fail_put: bool,
}

/// Deterministic cache adapter for native integration tests.
#[derive(Clone, Debug, Default)]
pub struct InMemorySnapshotCache {
    inner: Arc<Mutex<MemoryCacheState>>,
}

impl InMemorySnapshotCache {
    /// Creates an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts an arbitrary entry to exercise corruption recovery.
    ///
    /// # Errors
    /// Returns [`SnapshotCacheError::Unavailable`] when the test adapter is poisoned.
    pub fn insert(&self, entry: CachedSnapshot) -> Result<(), SnapshotCacheError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| SnapshotCacheError::Unavailable)?;
        state
            .entries
            .insert((entry.represented_sequence, entry.checksum.clone()), entry);
        Ok(())
    }

    /// Configures cache puts to fail after origin commit.
    ///
    /// # Errors
    /// Returns [`SnapshotCacheError::Unavailable`] when the test adapter is poisoned.
    pub fn set_fail_put(&self, fail: bool) -> Result<(), SnapshotCacheError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| SnapshotCacheError::Unavailable)?;
        state.fail_put = fail;
        Ok(())
    }

    /// Returns the number of immutable entries held by the test adapter.
    ///
    /// # Errors
    /// Returns [`SnapshotCacheError::Unavailable`] when the test adapter is poisoned.
    pub fn len(&self) -> Result<usize, SnapshotCacheError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| SnapshotCacheError::Unavailable)?;
        Ok(state.entries.len())
    }

    /// Returns whether the test adapter is empty.
    ///
    /// # Errors
    /// Returns [`SnapshotCacheError::Unavailable`] when the test adapter is poisoned.
    pub fn is_empty(&self) -> Result<bool, SnapshotCacheError> {
        self.len().map(|length| length == 0)
    }
}

impl SnapshotCache for InMemorySnapshotCache {
    fn get(&self, snapshot: &SnapshotRecord) -> Result<Option<CachedSnapshot>, SnapshotCacheError> {
        let state = self
            .inner
            .lock()
            .map_err(|_| SnapshotCacheError::Unavailable)?;
        Ok(state
            .entries
            .get(&(snapshot.represented_sequence, snapshot.checksum.clone()))
            .cloned())
    }

    fn put(&self, snapshot: &SnapshotRecord) -> Result<(), SnapshotCacheError> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| SnapshotCacheError::Unavailable)?;
        if state.fail_put {
            return Err(SnapshotCacheError::Unavailable);
        }
        let entry = CachedSnapshot {
            represented_sequence: snapshot.represented_sequence,
            checksum: snapshot.checksum.clone(),
            package_json: snapshot.package_json.clone(),
        };
        state
            .entries
            .insert((entry.represented_sequence, entry.checksum.clone()), entry);
        Ok(())
    }
}

/// Typed command-transaction failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionError {
    /// Authoritative persistence failed or rejected the commit.
    Origin(OriginError),
    /// The cached and authoritative snapshot packages were both invalid.
    InvalidOriginSnapshot,
    /// A command reused an identifier with a different canonical payload.
    IdempotencyConflict,
    /// Matching produced an ownership reference that cannot be recovered.
    OwnershipInvariant,
    /// Checked accounting failed.
    Accounting,
    /// The canonical event batch exceeded its bound.
    EventBatchTooLarge,
    /// Serialization or fingerprinting failed.
    Serialization,
    /// Upstream matching failed unexpectedly.
    Upstream,
}

impl fmt::Display for TransactionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for TransactionError {}

impl From<OriginError> for TransactionError {
    fn from(error: OriginError) -> Self {
        match error {
            OriginError::IdempotencyConflict => Self::IdempotencyConflict,
            other => Self::Origin(other),
        }
    }
}

impl From<LedgerError> for TransactionError {
    fn from(_: LedgerError) -> Self {
        Self::Accounting
    }
}

/// Complete command transaction over explicit origin and cache boundaries.
#[derive(Debug)]
pub struct CommandTransaction<'a, O, C> {
    origin: &'a O,
    cache: &'a C,
}

/// Candidate transaction prepared without performing authoritative I/O.
#[derive(Clone, Debug)]
pub struct PreparedCommand {
    /// Atomic origin commit request.
    pub commit: CommitRequest,
}

impl<'a, O, C> CommandTransaction<'a, O, C>
where
    O: OriginStore,
    C: SnapshotCache,
{
    /// Creates an orchestrator over explicit adapters.
    #[must_use]
    pub const fn new(origin: &'a O, cache: &'a C) -> Self {
        Self { origin, cache }
    }

    /// Executes one authenticated canonical command.
    ///
    /// Cache reads and writes are recoverable. The accepted response is returned
    /// only after the authoritative expected-version commit succeeds.
    ///
    /// # Errors
    /// Returns [`TransactionError`] for conflicts, invalid recovery state, or processing failure.
    pub fn execute(&self, command: &Command) -> Result<CommandResult, TransactionError> {
        let fingerprint = command_fingerprint(command)?;
        if let Some((stored_fingerprint, result)) = self
            .origin
            .find_command(command.run_id, command.command_id)?
        {
            return if stored_fingerprint == fingerprint {
                Ok(result)
            } else {
                Err(TransactionError::IdempotencyConflict)
            };
        }

        let candidate = self.origin.load_run(command.run_id)?;
        if candidate.version != command.expected_sequence {
            return Err(TransactionError::Origin(OriginError::VersionConflict {
                current: candidate.version,
            }));
        }
        let cached = self.cache.get(&candidate.snapshot).ok().flatten();
        let prepared = prepare_command(command, candidate, cached)?;
        let snapshot = prepared.commit.candidate.snapshot.clone();
        let outcome = self.origin.commit(prepared.commit)?;
        let committed = match outcome {
            CommitOutcome::Committed(committed) | CommitOutcome::Duplicate(committed) => committed,
        };
        let _cache_put = self.cache.put(&snapshot);
        Ok(committed)
    }
}

/// Prepares deterministic candidate state from explicit recovery inputs.
///
/// This function performs no origin or cache I/O. Platform adapters can load
/// D1 and Workers Cache asynchronously, call this function, then atomically
/// commit [`PreparedCommand::commit`] before publishing the result.
///
/// # Errors
/// Returns [`TransactionError`] when recovery, risk, matching, or accounting fails.
pub fn prepare_command(
    command: &Command,
    mut candidate: RunState,
    cached: Option<CachedSnapshot>,
) -> Result<PreparedCommand, TransactionError> {
    if candidate.run_id != command.run_id || candidate.version != command.expected_sequence {
        return Err(TransactionError::Origin(OriginError::VersionConflict {
            current: candidate.version,
        }));
    }
    let logical_millis = command.logical_time.get() / 1_000_000;
    let book = restore_book(cached, &candidate, logical_millis)?;
    let mut ledger =
        Ledger::from_projection(candidate.accounts.clone(), candidate.holdings.clone());
    let risk = restore_risk(&candidate);
    let mut ownership: BTreeMap<OrderId, OwnedOrder> = candidate
        .ownership
        .iter()
        .cloned()
        .map(|order| (order.order_id, order))
        .collect();
    let mut payloads = Vec::new();
    let (accepted, reject_code, order_id) = match &command.payload {
        CommandPayload::SubmitOrder(order) => {
            payloads.push(EventPayload::OrderReceived {
                order: order.clone(),
            });
            match prepare_submit(
                order,
                &book,
                &mut ledger,
                &risk,
                &mut ownership,
                &mut payloads,
            )? {
                Ok(()) => (true, None, Some(order.order_id)),
                Err(code) => {
                    payloads.push(EventPayload::OrderRejected {
                        order_id: Some(order.order_id),
                        code,
                    });
                    (false, Some(format!("{code:?}")), Some(order.order_id))
                }
            }
        }
        CommandPayload::CancelOrder(cancel) => {
            match prepare_cancel(cancel, &book, &mut ledger, &mut ownership, &mut payloads)? {
                Ok(()) => (true, None, Some(cancel.order_id)),
                Err(code) => {
                    payloads.push(EventPayload::OrderRejected {
                        order_id: Some(cancel.order_id),
                        code,
                    });
                    (false, Some(format!("{code:?}")), Some(cancel.order_id))
                }
            }
        }
        CommandPayload::ActivateKillSwitch => {
            book.engage_kill_switch();
            payloads.push(EventPayload::KillSwitchActivated);
            (true, None, None)
        }
    };
    if payloads.len() > MAX_EVENTS_PER_COMMAND {
        return Err(TransactionError::EventBatchTooLarge);
    }
    let events = envelope(command, payloads)?;
    let committed_sequence = events
        .last()
        .map_or(command.expected_sequence, |event| event.sequence);
    let snapshot = book
        .snapshot_package(SNAPSHOT_DEPTH)
        .map_err(|_| TransactionError::Upstream)?;
    candidate.version = committed_sequence;
    (candidate.accounts, candidate.holdings) = ledger.projection();
    candidate.ownership = ownership.into_values().collect();
    candidate.snapshot = snapshot_record(candidate.instrument_id, committed_sequence, snapshot);
    let result = CommandResult {
        accepted,
        reject_code,
        committed_sequence,
        order_id,
        snapshot_checksum: Some(candidate.snapshot.checksum.clone()),
    };
    Ok(PreparedCommand {
        commit: CommitRequest {
            run_id: command.run_id,
            command_id: command.command_id,
            fingerprint: command_fingerprint(command)?,
            expected_version: command.expected_sequence,
            events,
            result: result.clone(),
            candidate,
        },
    })
}

/// Computes the canonical payload fingerprint used by durable idempotency.
///
/// # Errors
/// Returns [`TransactionError::Serialization`] when canonical encoding fails.
pub fn command_fingerprint(command: &Command) -> Result<String, TransactionError> {
    let bytes = serde_json::to_vec(command).map_err(|_| TransactionError::Serialization)?;
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        output.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(HEX_DIGITS[usize::from(byte & 0x0f)]));
    }
    Ok(output)
}

fn restore_book(
    cached: Option<CachedSnapshot>,
    state: &RunState,
    logical_millis: u64,
) -> Result<KernelBook, TransactionError> {
    if let Some(entry) = cached
        && entry.represented_sequence == state.version
        && entry.represented_sequence == state.snapshot.represented_sequence
        && entry.checksum == state.snapshot.checksum
        && let Ok(book) =
            KernelBook::restore_snapshot_json_at(&state.symbol, &entry.package_json, logical_millis)
    {
        return Ok(book);
    }
    if state.snapshot.represented_sequence != state.version {
        return Err(TransactionError::InvalidOriginSnapshot);
    }
    KernelBook::restore_snapshot_json_at(
        &state.symbol,
        &state.snapshot.package_json,
        logical_millis,
    )
    .map_err(|_| TransactionError::InvalidOriginSnapshot)
}

fn restore_risk(state: &RunState) -> RiskState {
    let mut risk = RiskState::new();
    risk.configure_instrument(state.instrument_id, state.price_bounds);
    for participant in &state.participants {
        risk.configure_participant(participant.participant_id, participant.limits);
        risk.set_enabled(participant.participant_id, participant.enabled);
    }
    risk
}

fn prepare_submit(
    order: &bunting_market_events::SubmitOrder,
    book: &KernelBook,
    ledger: &mut Ledger,
    risk: &RiskState,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<Result<(), RejectCode>, TransactionError> {
    if ownership.contains_key(&order.order_id) {
        return Ok(Err(RejectCode::DuplicateOrderId));
    }
    let OrderKind::Limit { price } = order.kind else {
        return Ok(Err(RejectCode::PriceOutOfBounds));
    };
    let Ok(upstream_id) = to_upstream_order_id(order.order_id) else {
        return Ok(Err(RejectCode::InvalidOrderId));
    };
    let Ok(upstream_price) = to_upstream_price(price) else {
        return Ok(Err(RejectCode::PriceOutOfBounds));
    };
    let Ok(upstream_quantity) = to_upstream_quantity(order.quantity) else {
        return Ok(Err(RejectCode::InvalidQuantity));
    };
    let open_quantity = ownership
        .values()
        .filter(|owned| {
            owned.participant_id == order.participant_id
                && owned.instrument_id == order.instrument_id
                && owned.state == OwnedOrderState::Active
        })
        .try_fold(QuantityLots(0), |total, owned| {
            total.checked_add(owned.remaining_quantity)
        })
        .ok_or(TransactionError::Accounting)?;
    let reservation_price = match risk.check(order, open_quantity, ledger, None) {
        Ok(value) => value,
        Err(code) => return Ok(Err(code)),
    };
    ledger.reserve(
        order.participant_id,
        order.instrument_id,
        order.side,
        reservation_price,
        order.quantity,
    )?;
    ownership.insert(
        order.order_id,
        OwnedOrder {
            order_id: order.order_id,
            upstream_order_id: upstream_id,
            participant_id: order.participant_id,
            instrument_id: order.instrument_id,
            side: order.side,
            limit_price: price,
            original_quantity: order.quantity,
            remaining_quantity: order.quantity,
            state: OwnedOrderState::Active,
        },
    );
    let submission = book
        .submit_limit(
            upstream_id,
            upstream_price,
            upstream_quantity,
            to_upstream_side(order.side),
            TimeInForce::Gtc,
        )
        .map_err(|_| TransactionError::Upstream)?;
    payloads.push(EventPayload::OrderAccepted {
        order_id: order.order_id,
    });
    if let Some(trade_result) = submission.trade_result {
        let engine_sequence = trade_result.engine_seq;
        let trade_info = TradeInfo::from_trade_result(&trade_result, None);
        apply_trades(
            order.order_id,
            engine_sequence,
            &trade_info,
            ledger,
            ownership,
            payloads,
        )?;
    }
    let remaining = ownership
        .get(&order.order_id)
        .ok_or(TransactionError::OwnershipInvariant)?
        .remaining_quantity;
    if remaining.get() == 0 {
        if let Some(taker) = ownership.get_mut(&order.order_id) {
            taker.state = OwnedOrderState::Filled;
        }
        payloads.push(EventPayload::OrderCompleted {
            order_id: order.order_id,
        });
    } else {
        payloads.push(EventPayload::OrderRested {
            order_id: order.order_id,
            participant_id: order.participant_id,
            instrument_id: order.instrument_id,
            side: order.side,
            price,
            remaining,
        });
    }
    Ok(Ok(()))
}

fn apply_trades(
    taker_order_id: OrderId,
    engine_sequence: u64,
    trade_info: &TradeInfo,
    ledger: &mut Ledger,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<(), TransactionError> {
    for transaction in &trade_info.transactions {
        let maker_upstream = sequential_id_from_text(&transaction.maker_order_id)
            .ok_or(TransactionError::OwnershipInvariant)?;
        let maker_id = ownership
            .values()
            .find(|owned| owned.upstream_order_id == maker_upstream)
            .map(|owned| owned.order_id)
            .ok_or(TransactionError::OwnershipInvariant)?;
        let maker = ownership
            .get(&maker_id)
            .cloned()
            .ok_or(TransactionError::OwnershipInvariant)?;
        let taker = ownership
            .get(&taker_order_id)
            .cloned()
            .ok_or(TransactionError::OwnershipInvariant)?;
        let quantity_i64 =
            i64::try_from(transaction.quantity).map_err(|_| TransactionError::Accounting)?;
        let price_i64 =
            i64::try_from(transaction.price).map_err(|_| TransactionError::Accounting)?;
        let quantity = QuantityLots(quantity_i64);
        let execution_price = PriceTicks(price_i64);
        let (buyer, seller, buyer_limit, seller_limit) = if taker.side == Side::Buy {
            (
                taker.participant_id,
                maker.participant_id,
                taker.limit_price,
                maker.limit_price,
            )
        } else {
            (
                maker.participant_id,
                taker.participant_id,
                maker.limit_price,
                taker.limit_price,
            )
        };
        ledger.settle_trade(TradeSettlement {
            buyer,
            seller,
            instrument: taker.instrument_id,
            buyer_limit,
            seller_limit,
            execution_price,
            quantity,
        })?;
        reduce_order(maker_id, quantity, ownership, payloads)?;
        reduce_order(taker_order_id, quantity, ownership, &mut Vec::new())?;
        payloads.push(EventPayload::TradeExecuted {
            instrument_id: taker.instrument_id,
            maker_order_id: maker_id,
            taker_order_id,
            buyer_id: buyer,
            seller_id: seller,
            price: execution_price,
            quantity,
            upstream_engine_sequence: engine_sequence,
        });
    }
    Ok(())
}

fn reduce_order(
    order_id: OrderId,
    quantity: QuantityLots,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<(), TransactionError> {
    let owned = ownership
        .get_mut(&order_id)
        .ok_or(TransactionError::OwnershipInvariant)?;
    owned.remaining_quantity = owned
        .remaining_quantity
        .checked_sub(quantity)
        .filter(|remaining| remaining.get() >= 0)
        .ok_or(TransactionError::Accounting)?;
    if owned.remaining_quantity.get() == 0 {
        owned.state = OwnedOrderState::Filled;
        payloads.push(EventPayload::OrderCompleted { order_id });
    } else {
        payloads.push(EventPayload::OrderReduced {
            order_id,
            remaining: owned.remaining_quantity,
        });
    }
    Ok(())
}

fn prepare_cancel(
    cancel: &bunting_market_events::CancelOrder,
    book: &KernelBook,
    ledger: &mut Ledger,
    ownership: &mut BTreeMap<OrderId, OwnedOrder>,
    payloads: &mut Vec<EventPayload>,
) -> Result<Result<(), RejectCode>, TransactionError> {
    let Some(owned) = ownership.get(&cancel.order_id).cloned() else {
        return Ok(Err(RejectCode::UnknownOrder));
    };
    if owned.participant_id != cancel.participant_id {
        return Ok(Err(RejectCode::NotOrderOwner));
    }
    if owned.state != OwnedOrderState::Active {
        return Ok(Err(RejectCode::UnknownOrder));
    }
    let canceled = book
        .cancel_remaining(owned.upstream_order_id)
        .map_err(|_| TransactionError::Upstream)?;
    let Some(upstream_remaining) = canceled else {
        return Err(TransactionError::OwnershipInvariant);
    };
    if upstream_remaining
        != u64::try_from(owned.remaining_quantity.get())
            .map_err(|_| TransactionError::Accounting)?
    {
        return Err(TransactionError::OwnershipInvariant);
    }
    ledger.release(
        owned.participant_id,
        owned.instrument_id,
        owned.side,
        owned.limit_price,
        owned.remaining_quantity,
    )?;
    if let Some(record) = ownership.get_mut(&cancel.order_id) {
        record.state = OwnedOrderState::Canceled;
        record.remaining_quantity = QuantityLots(0);
    }
    payloads.push(EventPayload::OrderCanceled {
        order_id: owned.order_id,
        participant_id: owned.participant_id,
        instrument_id: owned.instrument_id,
        remaining: owned.remaining_quantity,
        reason: CancelReason::Requested,
    });
    Ok(Ok(()))
}

fn envelope(
    command: &Command,
    payloads: Vec<EventPayload>,
) -> Result<Vec<EventEnvelope>, TransactionError> {
    payloads
        .into_iter()
        .enumerate()
        .map(|(index, payload)| {
            let offset =
                u64::try_from(index + 1).map_err(|_| TransactionError::EventBatchTooLarge)?;
            let sequence = command
                .expected_sequence
                .get()
                .checked_add(offset)
                .map(EventSequence::new)
                .ok_or(TransactionError::EventBatchTooLarge)?;
            let event_id = command
                .command_id
                .get()
                .checked_add(u128::from(offset))
                .map(EventId::new)
                .ok_or(TransactionError::EventBatchTooLarge)?;
            Ok(EventEnvelope {
                schema_version: EVENT_SCHEMA_VERSION,
                run_id: command.run_id,
                event_id,
                sequence,
                logical_time: command.logical_time,
                actor: command.actor,
                command_id: command.command_id,
                correlation_id: command.correlation_id,
                causation_sequence: None,
                payload,
            })
        })
        .collect()
}

fn snapshot_record(
    instrument_id: bunting_market_types::InstrumentId,
    sequence: EventSequence,
    package: SnapshotPackage,
) -> SnapshotRecord {
    SnapshotRecord {
        instrument_id,
        represented_sequence: sequence,
        checksum: package.checksum,
        package_json: package.json,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_ledger::{Account, Holding};
    use bunting_market_events::{CancelOrder, SubmitOrder};
    use bunting_market_types::{
        CommandId, CorrelationId, InstrumentId, LogicalTimeNs, MoneyMinor, ParticipantId,
        PriceBounds, RunId,
    };
    use bunting_origin_store::{CommitRequest, InMemoryOrigin, OriginStore, ParticipantConfig};
    use bunting_risk_engine::RiskLimits;

    fn participant(id: u128) -> ParticipantConfig {
        ParticipantConfig {
            participant_id: ParticipantId::new(id),
            enabled: true,
            limits: RiskLimits {
                max_order_quantity: QuantityLots(100),
                max_open_order_quantity: QuantityLots(1_000),
                max_absolute_position: QuantityLots(1_000),
            },
        }
    }

    fn setup() -> (InMemoryOrigin, InMemorySnapshotCache) {
        let origin = InMemoryOrigin::new();
        let cache = InMemorySnapshotCache::new();
        let book = KernelBook::new_at("BNT/USD", 0);
        let package = book.snapshot_package(SNAPSHOT_DEPTH);
        assert!(package.is_ok());
        if let Ok(package) = package {
            let run = RunState {
                run_id: RunId::new(1),
                version: EventSequence::new(0),
                instrument_id: InstrumentId::new(1),
                symbol: "BNT/USD".to_string(),
                price_bounds: PriceBounds {
                    min: PriceTicks(1),
                    max: PriceTicks(1_000),
                },
                participants: vec![participant(1), participant(2)],
                accounts: vec![
                    (
                        ParticipantId::new(1),
                        Account {
                            cash: MoneyMinor(100_000),
                            reserved_cash: MoneyMinor(0),
                        },
                    ),
                    (
                        ParticipantId::new(2),
                        Account {
                            cash: MoneyMinor(100_000),
                            reserved_cash: MoneyMinor(0),
                        },
                    ),
                ],
                holdings: vec![
                    (
                        ParticipantId::new(1),
                        InstrumentId::new(1),
                        Holding {
                            position: QuantityLots(100),
                            reserved_inventory: QuantityLots(0),
                        },
                    ),
                    (
                        ParticipantId::new(2),
                        InstrumentId::new(1),
                        Holding {
                            position: QuantityLots(100),
                            reserved_inventory: QuantityLots(0),
                        },
                    ),
                ],
                ownership: Vec::new(),
                snapshot: snapshot_record(InstrumentId::new(1), EventSequence::new(0), package),
            };
            assert!(origin.insert_run(run).is_ok());
        }
        (origin, cache)
    }

    fn submit(
        command: u128,
        expected: u64,
        participant_id: u128,
        order_id: u128,
        side: Side,
        price: i64,
        quantity: i64,
    ) -> Command {
        Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(command),
            correlation_id: CorrelationId::new(command),
            logical_time: LogicalTimeNs::new(
                u64::try_from(command)
                    .unwrap_or(0)
                    .saturating_mul(1_000_000),
            ),
            expected_sequence: EventSequence::new(expected),
            actor: ParticipantId::new(participant_id),
            payload: CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(order_id),
                instrument_id: InstrumentId::new(1),
                participant_id: ParticipantId::new(participant_id),
                side,
                quantity: QuantityLots(quantity),
                kind: OrderKind::Limit {
                    price: PriceTicks(price),
                },
            }),
        }
    }

    fn cancel(
        command: u128,
        expected: EventSequence,
        participant_id: u128,
        order_id: u128,
    ) -> Command {
        Command {
            run_id: RunId::new(1),
            command_id: CommandId::new(command),
            correlation_id: CorrelationId::new(command),
            logical_time: LogicalTimeNs::new(
                u64::try_from(command)
                    .unwrap_or(0)
                    .saturating_mul(1_000_000),
            ),
            expected_sequence: expected,
            actor: ParticipantId::new(participant_id),
            payload: CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(order_id),
                participant_id: ParticipantId::new(participant_id),
            }),
        }
    }

    #[test]
    fn cache_miss_duplicate_cross_and_cancel_are_transactional() {
        let (origin, cache) = setup();
        let transaction = CommandTransaction::new(&origin, &cache);
        let sell = submit(10, 0, 1, 1, Side::Sell, 100, 10);
        let rested = transaction.execute(&sell);
        assert!(matches!(rested, Ok(CommandResult { accepted: true, .. })));
        let duplicate = transaction.execute(&sell);
        assert_eq!(rested, duplicate);
        let expected = rested
            .map(|result| result.committed_sequence.get())
            .unwrap_or(0);
        let buy = submit(20, expected, 2, 2, Side::Buy, 110, 4);
        let crossed = transaction.execute(&buy);
        assert!(
            matches!(crossed, Ok(CommandResult { accepted: true, .. })),
            "crossed={crossed:?}"
        );
        if let Ok(result) = crossed {
            let cancel = Command {
                run_id: RunId::new(1),
                command_id: CommandId::new(30),
                correlation_id: CorrelationId::new(30),
                logical_time: LogicalTimeNs::new(30_000_000),
                expected_sequence: result.committed_sequence,
                actor: ParticipantId::new(1),
                payload: CommandPayload::CancelOrder(CancelOrder {
                    order_id: OrderId::new(1),
                    participant_id: ParticipantId::new(1),
                }),
            };
            assert!(matches!(
                transaction.execute(&cancel),
                Ok(CommandResult { accepted: true, .. })
            ));
        }
    }

    #[test]
    fn corrupt_cache_falls_back_and_cache_put_failure_does_not_hide_commit() {
        let (origin, cache) = setup();
        let run = origin.load_run(RunId::new(1));
        assert!(run.is_ok());
        if let Ok(run) = run {
            assert!(
                cache
                    .insert(CachedSnapshot {
                        represented_sequence: run.version,
                        checksum: run.snapshot.checksum,
                        package_json: "corrupt".to_string()
                    })
                    .is_ok()
            );
        }
        assert!(cache.set_fail_put(true).is_ok());
        let result = CommandTransaction::new(&origin, &cache).execute(&submit(
            1,
            0,
            1,
            1,
            Side::Buy,
            100,
            2,
        ));
        assert!(matches!(result, Ok(CommandResult { accepted: true, .. })));
    }

    #[test]
    fn version_idempotency_numeric_risk_and_owner_conflicts_are_typed() {
        let (origin, cache) = setup();
        let transaction = CommandTransaction::new(&origin, &cache);
        assert!(matches!(
            transaction.execute(&submit(1, 9, 1, 1, Side::Buy, 100, 1)),
            Err(TransactionError::Origin(
                OriginError::VersionConflict { .. }
            ))
        ));
        let invalid = transaction.execute(&submit(
            2,
            0,
            1,
            u128::from(u64::MAX) + 1,
            Side::Buy,
            100,
            1,
        ));
        assert!(matches!(
            invalid,
            Ok(CommandResult {
                accepted: false,
                ..
            })
        ));
        let too_large = transaction.execute(&submit(
            3,
            invalid.map(|r| r.committed_sequence.get()).unwrap_or(0),
            1,
            3,
            Side::Buy,
            100,
            101,
        ));
        assert!(matches!(
            too_large,
            Ok(CommandResult {
                accepted: false,
                ..
            })
        ));
    }

    #[test]
    fn partial_fill_accounting_owner_cancel_and_recovery_are_exact() {
        let (origin, cache) = setup();
        let transaction = CommandTransaction::new(&origin, &cache);
        let sell_result = transaction.execute(&submit(10, 0, 1, 1, Side::Sell, 100, 10));
        assert!(sell_result.is_ok());
        let sell_sequence = sell_result
            .map(|result| result.committed_sequence)
            .unwrap_or_default();
        let buy_result =
            transaction.execute(&submit(20, sell_sequence.get(), 2, 2, Side::Buy, 110, 4));
        assert!(buy_result.is_ok());
        let buy_sequence = buy_result
            .map(|result| result.committed_sequence)
            .unwrap_or_default();
        let recovered = origin.load_run(RunId::new(1));
        assert!(recovered.is_ok());
        if let Ok(state) = recovered {
            let maker = state
                .ownership
                .iter()
                .find(|order| order.order_id == OrderId::new(1));
            assert!(matches!(maker, Some(order) if order.remaining_quantity == QuantityLots(6)));
            let ledger = Ledger::from_projection(state.accounts, state.holdings);
            assert_eq!(
                ledger.account(ParticipantId::new(2)).cash,
                MoneyMinor(99_600)
            );
            assert_eq!(
                ledger
                    .holding(ParticipantId::new(2), InstrumentId::new(1))
                    .position,
                QuantityLots(104)
            );
            assert_eq!(
                ledger
                    .holding(ParticipantId::new(1), InstrumentId::new(1))
                    .reserved_inventory,
                QuantityLots(6)
            );
        }
        let foreign_cancel = transaction.execute(&cancel(30, buy_sequence, 2, 1));
        assert!(matches!(
            foreign_cancel,
            Ok(CommandResult {
                accepted: false,
                ..
            })
        ));
        let cancel_sequence = foreign_cancel
            .map(|result| result.committed_sequence)
            .unwrap_or_default();
        let owner_cancel = transaction.execute(&cancel(40, cancel_sequence, 1, 1));
        assert!(matches!(
            owner_cancel,
            Ok(CommandResult { accepted: true, .. })
        ));
        if let Ok(state) = origin.load_run(RunId::new(1)) {
            let ledger = Ledger::from_projection(state.accounts, state.holdings);
            assert_eq!(
                ledger
                    .holding(ParticipantId::new(1), InstrumentId::new(1))
                    .reserved_inventory,
                QuantityLots(0)
            );
        }
    }

    #[test]
    fn command_id_conflict_and_serialization_preserve_exact_values() {
        let (origin, cache) = setup();
        let transaction = CommandTransaction::new(&origin, &cache);
        let command = submit(1, 0, 1, 1, Side::Buy, 100, 1);
        assert!(transaction.execute(&command).is_ok());
        let mut conflicting = command.clone();
        if let CommandPayload::SubmitOrder(order) = &mut conflicting.payload {
            order.kind = OrderKind::Limit {
                price: PriceTicks(101),
            };
        }
        assert_eq!(
            transaction.execute(&conflicting),
            Err(TransactionError::IdempotencyConflict)
        );
        let exact = submit(u128::MAX, 0, 1, u128::from(u64::MAX), Side::Buy, 100, 1);
        let encoded = serde_json::to_string(&exact);
        assert!(encoded.is_ok());
        if let Ok(encoded) = encoded {
            let decoded = serde_json::from_str::<Command>(&encoded);
            assert!(decoded.is_ok());
            if let Ok(decoded) = decoded {
                assert_eq!(decoded, exact);
            }
        }
    }

    #[derive(Debug)]
    struct FailingCommitOrigin(InMemoryOrigin);

    impl OriginStore for FailingCommitOrigin {
        fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError> {
            self.0.load_run(run_id)
        }

        fn find_command(
            &self,
            run_id: RunId,
            command_id: CommandId,
        ) -> Result<Option<(String, CommandResult)>, OriginError> {
            self.0.find_command(run_id, command_id)
        }

        fn commit(&self, _request: CommitRequest) -> Result<CommitOutcome, OriginError> {
            Err(OriginError::Unavailable)
        }
    }

    #[test]
    fn origin_failure_writes_no_committed_cache_entry() {
        let (origin, cache) = setup();
        let failing = FailingCommitOrigin(origin);
        let result = CommandTransaction::new(&failing, &cache).execute(&submit(
            1,
            0,
            1,
            1,
            Side::Buy,
            100,
            1,
        ));
        assert_eq!(
            result,
            Err(TransactionError::Origin(OriginError::Unavailable))
        );
        assert_eq!(cache.is_empty(), Ok(true));
    }

    #[test]
    fn insufficient_cash_and_inventory_commit_no_accounting_mutation() {
        let (seed_origin, _) = setup();
        let seed = seed_origin.load_run(RunId::new(1));
        assert!(seed.is_ok());
        if let Ok(mut state) = seed {
            state
                .accounts
                .iter_mut()
                .for_each(|(participant, account)| {
                    if *participant == ParticipantId::new(1) {
                        account.cash = MoneyMinor(5);
                    }
                });
            state
                .holdings
                .iter_mut()
                .for_each(|(participant, _, holding)| {
                    if *participant == ParticipantId::new(2) {
                        holding.position = QuantityLots(0);
                    }
                });
            let origin = InMemoryOrigin::new();
            assert!(origin.insert_run(state).is_ok());
            let cache = InMemorySnapshotCache::new();
            let transaction = CommandTransaction::new(&origin, &cache);
            let cash_reject = transaction.execute(&submit(1, 0, 1, 1, Side::Buy, 100, 1));
            assert!(matches!(
                cash_reject,
                Ok(CommandResult {
                    accepted: false,
                    ..
                })
            ));
            let next = cash_reject
                .map(|result| result.committed_sequence.get())
                .unwrap_or(0);
            let inventory_reject = transaction.execute(&submit(2, next, 2, 2, Side::Sell, 100, 1));
            assert!(matches!(
                inventory_reject,
                Ok(CommandResult {
                    accepted: false,
                    ..
                })
            ));
            if let Ok(recovered) = origin.load_run(RunId::new(1)) {
                let ledger = Ledger::from_projection(recovered.accounts, recovered.holdings);
                assert_eq!(
                    ledger.account(ParticipantId::new(1)).reserved_cash,
                    MoneyMinor(0)
                );
                assert_eq!(
                    ledger
                        .holding(ParticipantId::new(2), InstrumentId::new(1))
                        .reserved_inventory,
                    QuantityLots(0)
                );
            }
        }
    }
}
