# ADR 0018: One OrderBook-rs-backed Bunting market engine

- Status: Accepted
- Date: 2026-07-13
- Evidence baseline: `docs/research/rit-binary-audit/`, `docs/reference-functionality-audit.md`, and the existing NBC/RITC/QUARCC port records
- Supersedes: ADR 0014's selectable `orderbook-v1`/`nbc-v1` market-engine registry and separate production-kernel decision
- Preserves: ADR 0014's market-versus-participant authority boundary, ADR 0013's OrderBook-rs dependency decision, ADR 0017's NBC authorization/provenance rules, and ADR 0016's Worker deployment boundary

## Context

ADR 0014 correctly established that NBC is a complete market simulator and QUARCC is a participant execution engine, but it made the default OrderBook-rs-backed engine and `nbc-v1` separately selectable venue kernels. The resulting code now has two matching paths: `packages/orderbook` around released OrderBook-rs and a translated `NbcOrderBook` under `packages/nbc-market-engine`.

Static extraction of the supplied RIT User Application and RTD/API Link installers exposes a broad market contract: period/tick lifecycle, security and asset configuration, orders, depth, history, private account state, risk/fines, tenders, news, leases, settlement categories, scoring inputs, REST/API/RTD data, and ordered synchronization. Those capabilities overlap NBC, RITC, QUARCC and current Bunting concerns. Treating each source as a separate kernel would duplicate authoritative order, ledger, time, event and recovery state and make cross-profile determinism unprovable.

The source systems do not justify multiple sources of venue truth. Their useful differences are compatibility policy, scenario configuration, virtual participants, market-data shape, participant execution behavior, or external transport.

## Decision

### One authoritative kernel

Bunting has one production market engine named `bunting-engine`, implemented as reusable package `packages/bunting-engine` when the first compiling vertical slice is ready. Every run uses this engine; durable run records version the engine and its configuration, not a selectable kernel family.

The engine owns run state, logical time, authoritative order state, matching results, canonical events, public/private market data, scenario state, virtual participants, liquidity, market-wide risk, ledger projections, scoring, termination, snapshot/replay, and state hashes.

### OrderBook-rs remains the matching foundation

Released `orderbook-rs = 0.10.3` with `default-features = false` remains the single production CLOB and order-book foundation. ADR 0019 places its first-party adapter inside `packages/bunting-engine`; the current `packages/orderbook` crate is transitional until that migration. Bunting preserves the useful upstream surface: limit/market orders, IOC, FOK, post-only, iceberg/reserve, pegged, trailing-stop, market-to-limit, partial fills, cancellation, mass cancellation, price-time priority, STP, fees, book risk, kill switch, halt/drain, host-driven GTD/DAY expiry, lifecycle, snapshots/restore, replay helpers, depth, metrics, iterators, analytics and market impact.

Compatibility behavior may validate, transform or schedule a command before matching and may transform results after matching. A narrowly scoped upstream contribution or documented fork remains possible under ADR 0013. A second generic or profile-specific production matching loop is not allowed.

`packages/nbc-market-engine::NbcOrderBook` becomes transitional translated evidence and a differential oracle until the NBC compatibility layer reaches equivalent coverage through OrderBook-rs. It must not remain selectable production venue authority.

### Compatibility profiles surround the same state

NBC, RIT and RITC compatibility profiles can define configuration, supported commands, validation, timing, special-event behavior, output formatting and adapter contracts. They do not own separate books, clocks, ledgers, events, participants or recovery roots.

NBC-derived run lifecycle, step synchronization, scenario behavior, agents, liquidity, stress, scoring and termination are integrated into `bunting-engine` under ADR 0017 provenance. RIT-derived behavior is independently specified from the static evidence ledger and later authorized traces. Unknown formulas remain unresolved rather than invented. RIT REST, VBA and Excel RTD delivery remain external adapters over engine data and commands.

### Participant execution and strategy behavior

QUARCC remains an optional participant-side execution/OMS engine. It consumes public/private reports and submits ordinary commands; it never mutates internal engine state or becomes venue truth. Its lifecycle, duplicate/out-of-order report handling, identifier mapping and reconciliation belong in reusable participant packages.

RITC market-making estimators and quote-generation logic remain pure participant strategy models. Bunting may instantiate an independently implemented model as a built-in virtual trader, but that agent submits ordinary commands through the same risk and matching path. Strategy inventory constraints do not replace authoritative venue risk or ledger state.

### Full-state recovery and commit authority

OrderBook-rs snapshots are a component of the unified snapshot, not the whole snapshot. A complete engine snapshot includes run/clock state, matcher package, instruments, participants, ledger, risk, scheduled events, tenders, news, assets, scoring, agents and RNG streams, compatibility state, subscriptions, command/idempotency state required by the engine boundary, and a canonical state hash.

