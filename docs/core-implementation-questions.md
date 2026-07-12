# Core implementation questions and binding answers

This document is the concise implementation contract for the questions most likely to block a sprint. It supplements `docs/architecture.md` and the ADRs. When a summary elsewhere is less specific, this document and the referenced ADR take precedence.

## Decision index

| Question | Binding answer | Primary owner |
|---|---|---|
| Where does authoritative market state live? | One `MarketRun` Durable Object per run; the pure Rust aggregate owns matching, risk, ledger, logical clock, scenarios and canonical sequence. | `workers/market-run-do`, core crates |
| How is the resting book implemented? | Ordered `BTreeMap` price levels plus a Bunting-owned safe handle-backed FIFO and direct order-location index. No lock-free or hash-order-dependent book in the authoritative object. | `crates/orderbook` |
| How is the order book streamed? | Hibernatable WebSocket accepted by the `MarketRun` object; committed snapshots and absolute L1/L2 deltas are published through bounded, acknowledged stream records. | `workers/market-run-do`, `crates/protocol-native` |
| How do clients recover? | Resume from a bounded activation-local projection ring when possible; otherwise receive a fresh snapshot with a new stream epoch. Canonical events remain the durable recovery source. | `workers/market-run-do` |
| How are slow consumers handled? | Application acknowledgements and bounded outstanding bytes/records; coalesce public book state, never silently drop private executions, and disconnect persistent slow consumers with recovery metadata. | `workers/market-run-do` |
| Where does user Python run? | In isolated Cloudflare Dynamic Workers loaded only by a minimal TypeScript loader. It never runs in the market object. | `services/strategy-loader` |
| Does the market object wait on untrusted Python? | No. Strategy requests are committed and dispatched asynchronously; results return as authenticated, idempotent internal commands. | `workers/strategy-dispatch-consumer`, `workers/market-run-do` |
| Where is strategy state stored? | Explicit versioned state is included in invocation input/output and committed by the market run. No isolate-global state and no Dynamic Worker Durable Object Facet in the initial design. | `workers/market-run-do` |
| Are Dynamic Worker results re-executed during replay? | No. The invocation request, source/runtime versions, accepted output, state transition and generated actions are recorded. Replay applies recorded results. | `market-events`, `replay-format` |
| How are scenarios deterministic? | Single-threaded total order `(logical_time_ns, phase, priority, schedule_sequence)` and independently derived, versioned PRNG streams. | `scenario-engine`, `agent-models` |
| How do FIX, NBC, RITC and Nautilus interact with the kernel? | They translate into canonical commands/events and protocol-neutral reconciliation reports. No adapter owns matching, risk, ledger or authoritative position. | protocol crates and clients |
| What is durable versus derived? | Canonical events, idempotency, schedules, FIX session state and snapshots live in Durable Object SQLite. WebSocket projection rings, caches and analytics are derived. | `workers/market-run-do` |
| When is sharding introduced? | Not initially. One object owns all instruments in one run until measured CPU, storage or fan-out limits justify a new ADR. | architecture review |

## 1. Authoritative command and publication order

A command is processed in this order:

1. authenticate and authorize;
2. validate schema, units, run state and idempotency;
3. execute pre-trade risk and the pure deterministic transition;
4. atomically append the canonical event batch and idempotency result;
5. apply committed events to hot state;
6. acknowledge the private command result;
7. derive and publish public/private projections;
8. enqueue idempotent non-authoritative work.

No order acknowledgement, fill, stream update, strategy callback or analytics job may appear before the canonical event batch is durable.

## 2. Internal order-book implementation

The first correct implementation is:

```text
bids: BTreeMap<PriceTicks, PriceLevel>   # traversed descending
asks: BTreeMap<PriceTicks, PriceLevel>   # traversed ascending
orders: generational arena<OrderNode>
locations: OrderId -> { side, price, handle }
PriceLevel: { head_handle, tail_handle, total_quantity, order_count }
OrderNode: { order, previous_handle, next_handle }
```

Required properties:

