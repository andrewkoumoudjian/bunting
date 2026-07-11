# Bunting agent instructions

## Mission
Build a deterministic, event-sourced market simulator whose authoritative runtime is a Rust Cloudflare Worker and a per-run Durable Object.

## Global constraints
- Treat `ref/` as read-only evidence. Never import reference code blindly.
- Production Rust must be safe Rust unless an ADR approves a narrowly reviewed exception.
- Domain crates must not depend on Cloudflare, Tokio, filesystems, threads, sockets, or wall-clock APIs.
- Worker crates must compile for `wasm32-unknown-unknown`.
- Use integer fixed-point values for prices, quantities, money, logical time, and sequences.
- All state transitions must be deterministic for the same scenario version, seed, and command stream.
- Protocol adapters translate into canonical commands and events; protocol logic never enters matching logic.
- Durable Object SQLite is authoritative for active-run events, snapshots, idempotency, scheduled work, and FIX session state.
- D1, KV, Cache, Queues, R2, and Analytics Engine are never authoritative for live orders, positions, or balances.
- Keep buffers bounded and make backpressure explicit.
- Preserve upstream licenses and exact commit SHAs for copied or adapted code.

## Required checks
Run formatting, clippy, unit tests, property tests, dependency/license checks, and `cargo check --target wasm32-unknown-unknown` for Worker-compatible crates before marking work complete.
