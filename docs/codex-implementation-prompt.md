# Codex implementation prompt

You are implementing Bunting in `andrewkoumoudjian/bunting`. Work on a dedicated branch and submit reviewable pull requests. Read the nearest `AGENTS.md` before modifying any path.

## Mission

Build a deterministic, event-sourced stock-market simulator primarily in Rust for Cloudflare Workers. Administrators publish versioned scenarios. Multiple users trade through a browser, native APIs, NautilusTrader or FIX 4.4. Market data, orders, fills, positions and P&L stream in real time. Users can submit Python strategies that run in isolated Cloudflare Dynamic Workers.

Do not produce only plans. Implement working vertical slices with tests and documentation. Never advertise a feature as complete when it is a stub.

## Required reading

Before code changes, read:

- `AGENTS.md` and all relevant nested `AGENTS.md` files;
- `docs/architecture.md`;
- `docs/reference-inventory.md`;
- every ADR in `docs/adr/`;
- pinned source repositories in `ref/` relevant to the task.

Verify current Cloudflare APIs against official documentation and the pinned `ref/workers-rs` source. Do not infer APIs from memory.

## Non-negotiable constraints

1. The market kernel is pure, deterministic Rust.
2. Domain crates do not depend on Cloudflare, Tokio, threads, sockets, filesystem I/O, databases or wall-clock APIs.
3. Worker-compatible crates compile for `wasm32-unknown-unknown`.
4. One `MarketRun` Durable Object is authoritative for one run initially.
5. Durable Object SQLite stores canonical run events, snapshots, idempotency, schedules and FIX session state.
6. D1, KV, Cache, Queues, R2 and analytics never decide whether a live order, fill, balance or position exists.
7. Use integer fixed-point values for market correctness.
8. All arithmetic is checked.
9. Matching, risk and ledger are protocol-neutral.
10. Identical scenario version, seed and command stream produce identical state and event hashes.
11. Accepted commands are persisted before acknowledgement.
12. All queues and untrusted collections are bounded.
13. Queue consumers are idempotent.
14. Inbound raw TCP is not available directly in a Worker.
15. FIX 4.4 messages use a binary WebSocket subprotocol in Cloudflare.
16. A native Rust bridge presents ordinary local FIX/TCP.
17. NautilusTrader uses the native HTTP/WebSocket API.
18. User Python never executes inside the authoritative run object.
19. Dynamic Python Workers have denied global egress and no direct secret/storage bindings.
20. Every strategy action is validated by Rust and passes normal risk and matching.
21. Reference code is not assumed correct.
22. Copied/adapted code preserves licenses, exact SHAs and patch records.
23. Avoid `unsafe`; any exception requires a new ADR and targeted review.
24. Avoid `unwrap`, `expect` and panic in request, matching, persistence and protocol paths.

## Reference policy

### Existing references

- `ref/quarcc-trading-engine`: extract order lifecycle, gateway/store/journal interfaces, position behavior and tests. Do not copy its thread/gRPC/native SQLite architecture.
- `ref/nbc_engine`: extract scenario definitions and agent concepts. Treat compiled/binary behavior as unverified.
- `ref/ritc_mm`: extract reviewed RIT models and strategy formulas only. Do not import blocking HTTP, threads, Tokio or floating-point authority into the kernel.

### Pinned submodules

- `ref/workers-rs`: primary Worker runtime and examples.
- `ref/nbc-hft-simulation`: legacy API and client compatibility.
- `ref/ironfix`: candidate FIX core/tag-value source.
- `ref/fixer`: native FIX conformance peer.
- `ref/ferrumfix`: design/conformance reference only.
- `ref/nautilus-trader`: current adapter specification.
- `ref/wirefilter`: optional rule-engine candidate after a Wasm spike.

For every port create or update a porting note containing source file, commit, license, behavior retained, behavior rejected, Bunting abstraction, local changes and tests.

## Workspace target

The intended workspace contains:

```text
crates/
  market-types
  market-events
  orderbook
  matching-engine
  risk-engine
  ledger
  simulation-clock
  scenario-schema
  scenario-engine
  agent-models
  scoring
  protocol-native
  protocol-legacy-nbc
  simfix-wire
  simfix-session
  simfix-mapping
  replay-format
  test-fixtures
workers/
  edge-api
  market-run-do
  export-consumer
  analytics-consumer
services/
  strategy-loader
clients/
  rust-sdk
  fix-bridge
  python-sdk
  nautilus-adapter
web/
  trader-ui
```

