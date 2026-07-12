# ADR 0013: Plain Worker runtime, OrderBook-rs kernel, and Workers Cache snapshots

- Status: Accepted
- Date: 2026-07-12
- Supersedes: ADR 0002, the active-book portions of ADR 0008, and the Durable Object ownership portions of ADR 0011

## Context

The initial bootstrap assumed one Durable Object per run and a new Bunting-owned deterministic order book. The desired deployment is instead a plain Cloudflare Worker, and `joaquinbejar/OrderBook-rs` already supplies a broad, tested Rust implementation of the matching and book capabilities Bunting needs.

The upstream crate includes price-time matching, limit and market orders, IOC/FOK/post-only and special orders, self-trade prevention, fees, typed risk gates, an operational kill switch, deterministic host-driven expiry, engine sequencing, snapshots with checksums, restore, replay helpers, depth analytics, market impact, enriched snapshots, listeners, and extensive tests.

Reimplementing those capabilities would duplicate mature MIT-licensed work and prolong the project.

## Decision

### Runtime

Bunting runs as a plain Rust Cloudflare Worker. No Durable Object binding is required.

A warm Worker isolate may retain a reconstructed book as a performance optimization. Correctness never assumes that the same isolate receives the next request or that global memory survives.

### Matching kernel

The released crate below is the production kernel dependency:

```toml
orderbook-rs = { version = "=0.10.3", default-features = false }
```

The audited source revision is:

```text
joaquinbejar/OrderBook-rs@575de34260b0fce346372074b6b938df058693a8
```

`crates/orderbook` is a thin adapter and re-export boundary. It must not contain an independent price-level store or matching loop.

Bunting directly uses upstream:

- `add_limit_order_with_result` and the other per-call result APIs;
- market-order execution and trade results;
- cancellation, mass cancel, expiry, and kill-switch operations;
- checksum-protected snapshot packages and restore;
- risk, fee, STP, lifecycle, engine-sequence, and depth/analytics APIs;
- deterministic host clock and replay facilities where applicable.

### Wasm compatibility

The exact dependency graph must pass `cargo check --target wasm32-unknown-unknown`.

If the released crate fails because of a narrow platform issue, the order of preference is:

1. feature configuration;
2. upstream patch and contribution;
3. a minimal Bunting fork preserving the upstream API and tests.

A fork must retain the MIT license, identify every changed file, explain every divergence, and remain updateable from upstream. A Wasm issue is not permission to rewrite the matching engine.

### Workers Cache

The Workers Cache API is mandatory for `OrderBookSnapshotPackage` JSON payloads.

Each entry is immutable and content-addressed by:

```text
(run_id, instrument_id, committed_event_sequence, snapshot_checksum)
```

The response carries `Cache-Control`, `ETag`, and the represented event sequence. Cache reads never make an implicit origin subrequest.

A cache hit restores the upstream package after version and checksum validation. A cache miss falls back to the origin event/snapshot store. A corrupted or incompatible cache entry is discarded and rebuilt.

Workers Cache is not used as a lock, compare-and-swap register, idempotency table, participant balance store, or sole accepted-command journal.

### Command path

The intended request path is:

```text
HTTP/FIX/WebSocket command
  -> authenticate and validate
  -> load newest compatible snapshot from Workers Cache
  -> fall back to origin snapshot/event tail on miss
  -> restore OrderBook-rs
  -> apply Bunting participant/account risk
  -> invoke OrderBook-rs operation
  -> derive canonical Bunting events and ledger projections
  -> commit with expected origin stream version
  -> write immutable snapshot package to Workers Cache
  -> acknowledge and publish
```

The origin implementation must provide optimistic version checking so two Worker requests cannot both commit against the same run version.

### Streaming

A plain Worker may accept WebSockets, but one connection or isolate is not the owner of the market. Stream messages carry the committed event sequence and use snapshot/reset recovery.

OrderBook-rs `engine_seq` is retained as an upstream book-change sequence. Bunting's committed event sequence remains the cross-component persistence cursor.

No stream update is published before the corresponding origin commit succeeds.

## Reuse from the partially implemented branch

The old `feat/deterministic-kernel-vertical-slice` branch contains useful Bunting-owned work:

- checked identifiers and fixed-point values;
- canonical command and event structures;
- participant ledger and reservation projections;
- participant/account risk checks;
- property-test ideas and semantic fixtures.

Its custom `BTreeMap`/arena order book and custom matching loop are obsolete under this ADR and must not be merged.

## Consequences

Positive:

- Bunting immediately gains a much larger matching, order-type, snapshot, replay, risk, and analytics surface;
- upstream fixes and tests remain available through normal dependency upgrades;
- the Worker layer stays focused on orchestration and protocols;
- Workers Cache provides low-latency snapshot distribution without pretending to be transactional storage.

Negative:

- Bunting inherits the upstream dependency graph and must continuously verify Wasm compatibility;
- concurrent native data structures may add code size that a Worker-specific engine would avoid;
- upstream upgrades require semantic and snapshot-compatibility review;
- a separate origin versioning implementation is still required for concurrent plain-Worker requests.

## Validation

- the exact crate builds natively and for `wasm32-unknown-unknown`;
- limit, market, cancel, partial-fill, kill-switch, risk, snapshot, and restore tests call upstream APIs;
- Bunting contains no second production matching loop;
- cache keys are deterministic and immutable;
- cache miss, invalid checksum, and incompatible snapshot paths recover from origin;
- optimistic version tests prove conflicting commands cannot both commit;
- streaming tests prove that only committed sequences are published.
