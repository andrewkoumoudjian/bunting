# Bunting agent instructions

## Mission
Build a stock-market simulation and exchange-testing platform whose runtime is a plain Rust Cloudflare Worker and whose matching core is the released `OrderBook-rs` crate.

## Binding decisions
- `orderbook-rs = 0.10.3` is the production matching and order-book dependency.
- Do not create a second Bunting-owned limit-order book, price-level queue, matching loop, snapshot format, replay engine, kill switch, or market-depth analytics layer when the upstream API already provides it.
- A minimal attributed fork is permitted only when an upstream Wasm incompatibility cannot be fixed through features or an upstream contribution. Preserve the MIT license, API shape, tests, and an exact divergence log.
- The deployment target is a plain Cloudflare Worker. Do not introduce a Durable Object requirement without a new user-approved ADR.
- The Cloudflare Workers Cache API is mandatory for immutable, checksum-addressed `OrderBook-rs` snapshot packages.
- Worker global memory may cache reconstructed books during one warm isolate lifetime, but it is never the only recoverable copy.
- Canonical accepted-command history and optimistic stream versions remain in an origin store. Workers Cache is a required acceleration and distribution layer, not a transaction coordinator.

## Bunting-owned responsibilities
Bunting owns authentication, authorization, run and participant identifiers, canonical event envelopes, idempotency, participant cash and position accounting, scenario scheduling, protocol translation, Worker routes, persistence orchestration, streaming recovery, and Dynamic Worker strategy isolation.

## Upstream responsibilities
Use `OrderBook-rs` directly for order types, price levels, price-time matching, trade generation, snapshots and restore, engine sequencing, market depth, metrics, market-impact simulation, self-trade prevention, fees, risk hooks, order lifecycle tracking, mass cancel, expiry sweeps, and the operational kill switch.

## Source and dependency rules
- `ref/` is read-only evidence and provenance; production manifests use released packages.
- Preserve upstream licenses and exact source paths for any copied example or test.
- Prefer calling a stable upstream API over copying its implementation.
- Copy MIT-licensed examples or tests only when adaptation creates a Bunting-specific fixture or boundary test, and record the upstream commit and path.
- Worker-bound packages must compile for `wasm32-unknown-unknown`.
- Keep fixed-point and checked arithmetic at Bunting protocol and ledger boundaries.
- Keep buffers bounded and recovery explicit.

## Required checks
Run formatting, Clippy, unit tests, dependency-policy checks, and `cargo check --target wasm32-unknown-unknown` before marking work complete.
