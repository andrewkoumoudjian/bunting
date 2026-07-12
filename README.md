# Bunting

Bunting is a Rust stock-market simulation and exchange-testing platform designed to run in a plain Cloudflare Worker.

## Current architecture

- [`OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs) `0.10.3` is the production matching and order-book kernel.
- Bunting wraps the upstream engine with participant identity, canonical events, ledger projections, scenario logic, persistence, protocols, and Worker routes.
- Checksum-protected `OrderBook-rs` snapshot packages are stored in the Cloudflare Workers Cache API under immutable, content-addressed keys.
- An origin event/version store remains responsible for accepted-command durability and concurrency control; cache misses are normal recovery events.
- No Durable Object is required by the current architecture.
- User Python executes in isolated Dynamic Workers and returns proposed actions through the normal Bunting validation path.

## Reuse policy

Bunting does not reimplement upstream matching, FIFO queues, special-order behavior, snapshots, depth analytics, market-impact analysis, kill-switch behavior, or replay helpers. The project uses the upstream APIs and adapts its MIT-licensed examples and tests only where Bunting-specific protocol or recovery fixtures are needed.

See:

- [`docs/adr/0013-worker-orderbook-rs-kernel.md`](docs/adr/0013-worker-orderbook-rs-kernel.md)
- [`docs/orderbook-rs-example-adoption.md`](docs/orderbook-rs-example-adoption.md)
- [`docs/joaquin-repository-audit.md`](docs/joaquin-repository-audit.md)
- [`docs/core-implementation-questions.md`](docs/core-implementation-questions.md)
- [`docs/architecture.md`](docs/architecture.md)
- [`docs/reference-adoption.md`](docs/reference-adoption.md)

## Repository organization

The current workspace remains rooted at the repository `Cargo.toml`, with internal libraries under `crates/` and the Worker currently under `workers/edge-api`.

The next focused pull request will preserve the root Cargo workspace, add `bunting-rs/` as a small public facade package, move the deployable Worker to `apps/edge-api`, reserve `packages/` for independently distributed SDKs, and keep generated release bundles under ignored `dist/` paths and GitHub Releases.

Read the complete move map, branch disposition, acceptance criteria, and Codex execution contract in [`docs/repository-reorganization.md`](docs/repository-reorganization.md).

## Workspace

The current workspace contains:

- `market-types`: checked Bunting identifiers and fixed-point values;
- `market-events`: protocol-neutral commands and canonical event envelopes;
- `orderbook`: a thin version-pinned adapter around `OrderBook-rs`;
- `ledger`: participant cash, position, and reservation projections;
- `risk-engine`: Bunting participant/account limits not covered by the upstream book;
- `origin-store`: authoritative projections, idempotency, expected-version commits, and recovery metadata;
- `command-transaction`: recovery, risk, matching, accounting, and commit orchestration;
- `quarcc-trading-engine`: a WASM-safe compatibility contract for the legacy `quarcc.v1` service surface;
- `worker-cache`: immutable Workers Cache snapshot adapter;
- `workers/edge-api`: the plain Rust Cloudflare Worker entrypoint.

## Initial command API

The Worker exposes authenticated, bounded JSON routes for limit GTC submission and cancellation:

```text
POST /v1/runs/:run_id/instruments/:instrument_id/orders
POST /v1/runs/:run_id/instruments/:instrument_id/orders/:order_id/cancel
```

Send `Authorization: Bearer <token>` and `X-Bunting-Participant-Id: <u128>`. Exact identifiers, expected sequence, and logical time are JSON strings; price and quantity are checked integer units.

Create the D1 database, replace `REPLACE_WITH_D1_DATABASE_ID` in `workers/edge-api/wrangler.toml`, apply the migration, and install the authentication secret:

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
cargo check --locked --workspace --target wasm32-unknown-unknown
```
