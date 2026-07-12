# Port plan: QUARCC trading/execution engine to Rust

## Role

The QUARCC trading engine is a participant-side execution engine for a user, trader, or strategy operating outside the market. It is not a market engine and does not own authoritative venue state.

Its purpose in Bunting is to let users test and optionally enable a realistic execution stack against a Bunting market engine or another venue. It consumes market data and strategy signals, manages local and venue order state, applies participant-side controls, routes orders, reconciles reports, and tracks positions.

Bunting must remain usable without this package.

See ADR 0014.

## Source

- reference tree: `ref/quarcc-trading-engine`;
- public contracts under `contracts/`;
- C++ engine under `engine-cpp/`;
- Python client under `python_client/`;
- key concepts include trading engine, order manager, position keeper, feed registry, simulated feed, execution service, order store, journal, risk checks, gateways, and gRPC server.

## License status

No repository-level license was identified in the recorded audit. Until ownership and license are resolved:

- preserve public service/record names and discriminants only as required for compatibility;
- do not copy or mechanically translate implementation text;
- derive behavior from interfaces, tests, public contracts, captured sequences, and an independently written transition specification;
- record any authority or relicensing decision before direct porting.

## Target package

The mechanical repository move preserves the current package name first:

```text
packages/quarcc-trading-engine/
```

A later semantic PR should rename and expand it to:

```text
packages/quarcc-execution-engine/
  AGENTS.md
  Cargo.toml
  src/
    lib.rs
    config.rs
    engine.rs
    command.rs
    event.rs
    errors.rs
    ids.rs
    order.rs
    lifecycle.rs
    reconciliation.rs
    planner.rs
    positions.rs
    risk.rs
    market_data.rs
    strategy_signal.rs
    journal.rs
    snapshot.rs
    transport.rs
  tests/
    lifecycle.rs
    reconciliation.rs
    recovery.rs
    simulated_market.rs
```

Native-only adapters should be isolated from the portable core, for example:

```text
packages/quarcc-execution-native/
  gRPC, SQLite/filesystem journals, native sockets, external broker gateways
```

or as clearly separated optional features if that does not contaminate the Wasm-safe dependency graph.

## Existing implementation

The current Rust crate provides a WASM-safe compatibility surface for the legacy `quarcc.v1` records, enums, market-data/position records, and transport-neutral `ExecutionService` trait.

That work is a valid first layer, but it is not the final purpose of the package. The port must grow into a participant-side execution engine while preserving the compatibility contract.

Legacy floating-point fields remain quarantined at the protocol boundary and must convert through checked fixed-point types before entering execution, risk, or position logic.

## Execution-engine responsibilities

### Strategy and user input

- accept strategy signals or explicit user order intents;
- validate bounded order parameters;
- assign stable client/local identifiers;
- preserve correlation and causation across retries;
- expose typed submit, cancel, replace, and query intents.

### Market-data consumption

- subscribe to committed Bunting or venue market data;
- maintain a bounded local view suitable for strategy/execution decisions;
- detect stream gaps, resets, and stale data;
- never treat local market data as authoritative venue state.

### Order management

Model at least:

- pending submit;
- acknowledged/live;
- partially filled;
- cancel pending;
- replace pending;
- canceled;
- rejected;
- fully filled;
- externally discovered after reconnect;
- quarantined unknown/collision state.

Request success does not imply venue acknowledgement or fill.

### Reconciliation

- map client, local, and venue order identifiers;
- handle duplicate and out-of-order reports idempotently;
- reconcile desired orders with live venue state;
- recover after reconnect from venue snapshots and report tails;
- quarantine impossible or ambiguous states instead of silently mutating;
- produce bounded submit/cancel/replace/requery actions.

### Participant risk and positions

- project positions and cash/exposure from execution reports;
- enforce user-configured execution limits before routing;
- expose a participant-side kill switch and deterministic cancel plan;
- distinguish local estimates from authoritative venue/Bunting account reports;
- reconcile discrepancies explicitly.

### Routing and gateways

Route through transport adapters such as:

- Bunting native HTTP/WebSocket client;
- FIX;
- legacy `quarcc.v1` gRPC;
- simulated feed/venue adapters;
- future external broker gateways.

Transport code must not define lifecycle semantics. The core engine emits routing intents and consumes normalized venue reports.

