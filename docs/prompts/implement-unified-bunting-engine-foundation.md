# Implementation prompt: central unified Bunting engine foundation

Work in `andrewkoumoudjian/bunting` from the latest clean `main` after documentation PR #15. Create branch `feat/unified-bunting-engine-foundation`. Do not implement later RIT feature families in this PR.

Read and treat as binding before editing:

- root `AGENTS.md` and the nearest scoped `AGENTS.md` for every touched path;
- `docs/architecture.md`;
- ADR 0013, ADR 0016, ADR 0018 and ADR 0019;
- `docs/specs/rit-class-market-simulation.md`;
- `docs/plans/unified-bunting-engine-roadmap.md`;
- `docs/reference-functionality-audit.md` and `docs/reference-adoption.md`;
- `docs/ports/nbc-simulation.md` before touching NBC code.

## Goal

Land the first compiling `packages/bunting-engine` vertical slice. The package is the central authoritative market-simulation engine and directly owns the pinned OrderBook-rs integration. Establish a bounded multi-listing run aggregate and one transition API without adding agents, tenders, OTC, advanced products, reports, REST routes or FIX server behavior.

## Required dependency direction

```text
market-types / market-events / ledger / risk-engine
                     |
                     v
              bunting-engine
          (private OrderBook-rs adapter)
                     |
          +----------+----------+
          v                     v
      origin-store       command-transaction
                                  |
                                  v
                         bunting-rs / trpc-api
```

Adjust the exact graph to avoid cycles, but preserve these ownership rules:

- `bunting-engine` owns `RunState`, listing state, engine configuration/version and authoritative transition semantics;
- `bunting-engine` directly depends on workspace-pinned `orderbook-rs = 0.10.3` and `pricelevel = 0.8.4`;
- applications and orchestration packages do not depend directly on OrderBook-rs;
- origin storage persists engine-owned state but does not execute transitions;
- command transaction coordinates recovery/commit around the engine but does not own a second matcher or mutable run model.

## Scope

1. Add `packages/bunting-engine` with real code, tests and scoped `AGENTS.md`; add it to the root workspace only in the same change.
2. Move the first-party adapter/conversion/snapshot behavior from `packages/orderbook` into a private engine module while preserving current tests and the exact released dependency. Do not copy upstream source.
3. Remove `packages/orderbook` only after every production caller and test is migrated. If a temporary compatibility facade is unavoidable inside the PR, document it and delete it before completion; do not leave two production book APIs.
4. Introduce checked `VenueId`, `ScenarioId`, `ScenarioVersion`, `IterationId` and `ListingKey` types in the appropriate existing domain package.
5. Replace the single-instrument run model with a bounded deterministic map of listing states. Each listing owns symbol/configuration plus an OrderBook-rs snapshot/live book boundary. Preserve one run-level committed sequence.
6. Add minimal immutable `ScenarioDefinition`, `ListingDefinition` and `ParticipantDefinition` types needed to instantiate the migrated one-listing case. Use strict Serde decoding, canonical ordering and a SHA-256 content hash with explicit schema version. Do not add new schema crates until reference-adoption review approves them.
7. Add a sans-I/O engine transition API for the existing submit-limit and cancel commands. No mutable upstream book reference may escape. Outcomes remain canonical Bunting events and ledger/risk changes.
8. Migrate current origin, command-transaction, Worker and `bunting-rs` callers to the engine boundary without changing the public tRPC procedure set or deployed behavior.
9. Version the complete engine snapshot envelope so it can contain multiple listing snapshots. Migrate the existing one-listing state deterministically and reject unsupported versions.
10. Update active docs and the roadmap with exact implemented status; do not describe unimplemented scenario, multi-leg or RIT compatibility behavior as complete.

## Required behavior and tests

- the existing limit order, crossing fill, partial fill, cancel, rejection, risk, ledger, idempotency, expected-version, cache recovery and restart behaviors remain green;
- two independent listings can coexist without order/depth leakage;
- iteration over listing state and canonical serialization are deterministic;
- one command changes exactly one next run sequence even when it updates book, ledger and events;
- a staged failure leaves all listings, ledger, reservations and sequence unchanged;
- snapshot/restore and snapshot-plus-replay reproduce the same full engine state hash;
- direct mutable OrderBook-rs access is impossible through public engine APIs;
- a dependency check proves only `bunting-engine` reaches `orderbook-rs` in the production graph;
- no REST router, server-side FIX, new Durable Object, native-only dependency or unbounded collection enters the Worker graph.

## Evidence and scope rules

- Treat current NBC matching code as a transitional differential oracle. Do not migrate or delete it in this foundation PR unless required to remove a production dependency, and do not claim NBC parity.
- Do not copy RIT binaries, resources, decompiled bodies or unlicensed reference source.
- Do not add `scenario-schema`, `scenario-engine`, `instrument-models`, `settlement-engine`, `portfolio-risk`, `agent-models`, `news-engine`, `reporting` or similar packages in anticipation. Implement only the central package modules required by this vertical slice.
- Keep tRPC as the sole public application API. FIX remains under `clients/fix-bridge`; RIT REST/VBA/RTD remain future external adapters.

## Required checks

Run from repository root:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-engine | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

Also run a stale-dependency search proving production manifests outside `packages/bunting-engine` no longer name `orderbook-rs`, `pricelevel` or `bunting-orderbook`.

## Delivery

Commit and push the branch, then open one focused draft PR against `main`. In the PR body, distinguish existing behavior preserved, new foundation behavior implemented, transitional migrations removed, checks run and any genuinely unresolved item. Do not begin the next roadmap increment in the same PR.
