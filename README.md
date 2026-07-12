# Bunting

Bunting is a Rust market-simulation and exchange-testing platform designed to run in a plain Cloudflare Worker.

## Engine model

Bunting distinguishes venue-side market engines from participant-side execution engines.

- The current default market path uses released [`OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs) `0.10.3` for matching and order-book behavior.
- Bunting adds venue identity, canonical events, participant ledger/risk, origin persistence, recovery, protocols and Worker routes around that kernel.
- NBC is a separate venue-side market-engine port target. It is not merely scenario data. The current reference snapshot proves the packaged simulator/configuration/scenarios and observable client protocol, but not the missing Java internals.
- The QUARCC trading engine is an optional external participant execution/OMS port. It consumes market data, manages/routs orders, applies participant controls and projects positions; it does not own venue matching or Bunting origin state.

See:

- [`docs/adr/0013-worker-orderbook-rs-kernel.md`](docs/adr/0013-worker-orderbook-rs-kernel.md)
- [`docs/adr/0014-market-and-execution-engine-boundaries.md`](docs/adr/0014-market-and-execution-engine-boundaries.md)
- [`docs/reference-functionality-audit.md`](docs/reference-functionality-audit.md)
- [`docs/reference-adoption.md`](docs/reference-adoption.md)
- [`docs/architecture.md`](docs/architecture.md)

## Current architecture

- `OrderBook-rs` snapshots are checksum-protected and stored through the Cloudflare Workers Cache API under immutable, content-addressed keys.
- The origin event/version store remains authoritative for accepted commands, canonical events, idempotency, projections and optimistic concurrency.
- Cache misses or evictions are normal recovery events.
- No Durable Object is required by the accepted architecture.
- User strategy outputs enter through the normal authenticated command/risk/persistence path.

## Reference policy

`ref/` is read-only evidence. It contains 25 Git submodules and three checked-in source/asset trees. It is never a production path dependency.

`vendor/` currently contains no implementation. It is reserved for explicitly approved copied/patched third-party source with licenses, notices, upstream metadata and patch records.

Do not classify a reference by its name. The source-backed inventory is in [`docs/reference-functionality-audit.md`](docs/reference-functionality-audit.md).

## Repository organization

The current workspace is still rooted at the repository `Cargo.toml`, with implemented libraries under `crates/` and the Worker under `workers/edge-api`.

The next focused pull request is mechanical:

- move reusable first-party Rust crates to `packages/` without renaming them;
- add a thin `bunting-rs/` composition package;
- move `workers/edge-api` to `apps/edge-api`;
- keep one root Cargo workspace and lockfile;
- assemble generated release bundles under ignored `out/` paths;
- do not implement NBC, expand QUARCC, select a FIX stack or fork OrderBook-rs in the move.

Read the complete move map and Codex execution contract in [`docs/repository-reorganization.md`](docs/repository-reorganization.md).

## Current workspace

- `market-types`: checked Bunting identifiers and fixed-point values;
- `market-events`: protocol-neutral commands and canonical event envelopes;
- `orderbook`: thin version-pinned adapter around `OrderBook-rs`;
- `ledger`: participant cash, position and reservation projections;
- `risk-engine`: participant/account controls not supplied by the upstream book;
- `origin-store`: authoritative projections, idempotency, expected-version commits and recovery metadata;
- `command-transaction`: recovery, risk, matching, accounting and commit orchestration;
- `quarcc-trading-engine`: current WASM-safe `quarcc.v1` compatibility-contract seed, not the complete execution engine;
- `worker-cache`: immutable Workers Cache snapshot adapter;
- `workers/edge-api`: current plain Rust Cloudflare Worker entrypoint.

## Initial command API

The current Worker exposes authenticated, bounded JSON routes for limit GTC submission and cancellation:

```text
POST /v1/runs/:run_id/instruments/:instrument_id/orders
POST /v1/runs/:run_id/instruments/:instrument_id/orders/:order_id/cancel
```

Send `Authorization: Bearer <token>` and `X-Bunting-Participant-Id: <u128>`. Exact identifiers, expected sequence and logical time are JSON strings; price and quantity are checked integer units.

Before the mechanical move, deployment commands still use `workers/edge-api/wrangler.toml`:

```bash
npx wrangler d1 create bunting-origin
npx wrangler d1 migrations apply bunting-origin --config workers/edge-api/wrangler.toml --remote
npx wrangler secret put BUNTING_API_TOKEN --config workers/edge-api/wrangler.toml
```

Scenario/orchestration code provisions runs before order entry. The command endpoint returns `unknown_run` instead of creating authoritative state implicitly.

## Checks

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```
