# Codex implementation contract

Read `AGENTS.md`, ADR 0013, ADR 0014, ADR 0017, ADR 0018, ADR 0019, ADR 0020 and the nearest scoped instructions before changing code. Active sequencing lives in [`plans/corrected-bunting-implementation-plan.md`](plans/corrected-bunting-implementation-plan.md).

## Repository paths

- Reusable first-party crates live under `packages/`.
- The curated portable composition crate is `bunting-rs/`.
- The current Rust Worker, Wrangler config and D1 migrations live under `apps/bunting-worker/`.
- Do not create Cargo-less future scaffolds. A package or module appears only with its first compiling implementation and tests.
- Generated release assembly belongs under ignored `out/`; never commit Worker `build/`, Wasm or release artifacts.

## Non-negotiable decisions

- One native Rust Cloudflare Worker owns bounded browser dispatch and outbound FIX sessions; market authority remains in the in-process engine/application transaction.
- `orderbook-rs = 0.10.3` is the production matching and order-book kernel and becomes a direct private dependency of the central `bunting-engine` package under ADR 0019.
- `pricelevel = 0.8.4` is pinned for type identity.
- Workers Cache is mandatory for immutable checksum-addressed upstream snapshot packages.
- Do not implement a second order book, price-level FIFO, production matching loop, snapshot format, kill switch, STP engine, fee model, depth engine, or market-impact engine. The current NBC matcher is a transitional differential oracle, not production authority.
- ADR 0020 authorizes FIX-session Durable Objects for outbound TCP and recovery only. A stream coordinator remains conditional on the ADR 0016 evidence gate.

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
- the native Rust browser contract, authorized NBC translation and public mappings; FIX, RITC and Nautilus remain protocol/client concerns outside market authority;
- scenario and Dynamic Worker orchestration.

## Historical implementation target

Create `packages/bunting-engine`, move the tested first-party OrderBook-rs adapter into its private matching module, migrate production callers, and establish the bounded multi-listing run aggregate described by ADR 0019. Preserve the existing committed command path:

```text
auth -> expected version -> cache/origin restore -> Bunting risk
     -> OrderBook-rs -> canonical events/ledger -> origin commit
     -> immutable cache put -> response
```

Tests must retain cache hit, miss, corrupt package, duplicate command, version conflict, resting order, crossing trade, rejection and restart coverage, then add multi-listing isolation, atomic candidate failure, deterministic serialization and full engine state-hash recovery.

## Prohibited work

- restoring the deleted custom `BTreeMap`/arena book;
- copying multithreaded OrderBook-rs examples into the Worker;
- using `current_time_millis` as an unrecorded replay input;
- treating cache or isolate globals as transactional state;
- claiming Wasm compatibility without running the exact target check.
