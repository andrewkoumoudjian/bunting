# Codex implementation contract

Read `AGENTS.md` and ADR 0013 before changing code.

## Non-negotiable decisions

- The runtime is a plain Rust Cloudflare Worker.
- `orderbook-rs = 0.10.3` is the production matching and order-book kernel.
- `pricelevel = 0.8.4` is pinned for type identity.
- Workers Cache is mandatory for immutable checksum-addressed upstream snapshot packages.
- Do not implement a second order book, price-level FIFO, matching loop, snapshot format, kill switch, STP engine, fee model, depth engine, or market-impact engine.
- Do not introduce a Durable Object binding or architecture assumption.

## Preferred upstream APIs

Use per-call result APIs such as `add_limit_order_with_result`, direct market-order methods, typed `OrderBookError`, `RiskConfig`, kill-switch and mass-cancel methods, host-driven expiry, `create_snapshot_package`, package JSON/checksum validation, restore, replay helpers, depth iterators, metrics, impact, and enriched snapshots.

## Bunting responsibilities

Implement adapters for:

- checked Bunting IDs and units;
- actor/order ownership;
- auth and idempotency;
- origin expected-version transactions;
- canonical event translation;
- participant ledger and cross-book risk;
- Workers Cache keys and recovery;
- HTTP/WebSocket/FIX/NBC/RITC/Nautilus protocols;
- scenario and Dynamic Worker orchestration.

## First implementation target

Complete one limit-order command through:

```text
auth -> expected version -> cache/origin restore -> Bunting risk
     -> OrderBook-rs -> canonical events/ledger -> origin commit
     -> immutable cache put -> response
```

Tests must cover cache hit, miss, corrupt package, duplicate command, version conflict, resting order, crossing trade, rejection, and restart.

## Prohibited work

- restoring the deleted custom `BTreeMap`/arena book;
- copying multithreaded OrderBook-rs examples into the Worker;
- using `current_time_millis` as an unrecorded replay input;
- treating cache or isolate globals as transactional state;
- claiming Wasm compatibility without running the exact target check.
