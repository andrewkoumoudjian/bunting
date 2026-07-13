# ADR 0019: The central Bunting engine package owns the OrderBook-rs integration

- Status: Accepted
- Date: 2026-07-13
- Clarifies: ADR 0018 package ownership
- Supersedes: the production package boundary in ADR 0013 and ADR 0014 that leaves the OrderBook-rs adapter as an independently consumed `packages/orderbook` crate
- Preserves: the released dependency and upstream-first rules in ADR 0013, the authority boundaries in ADR 0014 and ADR 0018, the native tRPC Worker in ADR 0016, and the NBC provenance rules in ADR 0017

## Context

ADR 0018 selects one authoritative production engine, `bunting-engine`, backed by released OrderBook-rs. The current implementation predates that decision: `packages/orderbook` owns `KernelBook`, conversion helpers and snapshot handling, while `packages/command-transaction` and `apps/trpc-api` consume that crate directly.

That transitional graph makes the matcher look like a peer service that callers may bypass the engine to use. It also leaves the central package without clear ownership of listing books, cross-book transitions and complete engine snapshots. Bunting needs one package boundary through which all authoritative market mutations pass.

## Decision

### `bunting-engine` is the central market-simulation package

`packages/bunting-engine` owns the authoritative run transition, multi-listing state, logical clock, matching integration, participant and market projections, scenario actions, agents, news, tenders, assets, settlement, scoring and complete deterministic snapshot/state hash.

Focused packages below it may continue to own reusable value types, canonical events, ledger calculations and risk rules. They do not expose an alternate mutation path. Persistence, command-transaction orchestration, tRPC transport and deployable applications remain outside the engine package.

### OrderBook-rs is an internal engine component

`bunting-engine` directly depends on exactly:

```toml
orderbook-rs = { version = "=0.10.3", default-features = false }
pricelevel = "=0.8.4"
```

The first-party conversions, checked clock injection, order ownership mapping, result translation and snapshot wrapping live in a private engine module. Public callers submit Bunting commands and receive canonical Bunting outcomes; they cannot obtain a mutable `DefaultOrderBook`, choose another matcher or commit a match outside the engine transition.

The current `packages/orderbook` crate is transitional. Its tested behavior moves into `packages/bunting-engine` with callers migrated atomically. It is then removed from the production workspace rather than retained as a second public kernel boundary. This migration does not copy upstream source into Bunting.

### One run owns many listing books

Each run is one deterministic aggregate with a single committed sequence. It owns a bounded map from `(VenueId, InstrumentId)` listing keys to independently matched OrderBook-rs books plus the shared participants, ledger, clock, scheduled actions, agents, assets, news and scores needed for atomic cross-book behavior.

Candidate-state staging may touch several books, ledgers and facilities. A command either produces one canonical event batch and one next run version or produces no authoritative mutation. Wall-clock arrival never replaces the recorded run sequence or logical time as the ordering source.

### Protocols remain outside market authority

The native Rust Worker exposes only the versioned tRPC contract and calls `bunting-engine`. RIT REST/VBA/RTD compatibility is implemented by external adapters over tRPC. FIX remains a participant-side native client bridge that owns FIX sessions and maps them to tRPC; neither FIX nor a REST server belongs inside `bunting-engine` or `apps/trpc-api`.

## Consequences

The package name now matches the authority boundary: the platform has one central simulation engine and one internal production matcher. Multi-market, multi-leg, scoring and recovery work can share one state root without allowing applications to manipulate books directly.

The migration is semantic rather than a directory-only move. Origin-state types and command transactions must be rearranged without dependency cycles, and all current `bunting-orderbook` tests must move or be replaced before the transitional crate is deleted.

## Rejected alternatives

### Keep `packages/orderbook` as a public production peer

Rejected because Worker or transaction code could continue to depend on matching without the complete engine invariants, and the public package graph would contradict the single-authority decision.

### Copy or fork OrderBook-rs into `bunting-engine`

Rejected because package ownership does not change upstream provenance. Bunting consumes the pinned released crate and follows the ADR 0013 escalation path for upstream defects.

### Put tRPC, FIX, REST or Worker bindings inside the engine

Rejected because transport lifecycle and venue authority have different portability, security and recovery concerns. The engine remains sans-I/O and Wasm-compatible; adapters submit ordinary commands.

### Create separate scenario, product or NBC engines

Rejected because those capabilities change configuration and behavior around the same books, ledger, clock and event sequence. A focused package is extracted only when a second real consumer proves a reusable non-authoritative boundary.

## Validation

- `packages/bunting-engine/Cargo.toml` directly pins the workspace OrderBook-rs and PriceLevel dependencies;
- no production package or app other than `bunting-engine` depends directly on `orderbook-rs`, `pricelevel` or the transitional `bunting-orderbook` crate after migration;
- no public API returns a mutable upstream book reference;
- every accepted order, scheduled agent action, tender, OTC booking, settlement and admin mutation crosses one engine transition;
- multi-listing candidate failure leaves every book, ledger, facility and sequence unchanged;
- the full engine snapshot nests all listing book snapshots and restores to the same canonical state hash natively and on `wasm32-unknown-unknown`;
- the exact OrderBook-rs `0.10.3` dependency remains visible in `cargo tree -p bunting-engine`;
- the native Worker exposes tRPC only, and the FIX bridge remains client-side.

## Operational impact

The migration lands as a compiling vertical slice. During that change, persistence and Worker callers move to the engine API in the same PR; no deployment may contain both an authoritative engine path and a direct application-to-orderbook path. Persisted snapshots receive an explicit engine/schema version and reject incompatible restores.

## Security impact

Encapsulation prevents protocol adapters, agents and application code from bypassing authentication, risk, idempotency, ledger or commit-before-publish rules through a raw matcher handle. Multi-book and private-state outputs retain explicit audience filtering, bounded collections and checked arithmetic.

## References

- [`0013-worker-orderbook-rs-kernel.md`](0013-worker-orderbook-rs-kernel.md)
- [`0016-native-rust-trpc-worker.md`](0016-native-rust-trpc-worker.md)
- [`0018-unified-bunting-engine.md`](0018-unified-bunting-engine.md)
- [`../architecture.md`](../architecture.md)
- [`../specs/rit-class-market-simulation.md`](../specs/rit-class-market-simulation.md)
- [`../plans/unified-bunting-engine-roadmap.md`](../plans/unified-bunting-engine-roadmap.md)
