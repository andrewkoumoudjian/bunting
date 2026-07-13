# Bunting architecture

## 1. Purpose

Bunting is a stock-market simulation and exchange-testing platform implemented primarily in Rust for a plain Cloudflare Worker. It supports human and automated participants, configurable scenarios, standard protocol adapters, streaming market data, deterministic recovery, and isolated user strategies.

The system is an education, research, and integration environment. It is not a colocated real-money exchange.

## 2. Binding principles

1. **Use OrderBook-rs:** matching and book capabilities come from the released upstream crate rather than a parallel implementation.
2. **Plain Worker:** no Durable Object is required by the current design.
3. **Workers Cache required:** checksum-protected upstream snapshot packages are cached under immutable content-addressed keys.
4. **Origin versioning:** accepted commands and canonical events use an origin store with optimistic expected-version checks.
5. **Warm memory is optional:** an isolate may retain reconstructed books, but recovery cannot depend on isolate affinity.
6. **Bunting owns the exchange boundary:** authentication, participants, canonical events, ledger, scenarios, protocols, persistence orchestration, and streaming recovery remain Bunting concerns.
7. **Fixed-point boundaries:** external prices, quantities, money, limits, and sequences are checked integer values.
8. **Commit before publish:** no acknowledgement, fill, or stream update is public before origin persistence succeeds.
9. **Least privilege:** user Dynamic Workers receive bounded inputs and no direct market-state mutation capability.
10. **Upstream-first maintenance:** prefer dependency upgrades and upstream fixes to internal forks.

## 3. Topology

```text
Browser / SDK / Nautilus adapter
FIX initiator -> local FIX bridge
                  |
             typed tRPC client
                  |
                  v
       Public TypeScript tRPC Worker
       - public auth and schemas
       - bounded query/mutation/subscription routing
                  |
          private Service Binding
                  |
                  v
       Authoritative Rust Cloudflare Worker
       - protected actor identity
       - expected-version command handling
       - OrderBook-rs adapter
       - Bunting risk/ledger/events
       - snapshot and stream responses
          |                 |
          |                 +--> Workers Cache
          |                      immutable OrderBook-rs
          |                      snapshot packages
          |
          +--> Origin event/version store
               accepted commands, canonical events,
               idempotency, run metadata, recovery tail

User strategy source
       -> TypeScript Worker Loader boundary
       -> isolated Python Dynamic Worker
       -> validated action proposal
       -> ordinary Edge Worker command path
```

## 4. Repository ownership

### Reusable packages and composition

- `market-types`: Bunting identifiers and checked fixed-point values.
- `market-events`: commands, event envelopes, rejection codes, correlation, and causation.
- `orderbook`: thin version-pinned adapter around `OrderBook-rs`.
- `ledger`: participant cash, inventory, reservation, position, fee, and P&L projections.
- `risk-engine`: participant/account and cross-instrument controls not supplied by the upstream per-book layer.
- `origin-store`: Worker-independent persistence models and the atomic expected-version contract.
- `command-transaction`: sans-I/O recovery, risk, matching, event, ledger, and commit preparation.
- `quarcc-trading-engine`: legacy `quarcc.v1` compatibility types and service trait, not a matching engine.
- `worker-cache`: immutable Workers Cache key and snapshot operations.
- later crates: scenario clock, scenario engine, agent models, protocol-native, FIX, replay exports, and scoring.
- `bunting-rs`: thin portable composition boundary with curated stable re-exports and product metadata.

### Worker

- `apps/edge-api`: the current Rust Worker entrypoint; its command handlers become the private authoritative service behind the public tRPC Worker.
- planned `apps/trpc-api`: the plain TypeScript tRPC Worker defined by ADR 0015.
- planned `clients/typescript-sdk`: the router-derived public client used directly and by protocol adapters.
- `clients/fix-bridge`: the local FIX/TCP compatibility client; it owns participant-side FIX sessions and calls tRPC.

