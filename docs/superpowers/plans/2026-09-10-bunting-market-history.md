# Bunting Authoritative Market History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bounded, deterministic, server-authoritative recent trade, time-and-sales, and OHLC projections so the GPUI terminal never fabricates market history from L1 quote snapshots.

**Architecture:** Extend the origin boundary with a bounded committed-event tail read, project `TradeExecuted` events into a versioned history model in `bunting-application`, expose that projection through the existing FIX competition-report channel, and reduce it into the native client. Bars are derived from committed trades using logical time, a versioned half-open interval policy, and explicit event-window metadata. This first slice is a bounded recent-history window; older-history pagination remains outside this implementation and the RIT ledger stays partial for deep historical analytics.

**Tech Stack:** Rust 1.88, origin-store, market-events, bunting-application, simfix-mapping, native FIX server/client, serde JSON reports.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- History source is committed `EventPayload::TradeExecuted`; L1 quotes are not OHLC input.
- Bar policy version is 1.
- Bar buckets are anchored at logical time 0 and use half-open ranges `[start_ns, end_ns)`.
- Empty buckets are omitted in v1; the API never fabricates zero-volume bars.
- `open` is first committed trade in event-sequence order, `close` is last, `high`/`low` are extrema, and `volume_lots` is checked sum of trade quantities.
- Every response includes the committed sequence and exact event-window sequence bounds used to build the projection.
- Reads are bounded; default event-tail limit is 4096 and hard maximum is 16384 events per request.
- Event tails are returned in ascending committed event-sequence order even though selection is from the most recent end of the run.

---

### Task 1: Add a bounded committed-event tail to every `OriginStore` implementation

**Files:**
- Modify: `packages/origin-store/src/lib.rs`
- Modify: `apps/bunting-server/src/storage.rs`
- Modify: `packages/command-transaction/src/lib.rs`
- Test: `packages/origin-store/src/lib.rs`
- Test: `apps/bunting-server/src/storage.rs`
- Test: `packages/command-transaction/src/lib.rs`

**Interfaces:**
- Produces:

```rust
pub const MAX_EVENT_READ_LIMIT: usize = 16_384;

pub trait OriginStore {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError>;
    fn find_command(
        &self,
        run_id: RunId,
        command_id: CommandId,
    ) -> Result<Option<(String, CommandResult)>, OriginError>;
    fn load_event_tail(
        &self,
        run_id: RunId,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, OriginError>;
    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError>;
}
```

`load_event_tail` requires `limit > 0`, caps selection at `MAX_EVENT_READ_LIMIT`, selects the newest events, and returns them in ascending sequence order. Unknown run returns `OriginError::UnknownRun`.

- [ ] **Step 1: Write failing in-memory tail tests**

Commit enough commands to create at least four events, then assert:

```rust
let tail = origin.load_event_tail(run_id, 2).unwrap();
assert_eq!(tail.len(), 2);
assert!(tail[0].sequence < tail[1].sequence);
let all = origin.events(run_id).unwrap();
assert_eq!(tail, all[all.len() - 2..]);
```

