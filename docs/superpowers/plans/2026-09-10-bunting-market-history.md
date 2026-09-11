# Bunting Authoritative Market History Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add bounded, deterministic, server-authoritative recent OHLC and time-and-sales projections so the GPUI terminal never fabricates market history from L1 quote snapshots.

**Architecture:** Read a bounded tail of committed events from the authoritative origin, derive trade prints and bars from `TradeExecuted` using logical time, and expose two separate bounded resource projections through the existing `BE` competition-resource message shape. Separating `market_history` bars from `time_and_sales` keeps each FIX payload below the profile's existing 16,384-byte payload/message budget. The native client reduces both projections and the GPUI chart consumes only authoritative bars.

**Tech Stack:** Rust 1.88, origin-store, market-events, bunting-application, simfix-mapping, native FIX server/client, serde JSON reports.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- Source data is committed `EventPayload::TradeExecuted`; L1 quote samples never become OHLC or time-and-sales.
- Bar policy version is 1. Buckets are anchored at logical time 0 and use half-open ranges `[start_ns, end_ns)`.
- Empty buckets are omitted.
- `open` is first committed trade in sequence order, `close` is last, `high`/`low` are extrema, and volume is a checked quantity sum.
- Origin event-tail default is 4,096 events; hard maximum is 16,384 events per read.
- Market-history response hard maximum is 48 bars. Time-and-sales response hard maximum is 64 trades.
- Each serialized `10020=BuntingPayloadJSON` response must be at most 16,384 bytes; maximum-shape tests enforce this.
- Existing `BE` resource messages remain the transport envelope. `10016` distinguishes `news`, `market_history`, and `time_and_sales`; no new FIX message type or profile version is invented for this slice.
- This is recent bounded history. Older-history pagination remains unsupported and the RIT parity ledger must say so.

---

### Task 1: Add bounded committed-event tail reads to every origin implementation

**Files:**
- Modify: `packages/origin-store/src/lib.rs`
- Modify: `apps/bunting-server/src/storage.rs`
- Modify: `packages/command-transaction/src/lib.rs`
- Test: `packages/origin-store/src/lib.rs`
- Test: `apps/bunting-server/src/storage.rs`
- Test: `packages/command-transaction/src/lib.rs`

**Interfaces:**

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

`load_event_tail` rejects zero, caps to 16,384, selects newest committed events, and returns them in ascending event-sequence order.

- [ ] **Step 1: Write failing `InMemoryOrigin` tail tests**

After creating at least four committed events:

```rust
let tail = origin.load_event_tail(run_id, 2).unwrap();
let all = origin.events(run_id).unwrap();
assert_eq!(tail, all[all.len() - 2..]);
assert!(tail[0].sequence < tail[1].sequence);
```

Add a zero-limit error test and a limit-normalization unit test proving any requested limit above `MAX_EVENT_READ_LIMIT` becomes exactly 16,384 before slicing.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-origin-store load_event_tail
```

Expected: FAIL because the trait method is absent.

- [ ] **Step 3: Implement `InMemoryOrigin::load_event_tail`**

Under the existing origin mutex, reject zero, require the run, then:

```rust
let bounded = limit.min(MAX_EVENT_READ_LIMIT);
let start = events.len().saturating_sub(bounded);
Ok(events[start..].to_vec())
```

- [ ] **Step 4: Implement file/native origin support**

In `apps/bunting-server/src/storage.rs`, add the same method to `FileOriginStore`, reading the persisted per-run event vector under the same state lock as `load_run`. Delegate through `NativeOrigin` if it wraps memory/file variants.

- [ ] **Step 5: Update `CommitRaceOrigin` test mock**

`packages/command-transaction/src/lib.rs` has:

```rust
struct CommitRaceOrigin {
    committed: InMemoryOrigin,
    stale: RunState,
    commit_attempted: AtomicBool,
}
```

Add exactly:

```rust
fn load_event_tail(
    &self,
    run_id: RunId,
    limit: usize,
) -> Result<Vec<EventEnvelope>, OriginError> {
    self.committed.load_event_tail(run_id, limit)
}
```

- [ ] **Step 6: Run all affected tests**

```bash
cargo test -p bunting-origin-store
cargo test -p bunting-command-transaction
cargo test -p bunting-server storage
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/origin-store/src/lib.rs packages/command-transaction/src/lib.rs apps/bunting-server/src/storage.rs
git commit -m "feat: add bounded origin event tail"
```

---

### Task 2: Define deterministic trade, time-and-sales, and OHLC projections

**Files:**
- Modify: `packages/bunting-application/src/competition.rs`
- Modify: `packages/bunting-application/src/lib.rs`
- Test: `packages/bunting-application/src/competition.rs`

**Interfaces:**

```rust
pub const MARKET_HISTORY_POLICY_VERSION: u16 = 1;
pub const DEFAULT_HISTORY_EVENT_LIMIT: usize = 4_096;
pub const MAX_HISTORY_BARS: usize = 48;
pub const MAX_TIME_AND_SALES_TRADES: usize = 64;

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
    pub bars: Vec<OhlcBar>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TimeAndSalesProjection {
    pub run_id: RunId,
    pub instrument_id: InstrumentId,
    pub committed_sequence: EventSequence,
    pub window_first_sequence: Option<EventSequence>,
    pub window_last_sequence: Option<EventSequence>,
    pub truncated_before_window: bool,
    pub trades: Vec<TradePrint>,
}