There is no `market-run-do` runtime in the accepted architecture. Historical directories or instructions referring to it are superseded by ADR 0013 and should be removed as implementation proceeds.

## 5. OrderBook-rs boundary

The production dependency is exactly:

```toml
orderbook-rs = { version = "=0.10.3", default-features = false }
pricelevel = "=0.8.4"
```

The adopted upstream source revision is `575de34260b0fce346372074b6b938df058693a8`.

### Upstream owns

- limit, market, IOC, FOK, post-only, iceberg, reserve, pegged, trailing-stop, and market-to-limit semantics;
- price-time matching and partial-fill priority;
- price levels and direct order lookup;
- trade and price-level change results;
- mass cancel and cancellation ordering;
- self-trade prevention and fee schedules;
- book-level risk configuration and typed reject reasons;
- operational kill switch and halt-and-drain behavior;
- host-driven GTD/DAY expiry sweeps;
- order lifecycle tracking;
- engine sequence;
- checksum-protected snapshot package and restore;
- in-memory journal/replay helpers;
- L1/L2 depth, iterators, metrics, market impact, and enriched snapshots.

### Bunting owns

- run, instrument, participant, command, event, and correlation identities;
- authentication, authorization, tenancy, and protocol limits;
- idempotency and origin expected-version checks;
- cross-book and participant-level cash/inventory/position risk;
- canonical events and participant ledger projections;
- scenario scheduling, random streams, scoring, and administration;
- the private authoritative service, Cache API policy, committed stream content, and recovery behavior;
- Dynamic Worker strategy isolation;
- NBC mappings and the internal service contract; public tRPC, FIX, RITC, and Nautilus mappings stay outside the market engine.

## 6. Public API boundary

ADR 0015 makes `bunting.v1` tRPC the only public application API. The TypeScript Worker owns public authentication, runtime schemas, bounded transport and router-derived client types, but it owns no market state. It delegates mutations through a private service binding and returns only committed Rust transaction results.

The current Rust REST routes are provisional migration handlers for that private binding, not a supported public REST API. Public streams are tRPC subscriptions retaining the committed-sequence, reset, coalescing and backpressure rules in ADR 0011.

FIX compatibility is implemented by a local client bridge that terminates FIX/TCP and maps application messages through the typed tRPC client. FIX session sequences remain participant-side state and never replace Bunting event sequences.

## 7. Command transaction

For one mutating command:

1. authenticate the actor;
2. parse exact integer units and validate protocol limits;
3. verify idempotency and expected run version;
4. load the newest compatible upstream snapshot from Workers Cache;
5. on miss or validation failure, load an origin snapshot and event tail;
6. restore `OrderBook-rs` and Bunting projections;
7. run Bunting participant/account risk;
8. invoke one upstream operation;
9. translate upstream orders, rejects, trades, and level changes into canonical Bunting events;
10. project ledger and reservation changes against a candidate state;
11. atomically commit the event batch and next origin version;
12. create a new `OrderBookSnapshotPackage`;
13. asynchronously or inline write the immutable snapshot to Workers Cache;
14. acknowledge and publish only the committed result.

A cache write failure does not roll back an origin commit. The next request rebuilds and repopulates the cache.

## 8. Concurrency without a Durable Object

A plain Worker does not provide request affinity. The origin store therefore enforces optimistic concurrency:

```text
commit(command, expected_version, event_batch, snapshot_metadata)
```

The commit succeeds only when the current run version equals `expected_version`. A loser reloads the new state and returns or retries according to a bounded policy.

Workers Cache does not provide this compare-and-swap function and cannot be used as a lock.

The D1 adapter stores `u128` identifiers and `u64` sequences as decimal `TEXT`, avoiding JavaScript's 53-bit integer boundary. One D1 `batch()` inserts a command guard only when the run version matches, conditionally appends every event, result, and snapshot row, then conditionally updates the complete recovery projection. A zero-row final update is a sequence conflict; any statement error rolls back the batch.

The initial slice writes an authoritative upstream package and complete private projection after every command. Its event tail is empty without making Workers Cache authoritative; canonical events are still appended for audit and later bounded-tail replay.