ADR 0013's origin commit-before-acknowledgement rule remains binding. Workers Cache stores immutable checksum-addressed public snapshot packages and never coordinates transactions. No stream, adapter or agent publishes uncommitted state.

### Package boundaries

Reusable capabilities remain focused packages around the engine when they already have clear roles: market types/events, ledger, risk, origin store, command transaction, market-making models and participant reconciliation. The OrderBook-rs adapter is the exception because it is integral to `bunting-engine` and cannot remain an independently consumed production authority. The engine owns the authoritative transition but does not absorb persistence or platform bindings or turn into a generic utilities package.

No empty target directory or placeholder module is created. Each module appears only with compiling behavior, tests and the scoped instructions required by the repository.

## Consequences

Positive consequences:

- one book, clock, ledger, event sequence and recovery root eliminate cross-kernel authority drift;
- OrderBook-rs capabilities remain available while NBC/RIT behavior is expressed as policies and engine-owned simulation features;
- built-in agents and external participants exercise the same validation, risk, matching and event path;
- compatibility and parity can be measured row-by-row against one implementation.

Costs and constraints:

- the partial NBC matcher must be migrated into differential evidence instead of completed as an independent production engine;
- the unified state and snapshot schema are broader than the current vertical slice;
- several RIT formulas remain blocked on evidence, so complete RIT equivalence cannot be claimed yet;
- profile-specific behavior must remain explicit without reintroducing hidden kernel selection.

## Rejected alternatives

### Keep `orderbook-v1` and `nbc-v1`

Rejected because two production venue kernels duplicate authority and make the requested RIT/NBC/RITC/QUARCC superset depend on cross-engine normalization instead of one coherent transition model.

### Replace OrderBook-rs with a new generic matcher

Rejected because ADR 0013 already selected a broad released matching dependency, and the current evidence does not justify discarding its useful feature set.

### Put participant reconciliation or strategy analytics inside matching

Rejected because desired/live order state and quote models have different authority and failure semantics. Built-in agents may be engine-owned actors, but their orders still pass the public command path.

### Treat RIT as only UI/API compatibility

Rejected because its exposed fields and operations require underlying engine state for timing, risk, tenders, news, assets, positions, valuation and settlement. The transport is external; the authoritative semantics are not.

## Validation

- no durable run or public API selects `orderbook-v1`, `nbc-v1`, or another production kernel;
- dependency graphs contain one production CLOB implementation, released OrderBook-rs, except bounded development-only oracles;
- every engine-relevant parity row names an implementation owner and test;
- upstream OrderBook-rs capability regression tests remain green;
- NBC compatibility fixtures use the unified matcher and scheduler before the transitional matcher can be removed;
- built-in agents submit through ordinary commands and cannot mutate books or ledger state directly;
- full snapshot/restore/replay yields the same canonical state hash natively and on Wasm;
- command commits remain atomic and precede acknowledgement, cache publication and stream publication;
- all repository-required Cargo, formatting, clippy, test, dependency, Wasm and diff checks pass.

## Operational impact

Deployments continue to use one native Rust Cloudflare Worker and direct tRPC dispatch. Run records and snapshots require a schema migration from engine-family selection to unified engine/config/profile versions. Recovery must reject incompatible component versions explicitly. Native RIT/RTD/QUARCC/RITC adapters remain separately deployable and cannot become origin state.

## Security impact

One authority path reduces bypass risk: every user, adapter and agent crosses the same authentication, validation, risk, idempotency and commit boundary. Compatibility profiles cannot weaken global bounds or checked arithmetic. RIT credentials remain injected into native adapters and are never committed or logged. Proprietary extraction artifacts remain outside Git and cannot become build inputs.

## References

- [`../research/rit-binary-audit/README.md`](../research/rit-binary-audit/README.md)
- [`../research/rit-binary-audit/market-feature-ledger.md`](../research/rit-binary-audit/market-feature-ledger.md)
- [`../research/rit-binary-audit/engine-parity-matrix.md`](../research/rit-binary-audit/engine-parity-matrix.md)
- [`0013-worker-orderbook-rs-kernel.md`](0013-worker-orderbook-rs-kernel.md)
- [`0014-market-and-execution-engine-boundaries.md`](0014-market-and-execution-engine-boundaries.md)
- [`0017-authorized-nbc-jar-port.md`](0017-authorized-nbc-jar-port.md)
- [`0019-bunting-engine-package-owns-orderbook-rs.md`](0019-bunting-engine-package-owns-orderbook-rs.md)
- [`../specs/rit-class-market-simulation.md`](../specs/rit-class-market-simulation.md)
- [`../ports/nbc-simulation.md`](../ports/nbc-simulation.md)
- [`../ports/ritc-market-making.md`](../ports/ritc-market-making.md)
- [`../ports/quarcc-trading-engine.md`](../ports/quarcc-trading-engine.md)