- O(log P) price-level lookup, where `P` is active price levels;
- O(1) removal after direct order lookup;
- FIFO preservation after partial fills;
- stale handles reject through generations;
- deterministic sorted traversal independent of arena iteration order;
- snapshots serialize logical values and links, never pointer identity;
- all aggregates use checked fixed-point arithmetic.

`slotmap` is the preferred dependency experiment for the arena. `intrusive-rs` supplies list invariants and cursor ideas, not the initial pointer-backed implementation.

## 3. Streaming order book

### 3.1 Topology

The Edge API authenticates the upgrade and routes it to the authoritative `MarketRun` object. The Durable Object accepts the server socket with the Hibernation WebSocket API.

The object is both sequencer and publisher for the initial release. This avoids an eventually consistent second book. A separate fan-out tier is deferred until measured connection or CPU limits justify it.

### 3.2 Initial channels

```text
run.status
instrument.status:<instrument>
book.l1:<instrument>
book.l2:<instrument>
trades:<instrument>
bars:<instrument>:<interval>
orders.private
executions.private
positions.private
account.private
risk.private
strategy.logs
```

Public feeds never expose private client IDs, hidden quantities, strategy state or credentials. L3/order-by-order public depth is not part of the initial protocol.

### 3.3 Subscription and snapshot contract

A client sends a bounded subscription request:

```json
{
  "op": "subscribe",
  "protocol": "bunting.native.v1",
  "channels": [
    { "name": "book.l2", "instrument_id": "ABC", "depth": 50 },
    { "name": "trades", "instrument_id": "ABC" },
    { "name": "executions.private" }
  ],
  "resume": { "stream_epoch": "optional", "record_id": "optional" }
}
```

The server either resumes from its bounded projection ring or sends `stream.reset` with current snapshots:

```json
{
  "type": "stream.reset",
  "protocol_version": 1,
  "run_id": "run-1",
  "stream_epoch": "activation-epoch",
  "reason": "initial|resume_miss|activation_changed|authorization_changed",
  "as_of_event_sequence": "912",
  "snapshots": []
}
```

Each L2 snapshot contains sorted aggregated bid/ask levels, requested depth, exact integer tick/lot strings and a deterministic checksum over the visible projection.

### 3.4 Delta semantics

Projection records are ordered within a `stream_epoch`:

```json
{
  "type": "stream.batch",
  "protocol_version": 1,
  "run_id": "run-1",
  "stream_epoch": "activation-epoch",
  "record_id": "42",
  "as_of_event_sequence": "918",
  "messages": []
}
```

L2 changes use **absolute level quantity**, not arithmetic increments:

```json
{
  "channel": "book.l2",
  "instrument_id": "ABC",
  "type": "book.delta",
  "changes": [
    { "side": "bid", "price_ticks": "4000", "quantity_lots": "15" },
    { "side": "ask", "price_ticks": "4010", "quantity_lots": "0" }
  ],
  "checksum": "..."
}
```

`quantity_lots = 0` removes a visible level. Absolute quantities make retransmission and public-book coalescing idempotent. L1 messages carry the complete current best bid/ask state. Trades and private executions are append-only records and are not represented as book deltas.

### 3.5 Sequence model

- `EventSequence` is authoritative and durable.
- `stream_epoch` and `record_id` are derived delivery cursors for one Durable Object activation.
- Each record states the highest canonical event sequence reflected in its payload.
- A new activation creates a new epoch because the in-memory projection ring may be gone.
- After hibernation or activation change, connected clients receive `stream.reset` before new deltas.
- Canonical event recovery remains available through bounded HTTP/event archive endpoints; the WebSocket ring is an optimization.

### 3.6 Acknowledgements and backpressure

Clients periodically acknowledge the highest fully applied record:

```json
{
  "op": "ack",
  "stream_epoch": "activation-epoch",
  "record_id": "42"
}
```

The server tracks outstanding record count and bytes per socket. Because the Worker WebSocket API does not provide an authoritative application-level delivery backlog, ACK progress is the backpressure signal.

Policy:

- public L1 and L2 may coalesce to the latest absolute state;
- bars and status may coalesce by key;
- trades may be batched but not silently removed;
- private order, execution, account and risk records are never silently dropped;
- if a non-coalescible backlog exceeds limits, send `slow_consumer`, close the socket, and include the last recoverable cursor/current event sequence;
- frame, message, depth, subscription and outstanding-byte limits are versioned configuration.

