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

## Workspace

The current workspace contains:

- `market-types`: checked Bunting identifiers and fixed-point values;
- `market-events`: protocol-neutral commands and canonical event envelopes;
- `orderbook`: a thin version-pinned adapter around `OrderBook-rs`;
- `ledger`: participant cash, position, and reservation projections;
- `risk-engine`: Bunting participant/account limits not covered by the upstream book;
- `worker-cache`: immutable Workers Cache snapshot adapter;
- `workers/edge-api`: the plain Rust Cloudflare Worker entrypoint.

## Checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --target wasm32-unknown-unknown
```
