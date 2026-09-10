# Bunting Authoritative Market History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bounded, deterministic, server-authoritative trade, time-and-sales, and OHLC projections so the GPUI terminal never fabricates market history from L1 quote snapshots.

**Architecture:** Extend the origin boundary with bounded committed-event reads, project `TradeExecuted` events into a versioned history model in `bunting-application`, expose that projection through the existing FIX competition-report channel, and reduce it into the native client. Bars are derived from committed trades using logical time, a versioned half-open interval policy, and explicit retention limits.

**Tech Stack:** Rust 1.88, origin-store, market-events, bunting-application, simfix-mapping, native FIX server/client, serde JSON reports.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- Trade history source is committed `EventPayload::TradeExecuted`; L1 quotes are not OHLC input.
- Bar policy version is 1.
- Bar buckets are anchored at logical time 0 and use half-open ranges `[start_ns, end_ns)`.
- Empty buckets are omitted in v1; the API never fabricates zero-volume bars.
- `open` is first committed trade in event-sequence order, `close` is last, `high`/`low` are extrema, and `volume_lots` is checked sum of trade quantities.
- Every response includes represented/committed event sequence so clients can mark stale data.
- Reads are bounded; default event-tail limit is 4096 and hard maximum is 16384 events per request.

---

### Task 1: Add bounded committed-event reads to the origin contract

**Files:**
- Modify: `packages/origin-store/src/lib.rs`
- Modify: `apps/bunting-server/src/storage.rs`
- Test: `packages/origin-store/src/lib.rs`
- Test: `apps/bunting-server/src/storage.rs`

**Interfaces:**
- Produces:

```rust
pub const MAX_EVENT_READ_LIMIT: usize = 16_384;

pub trait OriginStore {
    fn load_run(&self, run_id: RunId) -> Result<RunState, OriginError>;
    fn find_command(&self, run_id: RunId, command_id: CommandId)
        -> Result<Option<(String, CommandResult)>, OriginError>;
    fn load_events(
        &self,
        run_id: RunId,
        after_sequence: EventSequence,
        limit: usize,
    ) -> Result<Vec<EventEnvelope>, OriginError>;
    fn commit(&self, request: CommitRequest) -> Result<CommitOutcome, OriginError>;
}
```

`load_events` returns strictly increasing event sequences greater than `after_sequence`, capped to `min(limit, MAX_EVENT_READ_LIMIT)`.

- [ ] **Step 1: Write failing in-memory event-tail tests**

Commit two commands that each produce events, then assert:

```rust
let tail = origin.load_events(run_id, EventSequence::new(0), 2).unwrap();
assert_eq!(tail.len(), 2);
assert!(tail.windows(2).all(|pair| pair[0].sequence < pair[1].sequence));

let next = origin.load_events(run_id, tail[0].sequence, 16).unwrap();
assert!(next.iter().all(|event| event.sequence > tail[0].sequence));
```

Add a test that a requested limit above 16384 is capped rather than allocating an unbounded result.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-origin-store load_events
```

Expected: FAIL because the trait method does not exist.

- [ ] **Step 3: Implement `InMemoryOrigin::load_events`**

Read the existing per-run event vector, filter `event.sequence > after_sequence`, take the bounded limit, and clone the selected envelopes. Unknown runs return `OriginError::UnknownRun` rather than an empty history.

- [ ] **Step 4: Implement native file/memory origin delegation**

In `apps/bunting-server/src/storage.rs`, implement/delegate the same trait method for `NativeOrigin`. The file-backed representation already persists event vectors with state; read under the same consistency lock used for `load_run` so history and run sequence come from one valid persisted snapshot.

- [ ] **Step 5: Run origin/server storage tests**

```bash
cargo test -p bunting-origin-store
cargo test -p bunting-server storage
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add packages/origin-store/src/lib.rs apps/bunting-server/src/storage.rs
git commit -m "feat: add bounded origin event reads"
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
    pub represented_sequence: EventSequence,
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