### 3.7 Batching and hibernation

The object batches logical messages by committed command/event batch and bounded frame size/count. It does **not** use a periodic `setInterval`/`setTimeout` flush loop because scheduled callbacks prevent WebSocket hibernation.

Per-connection hibernation attachment contains only small data:

- authenticated participant/role reference;
- authorized subscriptions;
- last acknowledged epoch/record;
- protocol version;
- optional FIX session identifier.

Canonical state and large queues never live in attachments.

## 4. Dynamic Worker strategy execution

### 4.1 Runtime topology

```text
MarketRun commit StrategyInvocationRequested
        |
        v
Cloudflare Queue (at-least-once)
        |
        v
strategy-dispatch-consumer
        |
        v service binding
minimal TypeScript strategy-loader
        |
        v
isolated Python Dynamic Worker
        |
        v
validated StrategyInvocationResult command
        |
        v
MarketRun commits result/state/actions
```

The authoritative Durable Object never waits inside its transaction or matching transition for untrusted Python.

### 4.2 Loader identity and reuse

Use `LOADER.get(id, callback)`, not `load()`, for recurring strategy invocations. The immutable worker ID is a hash of:

```text
user_source_hash
wrapper_version
sdk_version
compatibility_date
compatibility_flags
limits_profile_version
module_manifest_hash
```

The callback must always return identical `WorkerCode` for the same ID. Any change produces a new ID. Reuse is an optimization only: Cloudflare does not guarantee that later requests reach the same isolate.

### 4.3 Python wrapper

The loaded code includes the `python_workers` compatibility flag and a fixed wrapper exposing bounded callbacks:

```text
on_start(context, state)
on_market(context, state, batch)
on_fill(context, state, fill)
on_timer(context, state, timer)
```

The transport can be a default `WorkerEntrypoint.fetch()` accepting and returning bounded JSON/binary payloads. User code cannot replace the outer validation wrapper.

### 4.4 State and concurrency

- No strategy correctness depends on Python globals or isolate reuse.
- State is an explicit versioned input and output.
- The market run is authoritative for the current state revision and state hash.
- Initially, at most one invocation per strategy state revision is in flight.
- New market events while an invocation is in flight are coalesced into the next pending market batch.
- A result with a stale state revision, unknown invocation ID or expired deadline is rejected idempotently.
- Queue duplicate delivery may execute a strategy more than once; only the first valid result for an invocation ID can become canonical.

### 4.5 Sandbox

Mandatory:

- `globalOutbound: null` so `fetch()` and `connect()` fail;
- no direct D1, R2, KV, Queue, Durable Object, secret or credential binding;
- no Durable Object Facet for initial strategy state;
- no reusable token in input;
- source, module, input, state, output, action and log byte/count limits;
- custom `cpuMs` and `subRequests` limits on `WorkerCode` and/or entrypoint invocation;
- schema validation of every returned action;
- all accepted actions pass normal authorization, risk, matching and idempotency.

Cloudflare custom limits currently cover CPU time and subrequests. Bunting must enforce source size, recursion/nesting, input/output, state, action and log limits itself.

### 4.6 Observability

Attach a Tail Worker to capture console output, exceptions and request metadata with the immutable worker ID and invocation ID. Tail logs are operational and may arrive after the result; they are not canonical market truth.

The wrapper may also return bounded user-visible logs. Those logs are validated and streamed on `strategy.logs` separately from private execution state.

### 4.7 Replay and determinism

Dynamic strategy execution is treated like an external participant:

- record invocation ID, trigger, input event range/hash, source/wrapper/SDK/runtime versions, state revision/hash, deadlines, accepted output, next-state hash and generated actions;
- replay does not call the Dynamic Worker;
- replay applies the recorded accepted result and canonical actions;
- duplicate or failed outputs remain audit events but do not mutate strategy state;
- built-in Rust agents remain the option for fully endogenous accelerated simulation.

Initial live Dynamic Worker support is for lockstep and paced modes. Accelerated runs use built-in agents or previously recorded external actions until asynchronous checkpoint behavior has a dedicated load/fairness review.

