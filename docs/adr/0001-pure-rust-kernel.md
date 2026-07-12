# ADR 0001: Pure Rust deterministic market kernel

- Status: Accepted
- Date: 2026-07-11

## Context

Bunting must run on Cloudflare Workers while also supporting native tests, replay tools, a local FIX bridge and NautilusTrader clients. The reference implementations mix business logic with native threads, sockets, blocking HTTP, gRPC, filesystem SQLite and wall-clock reads. Those assumptions do not map cleanly to `wasm32-unknown-unknown` or to deterministic replay.

The C++ order manager does contain a valuable idea: process order and fill events sequentially. It currently achieves that with a dedicated dispatch thread and queue. A Durable Object already provides a single stateful coordination point, so reproducing the native thread architecture would add complexity without value.

## Decision

Create a pure Rust kernel composed of small crates:

- fixed-point domain types;
- canonical commands and events;
- order book;
- matching engine;
- risk engine;
- ledger;
- logical clock;
- scenario compiler and agent state machines;
- scoring and replay formats.

The kernel:

- has no Cloudflare dependencies;
- has no Tokio dependency;
- performs no network, filesystem or database I/O;
- does not read wall time;
- does not spawn threads;
- does not use global mutable state;
- receives all time, randomness, configuration and commands explicitly;
- returns canonical events and deterministic state transitions;
- compiles natively and for `wasm32-unknown-unknown`.

Runtime ports are implemented as adapters around traits for persistence, clocks, entropy derivation and transport.

## Consequences

Positive:

- deterministic replay and property testing are practical;
- Worker runtime changes do not infect matching logic;
- the same kernel can run in native benchmarks and local simulations;
- reference behavior can be ported incrementally;
- protocol adapters remain replaceable.

Negative:

- some native libraries cannot be reused directly;
- explicit interfaces and conversion layers add initial work;
- runtime optimizations must respect the pure boundary.

## Rejected alternatives

### Port the C++ service architecture literally

Rejected because threads, mutexes, gRPC, native SQLite and gateway races are platform-specific. Preserve business semantics, not accidental concurrency.

### Build the engine directly inside a Worker handler

Rejected because it makes testing, replay and native client reuse difficult and couples correctness to platform APIs.

### Use Tokio inside all crates

Rejected because the market kernel is not an asynchronous I/O application. Tokio belongs only in native clients and bridges.

## Validation

- `cargo test --workspace` passes natively.
- Worker-compatible crates pass `cargo check --target wasm32-unknown-unknown`.
- identical scenario, seed and command streams produce identical event and state checksums.
- no domain crate depends on `worker`, `tokio`, `std::fs`, socket crates or system time.

## References

- `ref/workers-rs` at `5f2d6c9192377451d43910098738624474196364`
- `ref/quarcc-trading-engine/engine-cpp/include/trading/core/order_manager.h`
- `ref/quarcc-trading-engine/engine-cpp/src/core/order_manager.cpp`
- Cloudflare Rust Workers documentation: https://developers.cloudflare.com/workers/languages/rust/
