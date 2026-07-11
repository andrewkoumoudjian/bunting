# Implementation pathway derived from reference research

This document converts the architecture and reference audit into an ordered implementation plan. It deliberately changes several initial instincts where the donor repositories show a safer or simpler path.

## Executive decisions

1. **Do not translate any reference repository wholesale.** Port observable behavior and tests into Bunting-owned types.
2. **Do not import concurrent order-book structures into the Durable Object kernel.** One authoritative sequencer makes lock-free maps, atomics and cross-thread callbacks unnecessary and potentially harmful to determinism and Wasm size.
3. **Use an event-sourced aggregate boundary, not an event-emitting collection of services.** A command is decided against one state version, produces an ordered event batch, and that batch is atomically appended before acknowledgement.
4. **Treat snapshots as caches.** Events remain authoritative; snapshots carry schema version, last sequence and state hash and can always be discarded and rebuilt.
5. **Separate simulation scheduling from matching.** Agents schedule intents; only the kernel validates, risks, matches and books them.
6. **Keep market-making analytics outside correctness-critical accounting.** Strategy math may propose exact tick/lot orders, but it never mutates balances, positions or books directly.
7. **Use independent implementations as differential oracles.** Liquibook, exchange-core, OrderBook-rs and QuickFIX/J should generate fixtures and edge cases; they should not dictate Bunting's runtime architecture.

## Target command pipeline

The pure kernel should expose a small aggregate-style API:

```rust
pub trait Decide<C, E, Err> {
    fn decide(&self, command: C) -> Result<Vec<E>, Err>;
}

pub trait Apply<E> {
    fn apply(&mut self, event: &E) -> Result<(), ApplyError>;
}
```

The concrete market aggregate may optimize this internally, but tests must preserve the semantic split:

1. validate command shape and referenced IDs;
2. enforce idempotency and expected stream version at the host boundary;
3. run deterministic pre-trade risk;
4. determine matching results without I/O;
5. derive canonical order, trade, ledger and risk events in one total order;
6. verify the event batch can be applied to a clone or checked transition state;
7. append the complete batch and idempotency record in one Durable Object SQLite transaction;
8. apply to hot state and acknowledge only after commit;
9. publish non-authoritative streams asynchronously.

No observer callback may issue a recursive command during `decide` or `apply`.

## Core data-structure decision

### Recommended first implementation

Use simple deterministic structures before specialized arenas:

- `BTreeMap<PriceTicks, PriceLevel>` for each side;
- explicit reverse ordering for bids rather than negated prices;
- each `PriceLevel` maintains FIFO order by a monotonic insertion sequence;
- direct `OrderId -> OrderLocation` lookup;
- exact cached level totals checked after every mutation in debug/tests;
- bounded depth extraction;
- a Bunting-owned arena or a small audited generational arena only when cancellation complexity requires it.

A plain `VecDeque<OrderId>` is acceptable only if stale entries are never accumulated and arbitrary cancellation remains bounded. The preferred production shape is an arena-backed intrusive FIFO represented by safe integer handles: each resting order stores previous/next handles and each level stores head/tail. This gives O(1) cancellation without pointers or `unsafe`.

### Explicit rejection

Do not directly depend on OrderBook-rs's lock-free `SkipMap`, `DashMap`, atomics or concurrent manager. Its recent replay-stability fixes are highly valuable evidence: unordered concurrent indexes can make mass-cancel and snapshot restore payloads differ across processes. Bunting should adopt the deterministic traversal lessons while avoiding the concurrency source of that problem.

Do not port exchange-core's Disruptor pipeline, object pools, thread affinity or shard coordination. Its integer accounting, atomic command semantics, state hashes and integrity tests are the reusable parts.

## Canonical matching semantics for the first slice

The first matching contract is intentionally narrow:

- limit and market orders;
- GTC and IOC;
- strict price priority, then insertion-sequence priority;
- resting/maker price is the execution price;
- partial fills preserve the resting order's queue position;
- market remainders never rest;
- IOC remainders produce an explicit expiration/cancellation event;
- duplicate order IDs reject before any state mutation;
- cancel is idempotent only through the command idempotency layer, not by silently accepting an unknown order;
- every fill carries maker order, taker order, exact quantity, exact price and deterministic execution ID;
- all event ordering is specified and tested.

