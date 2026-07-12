# ADR 0001: Pure Rust deterministic market kernel

- Status: Superseded in part by ADR 0013
- Date: 2026-07-11
- Superseded: 2026-07-12

## Historical decision

The original decision required a Bunting-owned pure kernel with no Tokio or runtime dependencies and listed separate Bunting order-book and matching-engine crates.

## Current interpretation

The general separation remains useful for Bunting-owned authentication, canonical events, ledger, scenarios, and protocol adapters. The order-book and matching portion is superseded.

`OrderBook-rs` `0.10.3` is now the production kernel dependency. Its transitive runtime/concurrency dependencies are accepted subject to native and Wasm checks. Bunting does not recreate its matching or price-level implementation.

See ADR 0013 for the binding architecture.