### 4.8 Failure behavior

Timeout, CPU limit, load error, malformed output or exception produces a typed `StrategyInvocationFailed` result. State remains unchanged and no proposed action is accepted. A versioned policy may pause or disable a strategy after repeated failures without halting the market run.

## 5. Scenario scheduler

The canonical scheduler is single-threaded and ordered by:

```text
(logical_time_ns, phase, priority, schedule_sequence)
```

Agents receive immutable observations and their own versioned PRNG streams. They return intents only. They cannot mutate books, ledger, risk, storage or another agent.

The Dynamic Worker path is not the implementation of built-in scenario agents. Built-in agents are pure Rust and replayed from scenario state and PRNG state.

## 6. External execution and reconciliation

- Exchange matching and participant-side order reconciliation are separate.
- FIX, RITC, Nautilus and future venue adapters normalize reports into `order-reconciliation`.
- Request success never implies acknowledgement or fill.
- Local/client/venue IDs are typed and collision checked.
- Duplicate and out-of-order reports are explicit idempotent transitions or quarantine outcomes.
- Canonical positions and balances come from `ledger`, not adapter caches.
- Kill switch is durable risk state and produces bounded cancellation actions.

## 7. Persistence and recovery

Durable Object SQLite stores:

- canonical events and event-batch metadata;
- idempotency requests/results;
- compatible checksummed snapshots;
- scheduled events and PRNG state through aggregate snapshots;
- FIX session sequence and outbound journal;
- pending strategy invocation metadata and accepted result/state revision.

Derived and discardable:

- WebSocket projection ring;
- public book projection caches;
- Tail Worker logs;
- analytics/metrics;
- R2 exports rebuilt from events.

On activation, load snapshot, replay tail events, verify state hash/invariants, initialize a new stream epoch, then accept commands.

## 8. Scale boundaries

The initial object owns all instruments in one run. Before sharding, measure:

- kernel CPU per command and accelerated batch;
- SQLite transaction/event bytes;
- activation replay time;
- snapshot size/time;
- active sockets and fan-out CPU;
- outstanding stream bytes;
- strategy invocation rate;
- logical-to-wall pacing lag.

If fan-out becomes the first bottleneck, prefer a derived read-only fan-out tier fed by committed events while keeping the run object authoritative. If matching becomes the bottleneck, a new ADR must define cross-instrument ordering and atomicity before instrument sharding.

## 9. Explicitly deferred

- lock-free authoritative order book;
- L3 public market data;
- durable WebSocket projection logs;
- binary native protocol before JSON fixtures and profiling;
- strategy network access or custom capabilities;
- Dynamic Worker Durable Object Facets;
- live Dynamic Workers in unrestricted accelerated mode;
- multi-object matching for one run;
- real-money routing.

## 10. Verification gates

### Streaming

- snapshot plus all following deltas equals a direct current snapshot;
- reconnect ring hit and ring miss both recover correctly;
- activation/hibernation reset sends snapshot before delta;
- absolute L2 changes are idempotent;
- private records are never silently lost;
- slow-consumer close contains usable recovery metadata;
- unauthorized channel subscriptions reject;
- frame/depth/subscription/input limits are enforced.

### Dynamic Worker

- same worker ID callback returns byte-identical code/config;
- no implementation assumes isolate reuse;
- global `fetch()` and `connect()` fail;
- no secret/storage binding is visible;
- CPU/subrequest and Bunting-owned byte/count limits terminate abuse;
- duplicate queue delivery is idempotent;
- stale state revisions reject;
- failure leaves state and market unchanged;
- replay succeeds with the loader disabled;
- accepted actions traverse ordinary risk and matching;
- Tail logs are correlated and non-authoritative.

## References

- `docs/adr/0011-streaming-market-data.md`
- `docs/adr/0012-asynchronous-strategy-dispatch.md`
- `docs/adr/0007-dynamic-worker-loader-boundary.md`
- `docs/architecture.md`
- Cloudflare Dynamic Workers getting started, API, bindings, egress, limits, observability and pricing documentation
- Cloudflare Durable Objects WebSocket Hibernation documentation