Add workspace members incrementally only when their manifests compile. Do not commit dozens of broken placeholder crates.

## Coding standards

- Rust edition 2024 when supported by selected dependencies.
- Safe Rust in project-owned code.
- `thiserror`-style structured errors where an external crate is justified; otherwise small local error enums.
- Strong identifier newtypes.
- No primitive string IDs across the kernel once a domain type exists.
- Checked fixed-point arithmetic.
- Stable event and rejection reason codes.
- Explicit schema and migration versions.
- Deterministic ordered collections where iteration affects output.
- Bounded vector/deque/map sizes for untrusted input.
- Rustdoc for public interfaces and invariants.
- Tests next to pure code; integration fixtures under `tests/`.
- No secret values in source, logs or test snapshots.

## First implementation PR: deterministic kernel vertical slice

Implement this before the full Worker, UI or Dynamic Worker integration.

### 1. Extend `market-types`

Add:

- strongly typed IDs for run, tenant, participant, instrument, order, client order, execution, event, agent, strategy, correlation and FIX session;
- `Side`;
- `OrderType` for market and limit initially;
- `TimeInForce` for GTC and IOC initially;
- checked conversion utilities from validated decimal strings and instrument definitions;
- explicit domain arithmetic errors;
- property and boundary tests.

Keep dependencies minimal. If serialization is added, isolate it behind a feature or adapter and confirm Wasm compilation.

### 2. Extend `market-events`

Define versioned canonical commands and events for:

- submit;
- cancel;
- order accepted/rejected/rested/reduced/canceled;
- trade executed;
- position and balance changed;
- logical clock advanced.

Envelopes include run, event ID, sequence, logical time, actor, correlation and causation. Avoid protocol-specific fields.

### 3. Implement `orderbook`

Create a deterministic price-time order book:

- descending bids and ascending asks;
- FIFO at a level;
- stable order storage using a slab/generational design or another justified bounded structure;
- direct order-ID lookup;
- exact level totals;
- add, reduce and cancel operations;
- best bid/ask and bounded depth snapshots.

Do not add networking, persistence or account logic.

Tests:

- FIFO;
- price priority;
- cancellation;
- partial reduction;
- level cleanup;
- duplicate order ID rejection;
- invariant checker after generated operation sequences.

### 4. Implement `matching-engine`

Support:

- market order;
- limit order;
- IOC;
- partial fills;
- passive rest;
- deterministic execution IDs supplied through explicit state or derivation.

Return canonical events. Do not mutate a database or send messages.

Tests:

- no overfill;
- buyer quantity equals seller quantity;
- execution price policy is explicit;
- multiple-level sweep;
- IOC remainder expires;
- market remainder cannot rest;
- replayed commands produce identical events.

### 5. Implement `ledger`

Project trade events into:

- cash by currency;
- positions by instrument;
- average entry price;
- realized P&L;
- fees initially zero but represented explicitly.

Prove conservation with property tests. Use widened intermediates and checked arithmetic.

### 6. Implement `risk-engine`

Initial deterministic checks:

- run and instrument active;
- positive quantity;
- valid tick/lot;
- max order quantity;
- max open orders;
- max absolute net position;
- price collar;
- participant kill switch.

Return stable reason codes. No I/O.

### 7. Implement logical clock and replay harness

Create a local executable or integration test that:

- constructs one instrument;
- creates two participants;
- submits deterministic limit and market orders;
- produces partial and full fills;
- cancels an order;
- writes an in-memory canonical event stream;
- creates a versioned snapshot;
- restores snapshot plus remaining events;
- computes deterministic book and ledger hashes.

Print:

```text
final event sequence: ...
final book checksum: ...
final ledger checksum: ...
replay book checksum: ...
replay ledger checksum: ...
deterministic replay: PASS
```

### 8. CI for the kernel

