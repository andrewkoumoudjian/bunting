# Port plan: QUARCC trading/execution engine to Rust

## Role

The QUARCC trading engine is a participant-side execution/OMS service for a user, trader, account, or strategy operating outside a market. It consumes market data and strategy/user intents, applies participant-side controls, routes orders to a gateway, processes execution reports, and projects positions.

It is not a venue matching engine and does not own authoritative Bunting market state.

See ADR 0014 and [`../reference-functionality-audit.md`](../reference-functionality-audit.md).

## Evidence baseline

The checked-in reference tree contains:

- protobuf contracts under `contracts/`;
- C++ source and headers under `engine-cpp/`;
- a Python client and strategy examples under `python_client/`;
- build and protocol-generation scripts;
- unit/integration test material.

No repository-level license is recorded in the current audit. Do not mechanically translate implementation text without documented authority.

## Observed reference functionality

### Execution service

`contracts/execution_service.proto` and `TradingEngine` prove an execution service with:

- `SubmitSignal`;
- `CancelOrder`;
- `ReplaceOrder`;
- bidirectional/streamed signal submission in the protobuf service;
- `GetPosition` and `GetAllPositions`;
- global kill-switch activation;
- server-side market-data streaming;
- accepted/rejected responses with order IDs, reasons and received timestamps.

The service uses legacy floating-point position/market fields at its external contract boundary.

### Strategy/account ownership

`TradingEngine` stores an `OrderManager` per strategy ID and configures feed/gateway/service components. This is participant/account-scoped execution state, not a shared exchange book.

### Order manager

The recorded `OrderManager` owns or consumes:

- account ID;
- `PositionKeeper`;
- `IExecutionGateway`;
- `IJournal`;
- `IOrderStore`;
- `RiskManager`;
- local order ID generator and local/broker ID mapper;
- strategy signal conversion to orders;
- submit/cancel/replace processing;
- cancel-all behavior;
- position queries;
- a sequential dispatch queue for market-data and execution-report events;
- deferred fill handling when execution reports arrive before a required ID mapping exists;
- market-data sink callbacks for a connected strategy client.

The event queue serializes handling inside an order manager, even though gRPC/feed/gateway callers can enqueue from different threads.

### Gateways and feeds

The source defines interfaces and implementations for:

- execution gateways;
- market-data feeds;
- simulated/paper behavior;
- optional external broker integration;
- feed registration and poll scheduling.

The gateway executes or routes an order elsewhere. It is not an exchange matcher inside the QUARCC engine.

### Persistence and observability

The tree includes:

- journal interfaces and SQLite journal implementation;
- order-store interfaces and SQLite order-store implementation;
- database/debug scripts;
- observability/build modules.

These are native participant-application facilities.

### Client surface

The Python package proves:

- generated gRPC client use;
- strategy signal submission;
- market-data consumption;
- configuration-driven client/strategy examples.

## Behaviors requiring further source/test verification

Before claiming exact compatibility, produce evidence for:

- the complete order-state transition table;
- duplicate execution-report handling and deduplication keys;
- out-of-order acknowledgement/fill/cancel behavior beyond the documented deferred-fill case;
- exact risk rules and limit units;
- replace semantics and ID retention;
- journal ordering and crash-recovery guarantees;
- SQLite transaction boundaries;
- position/PnL formulas and correction behavior;
- gateway reconnect and open-order reconciliation;
- kill-switch persistence and restart semantics;
- thread race behavior not intentionally part of the domain contract.

Do not infer these solely from class names or proposed Rust architecture.

## License and clean-room status

Until ownership/license or direct-port authority is recorded:

- preserve public contract names/discriminants only as required for compatibility;
- derive portable behavior from public interfaces, tests, captured sequences and an independently written transition specification;
- do not copy C++ implementation text or comments mechanically;
- record all source paths, evidence and divergences;
- isolate externally licensed broker/protocol dependencies from the clean portable core.

## Existing Rust implementation

The current Rust crate provides:

- WASM-safe `quarcc.v1` records/enums;
- market-data and position records;
- transport-neutral `ExecutionService` trait;
- preserved public names/enum discriminants;
- no matching engine, gateway, persistence, reconciliation, risk or position implementation.

This is a compatibility-contract seed, not a complete execution-engine port.

Legacy floating-point values remain at the compatibility edge and must convert through checked Bunting units before they drive canonical execution, risk or position state.

## Bunting-added Rust port requirements

The following are design requirements for the new Rust package, not claims about the C++ implementation:

- portable sans-I/O execution/lifecycle core;
- typed client/local/venue IDs and explicit collision states;
- normalized venue reports independent of gRPC/FIX/HTTP;
- checked fixed-point prices, quantities, money and positions;
- bounded desired/live reconciliation and action planning;
- explicit duplicate/out-of-order report handling;
- deterministic snapshots/replay for portable execution state;
- clear distinction between local estimates and authoritative venue reports;
- public Bunting client adapter;
- separate native gRPC, database, filesystem, socket and broker packages/features;
- capability/version metadata and typed errors.

## Target package topology

The mechanical repository move should preserve the current package name first. A later semantic change should produce:

```text
packages/quarcc-execution-engine/
  AGENTS.md
  Cargo.toml
  src/
    lib.rs
    capabilities.rs
    config.rs
    command.rs
    event.rs
    ids.rs
    order.rs
    lifecycle.rs
    normalized_report.rs
    reconciliation.rs
    planner.rs
    positions.rs
    risk.rs
    market_data.rs
    strategy_signal.rs
    journal.rs
    snapshot.rs
    errors.rs
  tests/
    compatibility.rs
    lifecycle.rs
    report_ordering.rs
    reconciliation.rs
    recovery.rs
```