Construct three `TradeExecuted` event envelopes for one instrument at logical times `1`, `40`, and `100` with prices `10`, `12`, `11`, quantities `2`, `3`, `4`, and an interval of `100` ns. Assert the first two form one bar:

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

Assert the trade at exactly `100` starts the next half-open bucket. Add another instrument event and prove it is filtered out.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-application project_market_history
```

Expected: FAIL because the projection types/function are absent.

- [ ] **Step 3: Implement trade extraction**

Iterate events in supplied committed order and select only:

```rust
EventPayload::TradeExecuted { instrument_id: id, price, quantity, .. }
    if *id == instrument_id
```

Reject a zero `bar_interval_ns`. Verify event sequences are strictly increasing; return a deterministic application error on malformed input rather than sorting a corrupt stream silently.

- [ ] **Step 4: Implement bar aggregation**

For each trade compute:

```rust
let bucket = trade.logical_time.get() / bar_interval_ns;
let start = bucket.checked_mul(bar_interval_ns).ok_or(ApplicationError::ArithmeticOverflow)?;
let end = start.checked_add(bar_interval_ns).ok_or(ApplicationError::ArithmeticOverflow)?;
```

Update only the current bucket because event/logical-time regression is invalid. Sum volume with checked arithmetic, increment `trade_count` with checked arithmetic, and record the last event sequence.

- [ ] **Step 5: Set represented sequence from authoritative state**

`represented_sequence` is `state.event_sequence()`. If the supplied event slice ends before that sequence because the bounded read is partial, the caller must page before claiming full requested history; this projection function may represent a requested bounded tail but still exposes the state sequence for staleness detection.

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

### Task 3: Add a FIX competition request/report for market history

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
    after_sequence: u64,
    limit: usize,
    bar_interval_ns: u64,
}
```

The wire action name is `market_history`. The server returns a public competition report whose projection name is `market_history` and whose JSON body is `MarketHistoryProjection`.

- [ ] **Step 1: Add a failing mapping round-trip test**

Build the exact request through the existing competition action message constructor and assert parser output equals:

```rust
CompetitionRequest::MarketHistory {
    instrument_id: 1,
    after_sequence: 0,
    limit: 4096,
    bar_interval_ns: 60_000_000_000,
}
```

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p simfix-mapping market_history
```

Expected: FAIL because the action is unknown.

- [ ] **Step 3: Implement request decoding with hard bounds**

Reject `limit == 0`, clamp/reject above `MAX_EVENT_READ_LIMIT` consistently with the API contract, reject `bar_interval_ns == 0`, and parse instrument/sequence as unsigned integer strings using existing mapping conventions.

- [ ] **Step 4: Implement server request handling**

In `competition_messages`, recover the requested bounded event tail through `OriginStore::load_events`, page only until `limit` events are collected or no more events remain, then call `project_market_history(&state, &events, InstrumentId::new(instrument_id), bar_interval_ns)`.

Return:

```rust
competition_report(
    "U7",
    "public",
    "market_history",
    "snapshot",
    "ok",
    state.sequence().get(),
    &projection,
)
```

If `U7` is already assigned in the current custom profile, choose the next unassigned user-defined message type and update the generated protocol registry atomically; never collide with an existing mapping.

- [ ] **Step 5: Generate protocol documentation and run tests**

```bash
cargo test -p simfix-mapping market_history
cargo test -p bunting-server market_history
python3 tools/generate_protocol.py
git diff --check
```

Expected: PASS and generated protocol docs contain the market-history action/report.

- [ ] **Step 6: Commit**

```bash
git add packages/simfix-mapping/src/lib.rs apps/bunting-server/src/session_host.rs tools/generate_protocol.py PROTOCOL.md
git commit -m "feat: expose market history over FIX"
```

---

### Task 4: Reduce authoritative history in the native client

**Files:**
- Modify: `apps/bunting-tui/src/protocol.rs`
- Modify: `apps/bunting-tui/src/io_task.rs`
- Modify: `apps/bunting-tui/src/lib.rs`
- Test: `apps/bunting-tui/src/protocol.rs`

**Interfaces:**
- Produces client projection fields:

```rust
pub market_history: Option<MarketHistoryProjection>,
pub market_history_stale: bool,
```

and constructor:

```rust
pub fn market_history_request(
    request_id: u128,
    instrument_id: u128,
    after_sequence: u64,
    limit: usize,
    bar_interval_ns: u64,
) -> FixMessage;
```

- [ ] **Step 1: Write a failing reducer test**

Feed a valid `market_history` competition report JSON with represented sequence 12 into `FixClient`, assert the parsed bars/trades are preserved exactly and staleness is false when committed sequence is 12.

Add a second case where committed sequence advances to 13 without a new history report and assert `market_history_stale` becomes true.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-tui market_history
```