### Journal and recovery

- persist command intent, normalized reports, lifecycle transitions, positions, and configuration revisions;
- snapshot portable state;
- replay deterministically;
- keep native storage adapters outside the portable core.

## Authority boundary

The QUARCC execution engine may:

- submit commands;
- consume market data and private execution/account reports;
- maintain local execution state;
- apply stricter participant-side limits;
- retry, cancel, reconcile, and stop routing.

It may not:

- match orders;
- write Bunting canonical events directly;
- mutate a market engine or authoritative ledger through an internal reference;
- assign authoritative venue sequence numbers;
- treat local acknowledgements or positions as venue truth;
- bypass Bunting authentication, risk, idempotency, or expected-version rules.

## Relationship to Bunting packages

The execution engine should depend on reusable package surfaces such as:

- `packages/market-types` for exact IDs and units;
- `packages/client` for Bunting transport and stream recovery;
- `packages/fix` for FIX codecs/sessions when enabled;
- narrowly scoped reconciliation and journal packages if later extracted.

It must not depend on `bunting-rs` internals or `apps/edge-api`. Integration occurs through public client/protocol contracts.

## Relationship to market engines

The same QUARCC execution engine should be testable against:

- a deterministic in-memory fake venue;
- the default OrderBook-rs-backed Bunting market engine;
- the NBC Rust market engine;
- external venues through gateways where supported.

Market-engine-specific details belong in adapters. The execution lifecycle core should consume normalized reports.

## Port phases

### Phase 0: behavioral inventory

1. Inventory contracts, engine state, order manager, position keeper, feed registry, simulated feed, gateways, storage, journal, risk, tests, and failure paths.
2. Produce a language-neutral lifecycle/reconciliation table.
3. Resolve license/ownership or establish clean-room rules.
4. Record which behaviors are portable core versus native adapter behavior.

### Phase 1: compatibility and exact types

1. Retain the existing `quarcc.v1` records and discriminants.
2. Add checked canonical conversions.
3. Define typed local/client/venue IDs and normalized venue reports.
4. Add compatibility fixtures for serialization and service behavior.

### Phase 2: portable execution core

1. Implement lifecycle transitions and invalid-transition rejection.
2. Implement desired/live reconciliation and bounded action planning.
3. Implement participant positions, execution risk, kill switch, snapshots, and replay.
4. Add property tests for arbitrary report ordering and duplication.

### Phase 3: Bunting client integration

1. Connect through the public Bunting client package.
2. Consume committed market data and private reports.
3. Route submit/cancel/replace commands with idempotency and expected versions.
4. Test reconnect/reset recovery.

### Phase 4: optional native adapters

Add gRPC packaging, Python bindings/wheel, SQLite or file journals, FIX, and external broker gateways only behind isolated adapters/features with dedicated tests and licenses.

### Phase 5: user-facing enablement

- expose configuration for enabling the QUARCC engine in a user test setup;
- provide example strategy-signal and manual-order flows;
- provide observability for orders, positions, risk, reconciliation, and gateway state;
- make clear that enabling the engine does not change the selected market engine.

## Required tests

### Lifecycle

- submit, acknowledge, reject, partial fill, full fill, cancel, cancel reject, and replace;
- fill before acknowledgement;
- cancel versus fill ordering;
- duplicate report IDs;
- unknown/colliding external IDs;
- invalid transitions and quarantine.

### Reconciliation

- reconnect discovers missing or unexpected live orders;
- desired/live planning is bounded and deterministic;
- repeated venue snapshots are idempotent;
- stale market data or missing private reports produces an explicit safe state.

### Positions and risk

- execution-driven position projection;
- local versus authoritative position reconciliation;
- pre-route limit rejection;
- kill switch blocks new orders and generates deterministic cancel actions.

### Recovery

- snapshot plus replay equals uninterrupted execution;
- journal duplicates do not change state;
- corrupted/incompatible snapshots reject safely.

### Integration

- simulated venue adapter;
- default Bunting market engine;
- NBC market engine;
- Bunting stream reset and expected-version conflict;
- transport failure without lifecycle corruption.

## Completion criteria

The port is complete when a user can optionally run the QUARCC execution engine outside a selected market engine, feed it strategy/user intents and market data, route orders through a supported adapter, recover/reconcile safely, and inspect deterministic order, risk, and position state.