Add GitHub Actions for:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --target wasm32-unknown-unknown
```

Add `cargo deny` and audit only after configuring accepted licenses and avoiding noisy unresolved policy. Do not suppress vulnerabilities without a documented issue and expiry.

## Second implementation PR: Worker vertical slice

After the kernel is merged:

1. scaffold `edge-api` and `market-run-do` from the pinned official `workers-rs` template/examples;
2. pin a current compatibility date;
3. add a Durable Object binding and SQLite migration;
4. route one run by stable name;
5. create canonical events and snapshots tables;
6. implement an internal command endpoint;
7. implement native JSON order submission;
8. implement a binary-capable WebSocket pair and sequence-aware JSON stream;
9. persist event batch before acknowledgement;
10. demonstrate reconstruction from SQLite;
11. test locally through Wrangler/Miniflare or the supported current Worker test path.

Do not add D1, KV, R2 or Queues to the matching hot path.

## FIX implementation sequence

Do not depend on the complete IronFix, Fixer or FerrumFIX engine in the Worker.

### Spike

- inventory IronFix files and licenses at the pinned SHA;
- compile `ironfix-core`, dictionary and tag-value candidates for Wasm with minimal features;
- measure Wasm size;
- create BodyLength, CheckSum, field lookup and malformed input tests;
- record findings in `docs/ports/ironfix.md`.

### Shared crates

Implement project-owned:

```rust
trait FixClock { ... }
trait FixTransport { ... }
trait FixMessageStore { ... }
```

`simfix-wire` owns framing and serialization. `simfix-session` owns logon, logout, heartbeat, test request, sequence validation, resend, gap fill, duplicate flags and reset. `simfix-mapping` owns FIX 4.4 business-message conversion.

### Worker endpoint

Use:

```text
GET /v1/runs/{run_id}/fix
Sec-WebSocket-Protocol: bunting.fix44.v1
```

One complete raw FIX message per binary frame. The Durable Object persists session and outbound resend state.

### Native bridge

Build `bunting-fix-bridge` with Tokio only in the native client. It supports acceptor and initiator modes, partial TCP reads, exact byte forwarding, authenticated WSS, bounded buffers, diagnostics and reconnect without silent reset.

Test interoperability with Fixer and another independent engine.

## Scenario implementation sequence

1. design canonical JSON schema with strict unknown-field handling;
2. port the five imported Java scenario definitions into validated examples;
3. replace floating prices/quantities at market boundaries with exact strings/fixed units;
4. implement independent deterministic PRNG streams;
5. implement fundamental and noise agents first;
6. add inventory market maker;
7. add momentum, institutional, spiking and shock agents incrementally;
8. test that unrelated agents do not perturb existing random streams;
9. record all admin actions as events;
10. benchmark any Wirefilter use before adoption.

## Nautilus implementation sequence

Follow `ref/nautilus-trader/docs/developer_guide/adapters.md` at the pinned commit.

- Rust HTTP and WebSocket clients;
- parsing and model conversion;
- PyO3 exposure;
- Python provider/data/execution/factories/config;
- snapshots and sequence recovery;
- order and account reconciliation;
- runnable examples and integration tests.

Use native Bunting APIs rather than the FIX bridge.

## Dynamic strategy implementation sequence

Use the minimal TypeScript `strategy-loader` only for supported Loader API calls.

- immutable source stored by hash;
- Python Worker wrapper with fixed callbacks;
- `globalOutbound: null`;
- no direct storage or secrets;
- CPU, subrequest, input, output, action and log limits;
- coalesced event batches;
- explicit user state serialization;
- Rust validation of every action;
- recorded source/runtime/SDK/input/output versions;
- tests proving network denial and run survival after strategy failure.

## Cloudflare storage rules

- DO memory: hot projections only.
- DO SQLite: authoritative run event/snapshot/session state.
- D1: global control plane, never matching.
- R2: immutable large artifacts.
- KV: non-authoritative metadata only.
- Cache: public immutable/versioned GETs only.
- Queues: idempotent derived work only.
- Analytics: telemetry only.

Tests must prove that loss or delay of KV, Cache, Queue or analytics cannot change market outcomes.

## Pull request discipline

Each PR must include:

- focused scope and architecture link;
- tests run and their results;
- Wasm compilation status where relevant;
- performance or size measurements for hot-path/dependency changes;
- reference source and license notes for ports;
- explicit deferred work;
- no generated binaries, databases, `.DS_Store`, `__pycache__`, ML runs or local secrets.

Do not modify reference submodules except by a deliberate pin update with an inventory change.

## Definition of done for the first runnable milestone

The milestone is done only when:

- one instrument trades deterministically;
- two participants place limit and market orders;
- price-time priority and partial fills work;
- cancel works;
- risk limits return stable reasons;
- positions and cash reconcile;
- canonical events are replayable;
- snapshot restoration matches direct state;
- native and Wasm checks pass;
- no platform dependency exists in the kernel;
- documentation reflects actual implementation status.