Stop, iceberg, post-only, replace, FOK and self-trade prevention are deferred until the base invariants and replay harness pass. Liquibook and OrderBook-rs supply later edge-case inventories, not a reason to enlarge the first slice.

## Ledger and risk boundary

### Ledger

Project canonical executions into:

- cash by participant and currency;
- signed position by participant and instrument;
- average entry price;
- realized P&L;
- explicit maker/taker fee fields, initially zero.

Use widened checked intermediates. Every execution must conserve base quantity and quote value after fees. Property tests should sum participant deltas and assert conservation.

### Risk

Pre-trade risk owns deterministic admission and reservations:

- active run/instrument/participant;
- lot and tick validity;
- positive quantity;
- maximum order size;
- maximum open orders;
- maximum absolute position;
- price collar;
- participant kill switch.

Risk does not read persistence or wall time. Limits and run status arrive in state-changing events. Exchange-core is the principal behavioral oracle for pre-risk/accounting invariants, while the C++ QUARCC engine supplies kill-switch and order-lifecycle expectations.

## Event and snapshot format

Each event envelope includes:

- schema version;
- run ID;
- event ID;
- stream sequence;
- logical time;
- actor;
- correlation ID;
- causation ID;
- stable event kind and rejection code;
- payload.

Canonical hashing must not depend on Rust map iteration or debug formatting. Define one versioned byte encoding for hashes. If a serialization crate cannot guarantee a stable cross-version encoding, serialize the hash preimage manually from primitive fields in a documented order.

A snapshot contains:

- snapshot schema version;
- scenario version and seed identity;
- last applied event sequence;
- book, risk, ledger, clock and scheduler state;
- PRNG stream states;
- canonical state hash.

Restore must reject a snapshot with an incompatible schema, mismatched run/scenario identity or invalid hash.

## Deterministic simulation scheduler

Borrow concepts from NeXosim and ABIDES, but implement a single-threaded Worker-compatible scheduler.

Every scheduled item has a total-order key:

```text
(logical_time_ns, phase, priority, schedule_sequence)
```

`phase` prevents accidental ordering changes between administrative actions, market events, agent wakeups and end-of-step calculations. `schedule_sequence` is allocated by the authoritative run and breaks all remaining ties.

Agents receive immutable observations and explicit context, then return intents. They cannot access the order book, ledger, persistence, wall clock or global RNG directly.

Use independent deterministic random streams derived from:

```text
master_seed + scenario_version + agent_id + stream_name
```

The derivation algorithm and PRNG version are part of the scenario version. Adding an unrelated agent must not perturb existing agents' sequences. Record external/admin inputs as events.

## Cloudflare Durable Object pathway

The Worker layer should remain thin:

- route a run to one stable Durable Object name;
- parse/authenticate protocol input;
- translate it to a canonical command;
- invoke the pure aggregate;
- transact event append, expected version, idempotency result and optional snapshot metadata;
- update hot projection;
- stream committed events.

SQLite tables should separate:

- `run_events` keyed by run and sequence;
- `command_results` keyed by idempotency key;
- `snapshots` keyed by sequence/schema;
- `scheduled_items` when schedules must survive hibernation;
- `fix_sessions` and outbound resend messages.

Do not put D1, KV, R2, Cache or Queues in the correctness path. Queue consumers receive committed event IDs and are idempotent.

## FIX pathway

Use three project-owned crates:

- `simfix-wire`: framing, BodyLength, CheckSum, field parsing and serialization;
- `simfix-session`: logon/logout, heartbeat, TestRequest, sequence validation, resend, gap fill, duplicate/reset rules and persisted session state;
- `simfix-mapping`: FIX 4.4 business messages to/from canonical commands/events.

IronFix is the first Rust code donor for codec experiments. Fixer and FerrumFIX are design peers. QuickFIX/J is the independent conformance oracle. The Worker endpoint transports one complete raw FIX message per binary WebSocket frame; the native bridge alone owns TCP partial-read handling and Tokio.

Build a table-driven conformance suite covering at minimum:

- BodyLength and CheckSum;
- missing/duplicate/out-of-order required tags;
- logon and reset sequence flags;
- too-low and too-high inbound sequence numbers;
- ResendRequest ranges;
- SequenceReset gap fill versus reset;
- PossDupFlag and OrigSendingTime;
- heartbeat/TestRequest timeout behavior;
- logout and reconnect without silent reset.