pub fn project_market_history(
    state: &RunState,
    events: &[EventEnvelope],
    instrument_id: InstrumentId,
    bar_interval_ns: u64,
    bar_limit: usize,
) -> Result<MarketHistoryProjection, ApplicationError>;

pub fn project_time_and_sales(
    state: &RunState,
    events: &[EventEnvelope],
    instrument_id: InstrumentId,
    trade_limit: usize,
) -> Result<TimeAndSalesProjection, ApplicationError>;
```

- [ ] **Step 1: Write failing bar golden-vector test**

Create three `TradeExecuted` events for one instrument at logical times 1, 40, 100 with prices 10, 12, 11 and quantities 2, 3, 4. For interval 100 assert first bar is `[0,100)` with OHLC `10/12/10/12`, volume 5, count 2; the trade at exactly 100 begins the next bar.

- [ ] **Step 2: Write failing time-and-sales ordering/limit test**

With 70 matching trades and one other-instrument trade, call `project_time_and_sales(..., 64)` and assert exactly the newest 64 matching trades remain in ascending committed sequence order. Call with 65 and require a validation error rather than silent widening beyond `MAX_TIME_AND_SALES_TRADES`.

- [ ] **Step 3: Run red**

```bash
cargo test -p bunting-application project_market_history
cargo test -p bunting-application project_time_and_sales
```

Expected: FAIL because projection types/functions are absent.

- [ ] **Step 4: Implement shared committed-window validation**

Reject zero limits and limits over their respective hard maxima. Require all supplied events to match `state.run_id()`, have strictly increasing sequence, and not exceed `state.event_sequence()`. Compute `window_first_sequence`/`window_last_sequence` from supplied events, and:

```rust
let truncated_before_window = events
    .first()
    .is_some_and(|event| event.sequence.get() > 1);
