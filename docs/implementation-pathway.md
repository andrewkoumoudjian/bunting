# Implementation pathway

This pathway implements ADR 0013 and replaces the previous custom-order-book/Durable-Object sequence.

## Completed in the corrective integration PR

1. Pin `OrderBook-rs` `0.10.3` and `PriceLevel` `0.8.4` as workspace dependencies.
2. Add `crates/orderbook` as a thin upstream adapter.
3. Salvage Bunting-owned checked identifiers, events, ledger, and participant risk from the old Codex branch.
4. Add `crates/worker-cache` with immutable content-addressed Workers Cache keys and official Cache API operations.
5. Add a plain `workers/edge-api` Rust Worker without a Durable Object binding.
6. Replace conflicting architecture, ADR, reference, and Codex instructions.
7. Pin the high-value Joaquín repositories under `ref/` and document their adoption status.

## Implemented: command transaction and origin store

The current implementation completes this vertical slice:

```text
submit limit -> expected-version load -> restore/cache -> risk -> OrderBook-rs
             -> canonical events -> ledger -> origin commit -> cache put -> response
```

Implemented work:

- D1 schema for run version, commands, canonical events, idempotency, private projections, and snapshot metadata;
- transactional D1 batch guarded by expected version, plus a contended in-memory concurrency proof;
- upstream-to-Bunting event translation;
- participant/order ownership index;
- exact conversion between Bunting IDs/units and upstream IDs/units;
- cache miss and invalid-snapshot recovery;
- authenticated bounded Worker routes for limit GTC submission and cancellation;
- integration tests for duplicates, command-ID conflict, version conflict, fills, owner cancellation, risk rejection, cache/origin failure, and restart.

The initial slice stores a complete authoritative package and private projection after every command. Its recovery event tail is therefore empty, while canonical events remain durable for audit and later coarser snapshot intervals.

## Immediate PR: repository organization

Execute [`repository-reorganization.md`](repository-reorganization.md) as a behavior-preserving pull request:

- keep the repository root as the Cargo workspace root;
- add `bunting-rs` as the small public facade package;
- keep internal Rust libraries under `crates/`;
- move `workers/edge-api` to `apps/edge-api` with history preserved;
- reserve `packages/` for independently distributed SDKs rather than internal crates;
- repair Cargo, CI, Wrangler, migration, documentation, and agent-instruction paths atomically;
- establish an ignored `dist/` release boundary without committing generated output.

Do not combine this move with crate renames, dependency upgrades, new order behavior, streaming, or persistence changes.

## Immediate product-readiness PR: staging and run provisioning

The order routes intentionally reject unknown runs, so a usable deployment requires an explicit provisioning boundary before streaming becomes valuable.

- create the real D1 database and environment-specific configuration;
- apply migrations and install the API token secret;
- add an administrative run-provisioning API or CLI for runs, instruments, participants, opening balances, and limits;
- keep provisioning authenticated, idempotent, bounded, and separate from participant order entry;
- add staging smoke tests for provisioning, submit, cancel, duplicate command, stale version, cache miss, and restart recovery;
- document migration, rollback, secret rotation, and environment promotion.

## Following PR: streaming

- plain Worker WebSocket endpoint;
- snapshot plus absolute L1/L2 updates;
- committed event-sequence cursors;
- reset and event-tail recovery;
- bounded subscriptions, frames, and backlog;
- no reliance on isolate-local resume rings;
- no public or private publication before the origin commit succeeds.

## Following PR: broader upstream capabilities

Expose upstream features incrementally instead of reimplementing them:

- IOC, FOK, post-only, replace, mass cancel, STP, and fees;
- host-driven GTD/DAY expiry;
- upstream risk configuration;
- lifecycle history and typed rejects;
- depth/metrics/market impact/enriched snapshots;
- snapshot/replay verification and upgrade tests.

## Scenarios and strategies

Scenario agents propose commands and never modify the upstream book directly. Use explicit logical time and named deterministic random streams.

Dynamic Worker outputs remain external participant actions and use the same expected-version command path.

Prioritize:

1. scenario schema and provenance;
2. deterministic run clock and named PRNG streams;
3. explicit participant and instrument provisioning;
4. NBC agent-model ports with unresolved legacy values retained as metadata;
5. scoring, replay, and conformance fixtures.

## FIX and native adapters

Protocol implementations translate to Bunting commands and events. They do not own a second matching engine.

- IronFix remains the FIX codec candidate.
- QuickFIX/J and Fixer remain conformance oracles.
- Nautilus and RITC remain external/native adapters.
- IronSBE is evaluated later for compact market-data and order-entry frames.
- A Rust FIX codec belongs in a focused crate; a deployable gateway belongs under `apps/` only when implemented.

## SDK and release packaging

- expose stable client-facing APIs through the `bunting-rs` facade;
- place independently versioned Python or JavaScript SDKs under `packages/`;
- produce the complete Worker bundle and raw Wasm under ignored `dist/` paths;
- attach versioned bundles, checksums, and build metadata to GitHub Releases;
- keep native gRPC and Python compatibility packaging blocked until licensing and source-provenance requirements are resolved.

## Dependency upgrade gate

Every OrderBook-rs upgrade must run:

- native and Wasm compilation;
- limit/market/cancel/partial-fill tests;
- snapshot checksum and restore tests;
- risk, kill-switch, expiry, and deterministic mass-cancel tests;
- cache round-trip tests;
- size and cold-start comparison;
- review of snapshot format, public API, and PriceLevel version changes.
