# Bunting agent instructions

## Mission

Build a Rust market-simulation and exchange-testing platform composed from reusable packages, with a plain Cloudflare Worker deployment target.

## Instruction precedence

- Read this file before changing the repository.
- Read the nearest scoped `AGENTS.md` for every path touched.
- Accepted ADRs and `docs/architecture.md` are binding.
- ADR 0014 defines the distinction between market engines and participant-side execution engines.
- Before reorganizing paths, read `docs/repository-reorganization.md` and follow its Codex execution contract.

## Engine roles

### Market engines

A market engine owns venue/simulation authority: run state, time or step advancement, market configuration, order processing, trades, public market data, snapshots, and deterministic recovery.

- The current default Bunting engine uses released `OrderBook-rs` for CLOB matching.
- NBC is a complete market engine to be ported to Rust. Do not describe or implement NBC as only scenario JSON, a scheduler helper, or a collection of agent models.
- NBC may reuse shared packages when behavior remains compatible, but its coherent engine boundary must remain visible.

### QUARCC execution engine

The QUARCC trading engine is an optional external participant-side execution engine for users, traders, and strategies. It consumes market data, manages local/venue order state, performs reconciliation and participant-side risk, tracks positions, and routes orders to Bunting or another venue.

It must never become authoritative market state or directly mutate a market engine. Bunting must run without it. The existing `quarcc.v1` compatibility crate is the first surface of the port, not its final scope.

## Repository organization

The repository root remains one Cargo workspace and owns the single `Cargo.lock` and workspace-wide `.cargo/config.toml`.

- `packages/`: reusable Rust packages that compose Bunting. This includes primitives, market engines, execution engines, protocols, clients, simulators, and narrowly scoped algorithm libraries.
- `bunting-rs/`: integrated Bunting product/library that imports packages, selects engines, and exposes the curated public API.
- `bunting-rs/crates/`: Bunting-private glue only when the code has no reusable package role.
- `apps/`: deployable Workers, binaries, CLIs, and gateways that depend on `bunting-rs`.
- `scenarios/`: human-reviewable scenario documents, fixtures, and provenance. Runtime NBC logic belongs in the NBC market-engine package.
- `schemas/`: versioned protocol and file schemas.
- `tests/`: cross-package, cross-engine, protocol, and deployment tests.
- `tools/`: repository automation and release tooling.
- `ref/`: read-only source evidence and provenance; never a production path dependency.
- `vendor/`: approved locally built third-party source with license, upstream revision, and patch log.
- `out/`: generated release bundles; ignored and never source of truth.

Do not create a nested Cargo workspace in `bunting-rs`. The root workspace includes `packages/*`, `bunting-rs`, private Bunting crates, and `apps/*`.

## Package discipline

- Reusable code belongs under `packages/`, not under product-private directories.
- A package must have one clear responsibility, explicit dependency direction, workspace metadata/lints, tests, and scoped instructions when needed.
- Packages must not depend on `bunting-rs` or `apps/`; dependency flow is packages -> `bunting-rs` -> apps.
- Avoid generic `common`, `utils`, or catch-all `algorithms` packages. Name packages after a specific behavior or model family.
- Keep mechanical moves separate from semantic renames and feature work.
- Use `git mv`, preserve package names during repository reorganization, and repair Cargo, CI, Wrangler, migrations, docs, scripts, and scoped instructions atomically.

## Binding architecture decisions

- `orderbook-rs = 0.10.3` remains the production matching dependency for the current default market engine.
- Do not create another Bunting-owned generic CLOB when the upstream API provides the required behavior.
- NBC may define engine-specific behavior, but any separate matching implementation requires a documented compatibility need and tests.
- An OrderBook-rs fork requires a dedicated ADR. It may be maintained as a Bunting package or patched vendor source only with complete MIT attribution, an upstream pin, `PATCHES.md`, synchronization policy, and native/Wasm compatibility tests.
- The deployment target is a plain Cloudflare Worker. Do not add a Durable Object requirement without a user-approved ADR.
- Workers Cache stores immutable checksum-addressed public book snapshots; it is not a transaction coordinator.
- Accepted commands, canonical events, idempotency, and optimistic versions remain authoritative in the origin store.
- Commit authoritative state before acknowledgement, cache publication, or stream publication.

## Authority boundaries

Bunting market engines own venue-side identities, matching results, canonical events, authoritative ledger projections, scenario/run state, and market-data publication.

Participant-side packages such as the QUARCC execution engine own local order intent, venue reconciliation, strategy state, participant risk, and client/gateway connectivity. They submit ordinary commands and consume committed reports.

No client, strategy, execution engine, adapter, or agent may mutate a market engine through an internal reference.

## Source and license rules

- Production manifests use packages or released dependencies, never paths under `ref/`.
- Preserve exact repositories, commits, paths, and licenses for copied/adapted material.
- Prefer stable upstream APIs over copied implementation.
- Unlicensed NBC and QUARCC sources may be used only according to documented ownership/license and clean-room rules; do not mechanically translate implementation text without authorization.
- Worker-bound packages must compile for `wasm32-unknown-unknown` unless explicitly native-only and excluded from the Worker dependency graph.
- Keep fixed-point and checked arithmetic at market, protocol, execution, and ledger boundaries.
- Keep all request, event, snapshot, queue, subscription, and recovery buffers bounded.
- Do not commit `target/`, Worker `build/`, `out/`, database, credential, or secret files.

## Required checks

Run from repository root before marking work complete:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

For path changes, also verify Worker build output, migration discovery, release assembly under ignored `out/`, stale-path searches, dependency direction, and active documentation of NBC/QUARCC roles.