## Port pathway by source target

### C++ QUARCC trading engine

Port business semantics in this order:

1. typed IDs and local/external ID mapping;
2. order lifecycle state machine;
3. kill-switch/risk transitions;
4. position and fill projection;
5. journal-derived golden command/event fixtures.

Replace threads, callbacks, gRPC and native SQLite with aggregate events, protocol adapters and Durable Object storage. See `docs/ports/quarcc-trading-engine.md`.

### Java/NBC simulation assets and scenarios

Port data before algorithms:

1. inventory every scenario and parameter;
2. define strict canonical schema and exact units;
3. transcribe scenarios with provenance;
4. implement scheduler and PRNG substreams;
5. implement fundamental/noise agents;
6. add market maker, momentum, institutional, spiking and shock agents one at a time;
7. compare aggregate outputs and qualitative stylized facts rather than assuming binary equivalence to unreviewed Java assets.

See `docs/ports/nbc-simulation.md`.

### Rust RITC market-making implementation

Split the monolith into:

1. `clients/ritc-adapter` or a generic native RIT connector with async HTTP, typed models and explicit clock;
2. pure strategy analytics with configuration and golden vectors;
3. order-intent generation;
4. an order manager that reconciles desired versus live quotes;
5. risk controls outside the strategy formula;
6. optional Bunting-native strategy integration.

Do not place FFT, GARCH, blocking HTTP, sleeps or API credentials in the market kernel. See `docs/ports/ritc-market-making.md`.

## Pull-request sequence and gates

### PR 2 — deterministic kernel

Implement market types, canonical commands/events, order book, matching, initial risk, ledger and logical clock.

Gate:

- unit and property tests;
- generated invariant sequences;
- no I/O/runtime dependencies;
- Wasm check;
- deterministic replay checksum.

### PR 3 — replay, snapshots and differential fixtures

Add canonical hash encoding, snapshots, restore, golden fixture runner and differential cases derived from Liquibook, OrderBook-rs, exchange-core and the C++ reference.

Gate:

- live versus replay state equality;
- snapshot plus tail equality;
- corrupted/incompatible snapshot rejection;
- stable bytes across repeated runs.

### PR 4 — MarketRun Durable Object

Add SQLite schema, atomic append/idempotency transaction, reconstruction and native JSON/WebSocket path.

Gate:

- accepted command is durable before acknowledgement;
- duplicate command returns the original result;
- restart reconstructs identical hashes;
- hibernation/reconnect tests.

### PR 5 — scenario schema and scheduler

Add strict scenario schema, scheduler, versioned PRNG derivation and first fundamental/noise agents.

Gate:

- unrelated-agent random-stream isolation;
- same scenario/seed/commands produce identical events;
- bounded scheduled queue;
- imported scenario provenance.

### PR 6 — legacy NBC compatibility

Add legacy REST/WebSocket translation and lockstep behavior as an adapter, not as kernel semantics.

Gate:

- compatibility fixtures from `ref/nbc-hft-simulation`;
- protocol errors map to stable responses;
- no legacy fields leak into canonical events.

### PR 7 — FIX vertical slice

Add wire/session/mapping crates, WSS endpoint and native TCP bridge.

Gate:

- QuickFIX/J and Fixer interoperability;
- persisted session recovery;
- bounded resend store and input buffers;
- malformed input corpus and fuzzing.

### PR 8 — native strategy clients

Add RITC and Nautilus adapters plus the refactored market-making strategy.

Gate:

- connector/strategy separation;
- deterministic strategy test vectors;
- quote reconciliation and stale-order cleanup;
- hard position/notional/kill-switch enforcement.

### PR 9 — Dynamic Worker strategies

Add isolated Python loader only after the authoritative engine, protocols and replay format are stable.

Gate:

- denied global egress;
- no storage/secret binding;
- action and output limits;
- strategy crash cannot crash or corrupt the run;
- all resulting actions pass canonical Rust validation and risk.

## Definition of done for every sprint

A sprint is not complete until:

- code paths are bounded and panic-free for untrusted input;
- public invariants are documented;
- source provenance and licensing are recorded;
- unit, property, differential and replay tests appropriate to the change pass;
- Worker-bound crates compile for `wasm32-unknown-unknown`;
- no planned behavior is described as implemented;
- new event/schema behavior is versioned and migration impact is stated.
