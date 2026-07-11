# Bunting

Bunting is a deterministic, event-sourced stock-market simulation engine designed for native Rust execution on Cloudflare Workers.

## Goals

- simulate configurable market scenarios with multiple human, API and strategy participants;
- stream books, trades, orders, fills, positions and P&L;
- expose native HTTP/WebSocket APIs, a NautilusTrader adapter and FIX 4.4 compatibility;
- execute user-submitted Python strategies in isolated Dynamic Workers;
- recover and replay runs exactly from versioned scenarios, seeds, snapshots and canonical events.

## Architecture

The authoritative live runtime is one `MarketRun` Durable Object per run. The matching, risk, ledger, clock and scenario logic live in a pure Rust kernel with no Worker, Tokio, filesystem, socket or wall-clock dependencies.

FIX is carried as raw FIX 4.4 messages over a WebSocket subprotocol. An installable native Rust bridge exposes ordinary local FIX/TCP to standard FIX clients.

See:

- [`docs/architecture.md`](docs/architecture.md)
- [`docs/reference-inventory.md`](docs/reference-inventory.md)
- [`docs/adr/`](docs/adr/)
- [`docs/codex-implementation-prompt.md`](docs/codex-implementation-prompt.md)

## Repository status

This bootstrap PR establishes architectural constraints, pinned references, workspace instructions and initial fixed-point/event primitives. Reference projects under `ref/` are unaudited and read-only; no reference implementation is assumed correct or Worker-compatible.

## Initialize references

```bash
git clone --recurse-submodules https://github.com/andrewkoumoudjian/bunting.git
cd bunting
git submodule update --init --recursive
```

## Initial checks

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace --target wasm32-unknown-unknown
```