Add a zero-limit test returning `OriginError::InvalidCommit` or introduce a more specific `OriginError::InvalidRead` and use that consistently. Add a hard-cap test against a populated synthetic event vector or a small helper that proves requested limits above 16384 are normalized to 16384 before slicing.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-origin-store load_event_tail
```

Expected: FAIL because the trait method does not exist.

- [ ] **Step 3: Implement `InMemoryOrigin::load_event_tail`**

Under the existing mutex, require that the run exists, obtain its event vector, compute:

```rust
let bounded = limit.min(MAX_EVENT_READ_LIMIT);
let start = events.len().saturating_sub(bounded);
Ok(events[start..].to_vec())
```

Reject zero before the slice.

- [ ] **Step 4: Implement `FileOriginStore::load_event_tail`**

In `apps/bunting-server/src/storage.rs`, read the already persisted per-run event vector while holding the same state lock used by `load_run`; apply identical newest-tail/ascending-order semantics and hard bound.

If `NativeOrigin` is an enum/wrapper rather than another trait implementation, delegate to its memory/file variant in the same file.

- [ ] **Step 5: Update the command-transaction test mock**

`packages/command-transaction/src/lib.rs` contains `CommitRaceOrigin: OriginStore`. Add:

```rust
fn load_event_tail(
    &self,
    run_id: RunId,
    limit: usize,
) -> Result<Vec<EventEnvelope>, OriginError> {
    self.committed.load_event_tail(run_id, limit)
}
```

Use the mock's actual backing field name if it differs at implementation time; the forwarding target is the `InMemoryOrigin` already used by that mock. Do not return an empty vector just to satisfy the trait.

- [ ] **Step 6: Run all trait-implementor tests**

```bash
cargo test -p bunting-origin-store
cargo test -p bunting-command-transaction
cargo test -p bunting-server storage
```

Expected: PASS and no `OriginStore` implementation is missing the new method.

- [ ] **Step 7: Commit**

```bash
git add packages/origin-store/src/lib.rs packages/command-transaction/src/lib.rs apps/bunting-server/src/storage.rs
git commit -m "feat: add bounded origin event tail"
```

---

### Task 2: Define deterministic trade and OHLC projection types

**Files:**
- Modify: `packages/bunting-application/src/competition.rs`
- Modify: `packages/bunting-application/src/lib.rs`
- Test: `packages/bunting-application/src/competition.rs`

**Interfaces:**
- Produces:

```rust
pub const MARKET_HISTORY_POLICY_VERSION: u16 = 1;
pub const DEFAULT_HISTORY_EVENT_LIMIT: usize = 4_096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TradePrint {
    pub event_sequence: EventSequence,
    pub logical_time: LogicalTimeNs,
    pub instrument_id: InstrumentId,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OhlcBar {
    pub start_ns: LogicalTimeNs,
    pub end_ns: LogicalTimeNs,
    pub open: PriceTicks,
    pub high: PriceTicks,
    pub low: PriceTicks,
    pub close: PriceTicks,
    pub volume_lots: QuantityLots,
    pub trade_count: u32,
    pub last_event_sequence: EventSequence,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketHistoryProjection {
    pub policy_version: u16,
    pub run_id: RunId,
    pub instrument_id: InstrumentId,
    pub committed_sequence: EventSequence,
    pub window_first_sequence: Option<EventSequence>,
    pub window_last_sequence: Option<EventSequence>,
    pub truncated_before_window: bool,
    pub trades: Vec<TradePrint>,
    pub bars: Vec<OhlcBar>,
}

pub fn project_market_history(
    state: &RunState,
    events: &[EventEnvelope],
    instrument_id: InstrumentId,
    bar_interval_ns: u64,
) -> Result<MarketHistoryProjection, ApplicationError>;
```

- [ ] **Step 1: Write failing golden-vector tests**

Construct three committed `TradeExecuted` envelopes for one instrument at logical times 1, 40, and 100 with prices 10, 12, 11, quantities 2, 3, 4, and interval 100 ns. Assert the first two create:

```rust
assert_eq!(bars[0].start_ns, LogicalTimeNs::new(0));
assert_eq!(bars[0].end_ns, LogicalTimeNs::new(100));
assert_eq!(bars[0].open, PriceTicks::new(10));
assert_eq!(bars[0].high, PriceTicks::new(12));
assert_eq!(bars[0].low, PriceTicks::new(10));
assert_eq!(bars[0].close, PriceTicks::new(12));
assert_eq!(bars[0].volume_lots, QuantityLots::new(5));
assert_eq!(bars[0].trade_count, 2);
```

Assert the trade at exactly 100 starts the next bucket. Include another instrument event and prove it is excluded from trades/bars but still contributes to event-window bounds.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-application project_market_history
```

Expected: FAIL because history types/function do not exist.

- [ ] **Step 3: Validate the supplied committed window**

Reject zero `bar_interval_ns`. Require every envelope's run ID to equal `state.run_id()`, sequence to be strictly increasing, and last sequence not exceed `state.event_sequence()`. Set window bounds from the first/last supplied envelope regardless of whether that envelope is a trade.

Set:

```rust
truncated_before_window = events
    .first()
    .is_some_and(|event| event.sequence.get() > 1);
```

This explicitly tells the client that the bounded tail is not the full run transcript.

- [ ] **Step 4: Extract authoritative trades**

Select only matching-instrument `EventPayload::TradeExecuted` and preserve the envelope's committed event sequence/logical time unchanged.

- [ ] **Step 5: Aggregate bars**

Compute each bucket from `logical_time / bar_interval_ns`, use checked multiply/add for boundaries, checked quantity sum, and checked `trade_count`. Event order determines open/close. Logical-time regression across selected trades is an error; do not sort corrupt input silently.

- [ ] **Step 6: Run application tests**

```bash
cargo test -p bunting-application project_market_history
cargo test -p bunting-application
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/bunting-application/src/competition.rs packages/bunting-application/src/lib.rs
git commit -m "feat: project authoritative trade history"
```

---

### Task 3: Add a bounded FIX market-history request/report

**Files:**
- Modify: `packages/simfix-mapping/src/lib.rs`
- Modify: `apps/bunting-server/src/session_host.rs`
- Modify: `PROTOCOL.md`
- Modify: `tools/generate_protocol.py`
- Test: `packages/simfix-mapping/src/lib.rs`
- Test: `apps/bunting-server/src/session_host.rs`

**Interfaces:**
- Extends `CompetitionRequest` with:

```rust
MarketHistory {
    instrument_id: u128,
    event_limit: usize,
    bar_interval_ns: u64,
}
```

Wire action name: `market_history`. The server returns a public competition report with projection name `market_history` and JSON body `MarketHistoryProjection`.

- [ ] **Step 1: Write failing mapping round-trip test**

Build the request through the existing competition action constructor and assert:

```rust
CompetitionRequest::MarketHistory {
    instrument_id: 1,
    event_limit: 4096,
    bar_interval_ns: 60_000_000_000,
}
```

- [ ] **Step 2: Run red**

```bash
cargo test -p simfix-mapping market_history
```

Expected: FAIL because action is unknown.

- [ ] **Step 3: Implement strict request bounds**

Reject `event_limit == 0`, reject values above `MAX_EVENT_READ_LIMIT`, reject zero bar interval, and parse numeric fields using existing unsigned-integer competition mapping conventions. Do not silently accept an unbounded request.

- [ ] **Step 4: Implement server projection**

For `MarketHistory`, load current authoritative run state, call `origin.load_event_tail(run_id, event_limit)`, and call `project_market_history(&state, &events, InstrumentId::new(instrument_id), bar_interval_ns)`.

Return the report through the existing `competition_report` helper and choose the next currently unused custom message type from the repository's FIX profile. Determine it by inspecting the registry in `simfix-mapping` before editing; add an assertion that the chosen message type is unique in the registry. Do not hard-code `U7` if it is already assigned.

- [ ] **Step 5: Generate protocol docs and run tests**

```bash
cargo test -p simfix-mapping market_history
cargo test -p bunting-server market_history
python3 tools/generate_protocol.py
git diff --check
```

Expected: PASS; protocol docs contain the exact request fields and market-history report.

- [ ] **Step 6: Commit**

```bash
git add packages/simfix-mapping/src/lib.rs apps/bunting-server/src/session_host.rs tools/generate_protocol.py PROTOCOL.md
git commit -m "feat: expose bounded market history over FIX"
```

---

### Task 4: Reduce authoritative history in the native FIX client

**Files:**
- Modify: `apps/bunting-tui/src/protocol.rs`
- Modify: `apps/bunting-tui/src/io_task.rs`
- Modify: `apps/bunting-tui/src/lib.rs`
- Test: `apps/bunting-tui/src/protocol.rs`

**Interfaces:**
- Produces:

```rust
pub market_history: Option<MarketHistoryProjection>,
pub market_history_stale: bool,

pub fn market_history_request(
    request_id: u128,
    instrument_id: u128,
    event_limit: usize,
    bar_interval_ns: u64,
) -> FixMessage;
```

These move unchanged into `packages/bunting-client` during the client-extraction plan.

- [ ] **Step 1: Write failing reducer tests**

Feed a valid report whose `committed_sequence` is 12 into `FixClient`, assert trade/bar/window metadata are preserved and `market_history_stale == false` while the client's committed sequence is 12. Advance committed sequence to 13 without a new history report and assert stale becomes true.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-tui market_history
```

Expected: FAIL because reducer/constructor do not exist.

- [ ] **Step 3: Implement constructor and reducer**

Use existing competition-action encoding. Deserialize report JSON directly into `MarketHistoryProjection`; do not create candle data in the transport layer.

- [ ] **Step 4: Request recent history during refresh**

After FIX establishment, request instrument 1/current selected instrument with `event_limit = DEFAULT_HISTORY_EVENT_LIMIT` and `bar_interval_ns = 60_000_000_000` for the initial view. Keep interval/limit arguments explicit so GPUI can request another bounded window later.

- [ ] **Step 5: Run TUI/client tests**

```bash
cargo test -p bunting-tui
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-tui/src/protocol.rs apps/bunting-tui/src/io_task.rs apps/bunting-tui/src/lib.rs
git commit -m "feat: reduce authoritative market history"
```

---

### Task 5: Remove quote-candle history from the GPUI model

**Files:**
- Modify: `apps/bunting-terminal/src/terminal.rs`
- Modify: `apps/bunting-terminal/src/terminal/state.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_trading.rs`
- Test: `apps/bunting-terminal/src/terminal/state.rs`

**Interfaces:**
- Removes `quote_candles()`/quote-to-OHLC history.
- Produces a typed chart view from `client.market_history` only:

```rust
pub struct ChartHistoryView {
    pub committed_sequence: u64,
    pub window_first_sequence: Option<u64>,
    pub window_last_sequence: Option<u64>,
    pub stale: bool,
    pub truncated_before_window: bool,
    pub bars: Vec<MarketHistoryBarView>,
}
```

- [ ] **Step 1: Write failing model test**

Give the terminal L1 book/quote samples but no market-history report and assert `chart_history().bars.is_empty()`. Then inject one authoritative bar and assert exactly that bar is returned.

- [ ] **Step 2: Run red**

```bash
cd apps/bunting-terminal
cargo test chart_history
```

Expected: FAIL because current quote samples manufacture candles.

- [ ] **Step 3: Delete quote-candle accumulation**

Keep current bid/ask/spread L1 metrics, but no quote sample can enter chart-history bars.

- [ ] **Step 4: Render explicit pre-GPUI-Kit states**

Until the dedicated GPUI Kit chart plan executes, render `Authoritative trade history unavailable` when missing and `History stale at committed sequence N` when stale. If `truncated_before_window`, label the data `Recent history window`, not full-run history.

- [ ] **Step 5: Run terminal tests**

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src/terminal.rs apps/bunting-terminal/src/terminal/state.rs apps/bunting-terminal/src/terminal/views_trading.rs
git commit -m "fix: remove synthetic quote market history"
```

---

### Task 6: Add end-to-end venue trade-to-history evidence

**Files:**
- Modify: `apps/bunting-server/tests/tui_tcp_black_box.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/specs/rit-tui-parity-matrix.md`
- Modify: `docs/research/rit-binary-audit/market-feature-ledger.md`

**Interfaces:**
- Proves one committed match appears unchanged in time-and-sales and in the deterministic OHLC projection consumed by the native client.

- [ ] **Step 1: Add black-box history regression**

Start native server fixture, create crossing authenticated orders that produce a known `TradeExecuted`, request `market_history`, and assert price, quantity, event sequence, logical time, and OHLC values match the committed trade. Also assert report `committed_sequence` equals the server/client committed sequence observed for that response.

- [ ] **Step 2: Run focused integration test**

```bash
cargo test -p bunting-server --test tui_tcp_black_box market_history
```

Expected: PASS.

- [ ] **Step 3: Add focused path to canonical CI**

Keep the integration test in the root suite and have the GPUI/local-server smoke request history after FIX establishment.

- [ ] **Step 4: Update RIT parity conservatively**

Mark recent committed trade/time-and-sales/OHLC sourcing as implemented for the bounded tail contract only. Leave deep historical navigation, proprietary RIT analytics, drawing, and export partial/missing.

- [ ] **Step 5: Run root validation**

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-server/tests/tui_tcp_black_box.rs .github/workflows/ci.yml docs/specs/rit-tui-parity-matrix.md docs/research/rit-binary-audit/market-feature-ledger.md
git commit -m "test: prove authoritative market history end to end"
```