```

- [ ] **Step 5: Implement trade extraction and bars**

Preserve committed sequence/logical time for matching `TradeExecuted`. Time-and-sales keeps the newest `trade_limit` matching prints. Bar aggregation uses logical-time bucket arithmetic with checked multiply/add, checked volume sum and checked trade count. After building bars, retain only the newest `bar_limit` bars while preserving ascending start time.

- [ ] **Step 6: Add maximum serialized-size tests**

Construct worst-case numeric values and maximum counts, serialize each projection independently, and assert:

```rust
assert!(serde_json::to_vec(&history).unwrap().len() <= 16_384);
assert!(serde_json::to_vec(&time_and_sales).unwrap().len() <= 16_384);
```

If either assertion fails, reduce its hard count constant in this task until the worst-case payload is under 16,384; do not raise the FIX profile payload limit.

- [ ] **Step 7: Run package tests**

```bash
cargo test -p bunting-application
```

Expected: PASS including payload-size tests.

- [ ] **Step 8: Commit**

```bash
git add packages/bunting-application/src/competition.rs packages/bunting-application/src/lib.rs
git commit -m "feat: project authoritative recent market history"
```

---

### Task 3: Add `BE` resource requests for bars and time-and-sales

**Files:**
- Modify: `packages/simfix-mapping/src/lib.rs`
- Modify: `apps/bunting-server/src/session_host.rs`
- Modify: `PROTOCOL.md`
- Modify: `tools/generate_protocol.py`
- Test: `packages/simfix-mapping/src/lib.rs`
- Test: `apps/bunting-server/src/session_host.rs`

**Interfaces:**

```rust
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarketHistoryRequestPayload {
    pub instrument_id: InstrumentId,
    pub event_limit: usize,
    pub bar_interval_ns: u64,
    pub bar_limit: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TimeAndSalesRequestPayload {
    pub instrument_id: InstrumentId,
    pub event_limit: usize,
    pub trade_limit: usize,
}
```

`CompetitionRequest` adds:

```rust
MarketHistory(MarketHistoryRequestPayload),
TimeAndSales(TimeAndSalesRequestPayload),
```

Wire requests use:

```text
35=BE
10016=market_history | time_and_sales
10018=query
10020=<bounded JSON request payload>
```

Responses use the same `BE` envelope with `10018=snapshot` and the resource projection in `10020`.

- [ ] **Step 1: Write failing mapping tests**

Build two `FixMessage::new("BE")` requests. For history, set `10016=market_history`, `10018=query`, and payload JSON for instrument 1/event_limit 4096/bar_interval 60_000_000_000/bar_limit 48. For time-and-sales, use `10016=time_and_sales`, event_limit 4096/trade_limit 64. Assert exact `CompetitionRequest` variants.

- [ ] **Step 2: Run red**

```bash
cargo test -p simfix-mapping market_history
cargo test -p simfix-mapping time_and_sales
```

Expected: FAIL because current `BE` mapping recognizes only `10016=news`.

- [ ] **Step 3: Implement exact `BE` resource mapping**

Preserve the current news branch, then add resource-kind branches for `market_history` and `time_and_sales`. Require `10018=query`, require tag 10020, deserialize with `serde_json`, and validate all limits against constants from `bunting-application`. Any unknown BE resource kind returns `MappingError::UnsupportedMessage`; malformed/oversized JSON returns the existing serialization/payload error class or a new exact `InvalidPayload` variant used by both resources.

- [ ] **Step 4: Implement server resource handling**

For either request, load the current authoritative run state once and `origin.load_event_tail(run_id, event_limit)`. Call the corresponding projection function. Return through:

```rust
competition_report(
    "BE",
    "public",
    "market_history", // or "time_and_sales"
    "snapshot",
    "ok",
    state.sequence().get(),
    &projection,
)
```

`competition_report`'s existing 16,384-byte payload check remains unchanged and acts as a second defense after the maximum-shape tests.

- [ ] **Step 5: Update generated protocol documentation**

Do not add a new message type/tag. Update the generator/source schema description so `BE` is documented as the current Bunting bounded resource envelope for `news`, `market_history`, and `time_and_sales`, with resource-specific JSON schemas in 10020.

- [ ] **Step 6: Run mapping/server/protocol tests**

```bash
cargo test -p simfix-mapping market_history
cargo test -p simfix-mapping time_and_sales
cargo test -p bunting-server market_history
cargo test -p bunting-server time_and_sales
python3 tools/generate_protocol.py
git diff --check
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/simfix-mapping/src/lib.rs apps/bunting-server/src/session_host.rs tools/generate_protocol.py PROTOCOL.md schemas/fix
git commit -m "feat: expose authoritative market history resources"
```

---

### Task 4: Reduce both history resources in the native client

**Files:**
- Modify: `apps/bunting-tui/src/protocol.rs`
- Modify: `apps/bunting-tui/src/io_task.rs`
- Modify: `apps/bunting-tui/src/lib.rs`
- Test: `apps/bunting-tui/src/protocol.rs`

**Interfaces:**

```rust
pub market_history: Option<MarketHistoryProjection>,
pub time_and_sales: Option<TimeAndSalesProjection>,
pub market_history_stale: bool,
pub time_and_sales_stale: bool,

pub fn market_history_request(
    request_id: u128,
    instrument_id: u128,
    event_limit: usize,
    bar_interval_ns: u64,
    bar_limit: usize,
) -> FixMessage;

pub fn time_and_sales_request(
    request_id: u128,
    instrument_id: u128,
    event_limit: usize,
    trade_limit: usize,
) -> FixMessage;
```

- [ ] **Step 1: Write failing reducer tests**

Feed valid `BE` reports with `10016=market_history` and `10016=time_and_sales`, each at committed sequence 12. Assert exact projection preservation and stale flags false at sequence 12. Advance client committed sequence to 13 without fresh resource reports and assert both stale flags true.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-tui market_history
cargo test -p bunting-tui time_and_sales
```

Expected: FAIL because these resource reducers do not exist.

- [ ] **Step 3: Implement request constructors**

Both constructors create `FixMessage::new("BE")`, push `10016` resource kind, `10018=query`, and exact serialized request payload at 10020. They do not create a chart-specific type.

- [ ] **Step 4: Implement reducers and refresh requests**

Deserialize response 10020 directly into the corresponding application projection. After FIX establishment/refresh request current selected instrument with default event limit 4096, 60-second bar interval, 48 bars, and 64 trades.

- [ ] **Step 5: Run client tests**

```bash
cargo test -p bunting-tui
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-tui/src/protocol.rs apps/bunting-tui/src/io_task.rs apps/bunting-tui/src/lib.rs
git commit -m "feat: reduce authoritative market history resources"
```

---

### Task 5: Remove quote-candle authority from the GPUI model

**Files:**
- Modify: `apps/bunting-terminal/src/terminal.rs`
- Modify: `apps/bunting-terminal/src/terminal/state.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_trading.rs`
- Test: `apps/bunting-terminal/src/terminal/state.rs`

**Interfaces:**

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

Give the terminal L1 quote/book samples but no `market_history`; assert `chart_history().bars.is_empty()`. Inject one authoritative `OhlcBar` and assert exactly that bar is returned. Add a separate assertion that time-and-sales rows come only from `client.time_and_sales`.

- [ ] **Step 2: Run red**

```bash
cd apps/bunting-terminal
cargo test chart_history
cargo test time_and_sales
```

Expected: chart test fails under current `quote_candles()` behavior.

- [ ] **Step 3: Delete quote-to-candle state**

Keep current bid/ask/spread as L1 metrics, but no quote sample may enter chart-history or time-and-sales views.

- [ ] **Step 4: Render explicit pre-GPUI-Kit states**

When history is absent, show `Authoritative trade history unavailable`; when stale, include committed sequence; when `truncated_before_window`, label it `Recent history window`. Time-and-sales gets analogous unavailable/stale labels.

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

### Task 6: Prove committed trade-to-history behavior end to end

**Files:**
- Modify: `apps/bunting-server/tests/tui_tcp_black_box.rs`
- Modify: `.github/workflows/ci.yml`
- Modify: `docs/specs/rit-tui-parity-matrix.md`
- Modify: `docs/research/rit-binary-audit/market-feature-ledger.md`

**Interfaces:**
- One real native-venue match must appear unchanged in both time-and-sales and its containing deterministic OHLC bar.

- [ ] **Step 1: Add black-box integration test**

Start the native server fixture, create crossing authenticated participant orders that produce a known committed `TradeExecuted`, request both BE resources, and assert trade price/quantity/event sequence/logical time plus OHLC open/high/low/close/volume match the committed event. Assert both reports carry the expected committed sequence.

- [ ] **Step 2: Run focused test**

```bash
cargo test -p bunting-server --test tui_tcp_black_box market_history
```

Expected: PASS.

- [ ] **Step 3: Add path to CI/local terminal smoke**

Keep the test in canonical root CI and have the GPUI/local-server smoke request both history resources after FIX establishment.

- [ ] **Step 4: Update RIT parity conservatively**

Mark recent committed trade/time-and-sales/OHLC sourcing implemented for this bounded contract only. Leave older-history navigation, proprietary RIT analytics, drawing, and export partial/missing.

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