Expected: FAIL because the reducer has no history projection.

- [ ] **Step 3: Implement constructor and reducer**

Use the existing `competition_action` representation. Deserialize the body directly into `MarketHistoryProjection`; do not create a UI-specific candle struct in the transport reducer.

- [ ] **Step 4: Request history from refresh/headless validation**

Include one market-history request in the terminal refresh path after an established session. Use a default 60-second logical-time bar interval and bounded 4096-event tail for the initial workstation view; the future GPUI interval selector can request another interval explicitly.

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

### Task 5: Delete quote-candle authority from the GPUI terminal model

**Files:**
- Modify: `apps/bunting-terminal/src/terminal.rs`
- Modify: `apps/bunting-terminal/src/terminal/state.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_trading.rs`
- Test: `apps/bunting-terminal/src/terminal/state.rs`

**Interfaces:**
- Removes `quote_candles()`/quote-to-OHLC history from canonical rendering.
- Produces a typed view model derived only from `client.market_history`:

```rust
pub struct ChartHistoryView {
    pub represented_sequence: u64,
    pub stale: bool,
    pub bars: Vec<MarketHistoryBarView>,
}
```

- [ ] **Step 1: Write a failing model test**

Construct a client with L1 book data but no market-history report and assert:

```rust
assert!(terminal.chart_history().bars.is_empty());
```

Then inject one authoritative OHLC bar and assert exactly that bar is returned.

- [ ] **Step 2: Run and verify current behavior fails**

```bash
cd apps/bunting-terminal
cargo test chart_history
```

Expected: FAIL because current `quote_candles()` manufactures candles from quote samples.

- [ ] **Step 3: Replace quote candle state**

Delete bounded quote-candle accumulation from the terminal state. Keep current bid/ask/spread as L1 metrics, but chart history comes only from `market_history`.

- [ ] **Step 4: Make unavailable/stale history explicit in the current pre-GPUI-Kit view**

Until the GPUI Kit chart migration plan executes, render text state `Authoritative trade history unavailable` when history is absent and `History stale at sequence N` when stale. Do not fall back to quote candles.

- [ ] **Step 5: Run terminal tests**

```bash
cd apps/bunting-terminal
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

### Task 6: Add end-to-end trade/history path evidence

**Files:**
- Modify: `apps/bunting-server/tests/tui_tcp_black_box.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/specs/rit-tui-parity-matrix.md`
- Modify: `docs/research/rit-binary-audit/market-feature-ledger.md`

**Interfaces:**
- Proves a real matched trade committed by the native venue appears unchanged in time-and-sales and in the deterministic OHLC projection consumed by the client.

- [ ] **Step 1: Add a black-box history regression**

Start the native server fixture, create crossing participant orders that produce a known `TradeExecuted`, request `market_history`, and assert trade price/quantity/event sequence plus OHLC open/high/low/close/volume match the committed event.

- [ ] **Step 2: Run focused integration test**

```bash
cargo test -p bunting-server --test tui_tcp_black_box market_history
```

Expected: PASS.

- [ ] **Step 3: Add the focused test to canonical CI before GPUI packaging**

Run the black-box history test in the root test suite and ensure the GPUI workflow's server smoke requests history after FIX establishment.

- [ ] **Step 4: Update parity documents conservatively**

Mark tick/time-and-sales/OHLC source capability as implemented only for the exact bounded projection delivered here. Leave unsupported RIT history analytics/drawing/export rows partial or missing.

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