Native layers should be separate when they pull native runtimes or storage into the graph:

```text
packages/quarcc-execution-grpc/     # generated service/client adapter
packages/quarcc-execution-sqlite/   # optional native journal/order store
packages/quarcc-execution-gateway-* # concrete venue/broker adapters
```

Do not create these packages until implementation begins. The portable core may instead use narrowly controlled optional features when target isolation remains provable.

## Portable execution model

### Commands/intents

At minimum:

- submit;
- cancel;
- replace;
- cancel all / kill switch;
- query/reconcile;
- subscribe/unsubscribe market data.

Commands record correlation, causation, configuration revision and idempotency identity.

### Normalized venue reports

A normalized report should carry, when available:

- venue/source identity;
- report/deduplication ID;
- client, local and venue order IDs;
- state/status and reason;
- last and cumulative quantity;
- exact execution/order price;
- received/venue sequence metadata;
- authoritative account/position metadata where supplied.

This normalized model is Bunting-added unless a direct equivalent is proven in the reference.

### Lifecycle and reconciliation

The Rust design should model pending, live, partially filled, cancel/replace pending, terminal, externally discovered and quarantined states. The exact transition table must be derived from recorded QUARCC tests/contracts and explicitly supplemented where Bunting adds safe behavior.

Request submission success does not imply venue acknowledgement or fill.

Desired-versus-live reconciliation is a proposed Rust capability. It should:

- compare desired local intent with authoritative venue/open-order reports;
- plan bounded submit/cancel/replace/requery actions;
- make duplicates idempotent;
- quarantine ambiguity rather than silently repair it;
- survive reconnect/reset without mutating venue state directly.

### Participant risk and positions

The package owns participant-side pre-route controls and local projections. It must not claim local state is authoritative market/account truth. Reconciliation with Bunting private account reports is explicit.

### Transport/gateway boundary

The core emits routing intents and consumes normalized reports. Adapters may include:

- Bunting HTTP/WebSocket client;
- FIX session/gateway;
- legacy `quarcc.v1` gRPC;
- simulated venue;
- separately licensed external gateway.

Transport code does not define lifecycle semantics.

## Authority boundary

The QUARCC execution engine may:

- consume public market data and private reports;
- submit ordinary venue commands;
- maintain local order/position/risk/reconciliation state;
- apply stricter participant controls;
- stop routing and request cancellation;
- persist its own application state.

It may not:

- match venue orders;
- assign authoritative venue event sequences;
- write Bunting canonical events/origin state directly;
- mutate a market engine or ledger through an internal reference;
- bypass authentication, venue risk, idempotency or expected-version checks;
- treat transport delivery as execution.

## Relationship to other audited references

- NautilusTrader and Barter are broad participant trading-platform architecture references.
- market-maker-rs and `ritc_mm` are participant strategy/model references.
- IronFix/fixer/FerrumFIX/QuickFIX/J are protocol/session references, not execution lifecycle authority.
- the NBC student client is a thin participant protocol client, not an OMS replacement.

## Port phases

### Phase 0: exact behavioral inventory

1. Record reference-tree commit, license status and source paths.
2. Inventory every public contract, state field, gateway/feed/store/journal interface and test.
3. Produce an evidence-linked language-neutral transition table.
4. Mark observed, inferred, Bunting-added and unresolved behavior.
5. Capture representative C++/Python service traces where authorized.

### Phase 1: compatibility contract and exact units

1. Retain the existing `quarcc.v1` types/discriminants.
2. Add strict checked conversions.
3. Define typed IDs and normalized reports.
4. Add serialization/service fixtures.

### Phase 2: portable lifecycle core

1. Implement only transitions proved or explicitly specified.
2. Add invalid-transition and quarantine outcomes.
3. Add position projection and participant risk under exact units.
4. Add property tests for report permutations/duplicates.

### Phase 3: reconciliation and recovery

1. Implement the Bunting-added desired/live planner.
2. Add snapshot/replay for portable state.
3. Add deterministic kill-switch/cancel planning.
4. Test reconnect/open-order reconciliation against a deterministic fake venue.

### Phase 4: Bunting client integration

1. Route through the public Bunting client package.
2. Consume committed market/private reports.
3. Handle stream reset, idempotency and expected-version conflicts.
4. Test against the default market engine and NBC through external interfaces.

### Phase 5: optional native compatibility

Add gRPC generation/client/server adapters, Python packaging, SQLite/filesystem stores, FIX and external gateways only behind separately reviewed native layers.

## Required tests

### Reference compatibility

- protobuf records/discriminants and service response shapes;
- submit/cancel/replace/positions/kill-switch/market-data fixtures;
- source-proven order-manager sequences.

### Lifecycle and report ordering

- submit, acknowledge, reject, partial/full fill, cancel, cancel reject and replace;
- fill before ID mapping/acknowledgement where specified;
- duplicate and out-of-order reports;
- unknown/colliding IDs and quarantine;
- transition-table coverage.

### Reconciliation and recovery

- reconnect discovers missing/unexpected orders;
- desired/live plans are bounded and deterministic;
- repeated snapshots/reports are idempotent;
- snapshot plus replay equals uninterrupted Rust execution;
- transport/storage failures do not corrupt lifecycle state.

### Authority integration

- deterministic fake venue;
- default Bunting market engine;
- NBC market engine;
- no adapter has direct mutable access to venue state.

## Completion criteria

The first complete Rust port allows a user to run QUARCC outside a selected market engine, consume market data, submit strategy/user intents, route through a supported adapter, reconcile execution reports, enforce participant-side controls, recover its own state, and inspect positions/orders without claiming venue authority or unsupported equivalence to unverified C++ behavior.
