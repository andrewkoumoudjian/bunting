# Bunting agent instructions

## Mission

Build a stock-market simulation and exchange-testing platform whose runtime is a plain Rust Cloudflare Worker and whose matching core is the released `OrderBook-rs` crate.

## Instruction precedence

- Read this file before changing the repository.
- Read the nearest scoped `AGENTS.md` for every path touched; scoped instructions add to or narrow these rules.
- Accepted ADRs and `docs/architecture.md` are binding. When documentation conflicts, stop and reconcile the active decision instead of silently choosing one.
- Before any path reorganization, read `docs/repository-reorganization.md` and follow its Codex execution contract.

## Repository organization

The repository root remains the Cargo workspace root and owns the single `Cargo.lock` and workspace-wide `.cargo/config.toml`.

- `bunting-rs/`: public Rust facade package only. Keep `src/lib.rs` small and limited to curated stable re-exports.
- `crates/`: internal reusable Rust libraries and domain boundaries.
- `apps/`: deployable binaries and services. The current `workers/edge-api` path is planned to move atomically to `apps/edge-api`.
- `packages/`: independently released user-facing packages outside the internal Rust crate graph, such as Python or JavaScript SDKs. Do not move internal Rust crates here.
- `scenarios/`: canonical scenario documents, fixtures, and provenance; executable scenario behavior belongs in crates.
- `schemas/`: versioned protocol and file schemas when they exist.
- `tests/`: cross-package conformance and black-box tests; crate unit tests stay with their crate.
- `tools/`: repository automation or a future `xtask`.
- `ref/`: read-only evidence and provenance; never a production path dependency.
- `vendor/`: approved locally built third-party source only, with license, upstream revision, and patch log.
- `dist/`: generated release bundles; never source of truth and never committed.

Do not create empty directories for a hypothetical future. Do not maintain both old and new active paths during a move. Use `git mv`, preserve Cargo package names in the mechanical reorganization, and repair manifests, CI, Wrangler commands, documentation, migrations, scoped instructions, and scripts in the same pull request. Crate or API renames belong in later focused changes.

## Binding architecture decisions

- `orderbook-rs = 0.10.3` is the production matching and order-book dependency.
- Do not create a second Bunting-owned limit-order book, price-level queue, matching loop, snapshot format, replay engine, kill switch, or market-depth analytics layer when the upstream API already provides it.
- A minimal attributed fork is permitted only when an upstream Wasm incompatibility cannot be fixed through features or an upstream contribution. Preserve the MIT license, API shape, tests, and an exact divergence log. An approved local fork belongs under `vendor/`, not `packages/`.
- The deployment target is a plain Cloudflare Worker. Do not introduce a Durable Object requirement without a new user-approved ADR.
- The Cloudflare Workers Cache API is mandatory for immutable, checksum-addressed `OrderBook-rs` snapshot packages.
- Worker global memory may cache reconstructed books during one warm isolate lifetime, but it is never the only recoverable copy.
- Canonical accepted-command history and optimistic stream versions remain in an origin store. Workers Cache is an acceleration and distribution layer, not a transaction coordinator.
- Commit authoritative state before acknowledgement, cache publication, or streaming publication.

## Bunting-owned responsibilities

Bunting owns authentication, authorization, run and participant identifiers, canonical event envelopes, idempotency, participant cash and position accounting, scenario scheduling, protocol translation, Worker routes, persistence orchestration, streaming recovery, and Dynamic Worker strategy isolation.

## Upstream responsibilities

Use `OrderBook-rs` directly for order types, price levels, price-time matching, trade generation, snapshots and restore, engine sequencing, market depth, metrics, market-impact simulation, self-trade prevention, fees, risk hooks, order lifecycle tracking, mass cancel, expiry sweeps, and the operational kill switch.

## Source and dependency rules

- Production manifests use released packages rather than paths under `ref/`.
- Preserve upstream licenses and exact source paths for any copied example or test.
- Prefer calling a stable upstream API over copying its implementation.
- Copy MIT-licensed examples or tests only when adaptation creates a Bunting-specific fixture or boundary test, and record the upstream repository, commit, path, license, and divergence.
- Worker-bound packages must compile for `wasm32-unknown-unknown`.
- Keep fixed-point and checked arithmetic at Bunting protocol and ledger boundaries.
- Keep request, response, snapshot, subscription, event-batch, and recovery buffers bounded.
- Do not commit generated `target/`, Worker `build/`, `dist/`, coverage, database, or secret files.

## Change discipline

- Keep mechanical moves separate from semantic behavior changes.
- Preserve public package names and Rust paths during repository moves.
- Add new crates only with one clear owner, a narrow dependency direction, package metadata, lints inherited from the workspace, tests, and a scoped `AGENTS.md` when special constraints exist.
- Do not add catch-all crates named `common`, `utils`, or `algorithms`; name packages after one responsibility.
- Update active documentation and commands whenever paths, APIs, deployment configuration, or ownership change.
- Use checked conversions and explicit typed errors; do not add `unwrap`, `expect`, panic-driven control flow, or unsafe code.

## Required checks

Run from the repository root before marking work complete:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

For path changes, also search the full tracked tree for stale paths and verify the Worker build, migration discovery, deployment commands, and CI workflow from the new location.