## 9. Snapshot and Cache API

The cache key is:

```text
https://cache.bunting.invalid/v1/orderbooks/
  {run_id}/{instrument_id}/{event_sequence}/{snapshot_checksum}
```

Properties:

- immutable key;
- JSON `OrderBookSnapshotPackage` body;
- upstream package validation on every restore;
- checksum as ETag;
- represented event sequence in a response header;
- explicit `s-maxage` and `immutable` Cache-Control;
- no private participant data in public book cache entries;
- bounded depth and payload size;
- ordinary recovery on miss, eviction, or POP-local absence.

Private ledgers, credentials, idempotency results, and accepted-command records are never cached as public responses.

## 10. Streaming

The public Worker exposes versioned tRPC subscriptions first. Binary transport may later use IronSBE after Wasm and compatibility review, but it remains behind the same client contract rather than becoming a second public authority path.

Book streams are generated from committed upstream snapshots and absolute resulting level quantities. Each message includes:

- protocol version;
- run and instrument IDs;
- committed Bunting event sequence;
- upstream engine sequence where relevant;
- channel and message type;
- payload and optional visible-projection checksum.

A reconnect presents its last committed sequence. The Worker either supplies an available event tail or emits `stream.reset` with a current snapshot. No resume contract depends on one isolate's memory.

Public book state may coalesce. Trades and private execution/account records cannot be silently discarded. Slow consumers are disconnected with a recovery cursor.

## 11. Numerics and identity

Bunting protocol and ledger types include:

```text
PriceTicks(i64)
QuantityLots(i64)
MoneyMinor(i128)
LogicalTimeNs(u64)
EventSequence(u64)
RunId(u128)
InstrumentId(u128)
ParticipantId(u128)
OrderId(u128)
```

The OrderBook-rs adapter performs checked conversion to the upstream `u128` price, `u64` quantity, and `Id` types. The first adapter supports sequential IDs representable as `u64`; broader ID mapping must be explicit and collision tested.

Floating analytics from upstream metrics are derived only. They never determine canonical money, quantity, priority, or ledger equality.

## 12. Risk and ledger

OrderBook-rs book-level risk is enabled where it matches the requirement: open-order counts, notional, price bands, kill switch, STP, fees, and validation.

Bunting retains separate participant-level controls for:

- enabled/disabled actor state;
- available cash and inventory;
- cross-instrument position and exposure;
- run-wide and role-specific limits;
- reservation policy;
- administrative halts and competition rules.

The ledger projects canonical trades and cancellations. It never infers fills from snapshots.

## 13. Strategies and scenarios

Dynamic Worker strategy execution remains asynchronous and capability-limited. A strategy proposes commands; it never receives a direct reference to the upstream book or cache.

Built-in agents use explicit scenario time and named seeded random streams. Host-driven upstream clock/expiry APIs receive recorded logical or scheduled cutoffs.

## 14. Reference policy

- `OrderBook-rs` and `PriceLevel` are production dependencies.
- `workers-rs` is the Worker/Cache production dependency.
- Liquibook and exchange-core remain independent differential oracles.
- Option-Chain-OrderBook is a future options-layer candidate.
- market-maker-rs is a pure-formula/test donor, not a wholesale dependency.
- IronSBE is a later binary-protocol candidate.
- fauxchange currently has no implementation to copy.

Any copied MIT source retains its notice, exact path, commit, and divergence record. Prefer upstream APIs and normal dependencies over copied source.

## 15. Validation gates

Every change to the kernel boundary runs:

- native formatting, Clippy, and unit tests;
- `wasm32-unknown-unknown` compilation;
- dependency-tree assertion for the pinned upstream version;
- snapshot package checksum and restore tests;
- limit, market, cancel, partial-fill, risk, kill-switch, and expiry tests;
- cache hit/miss/corruption tests;
- expected-version conflict tests;
- no-second-matcher policy checks;
- size and cold-start measurement before deployment.
