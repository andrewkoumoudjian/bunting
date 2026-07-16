# Bunting architecture

## 1. Purpose

Bunting is a stock-market simulation and exchange-testing platform implemented primarily in Rust for a plain Cloudflare Worker. It supports human and automated participants, configurable scenarios, standard protocol adapters, streaming market data, deterministic recovery, and isolated user strategies.

The system is an education, research, and integration environment. It is not a colocated real-money exchange.

## 2. Binding principles

1. **Use OrderBook-rs:** matching and book capabilities come from the released upstream crate rather than a parallel implementation.
2. **Plain Worker authority:** Durable Objects may own outbound FIX session state, but no Durable Object owns market authority.
3. **Workers Cache required:** checksum-protected upstream snapshot packages are cached under immutable content-addressed keys.
4. **Origin versioning:** accepted commands and canonical events use an origin store with optimistic expected-version checks.
5. **Warm memory is optional:** an isolate may retain reconstructed books, but recovery cannot depend on isolate affinity.
6. **Bunting owns the exchange boundary:** authentication, participants, canonical events, ledger, scenarios, protocols, persistence orchestration, and streaming recovery remain Bunting concerns.
7. **Fixed-point boundaries:** external prices, quantities, money, limits, and sequences are checked integer values.
8. **Commit before publish:** no acknowledgement, fill, or stream update is public before origin persistence succeeds.
9. **Least privilege:** user Dynamic Workers receive bounded inputs and no direct market-state mutation capability.
10. **Upstream-first maintenance:** prefer dependency upgrades and upstream fixes to internal forks.
11. **One central production engine package:** `bunting-engine` directly owns its private OrderBook-rs integration and composes matching with scenario, NBC compatibility, RIT-derived behavior, ledger, risk, recovery and publication; profiles configure that engine rather than selecting alternate venue kernels.

## 3. Topology

```text
Rust/WASM browser client -- bounded fetch/stream -->
       Native Rust Cloudflare Worker
       - browser-compatible fetch/stream dispatch
       - outbound FIX/TCP session Durable Objects
       - direct in-process Rust application calls
       - auth, schemas and protocol bounds
       - expected-version command handling
       - unified bunting-engine
         - private OrderBook-rs matching adapter
       - scenario and NBC-compatibility profiles
       - Bunting risk/ledger/events
       - snapshot and stream responses
          |                 |
          |                 +--> optional Rust RunStreamCoordinator DO
          |                      committed fan-out only after ADR 0016 gate
          |                 |
          |                 +--> Workers Cache
          |                      immutable OrderBook-rs
          |                      snapshot packages
          |
          +--> Origin event/version store
               accepted commands, canonical events,
               idempotency, run metadata, recovery tail

External FIX acceptor <-- bidirectional session -- outbound TCP initiator

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
- `bunting-engine`: the implemented central production venue package. It owns bounded multi-listing run state, the authoritative submit-limit/cancel transition, deterministic scenario and engine versions, complete snapshot envelopes and the private version-pinned OrderBook-rs adapter.
- `ledger`: participant cash, inventory, reservation, position, fee, and P&L projections.
- `risk-engine`: participant/account and cross-instrument controls not supplied by the upstream per-book layer.
- `origin-store`: Worker-independent persistence models and the atomic expected-version contract.
- `command-transaction`: sans-I/O recovery, risk, matching, event, ledger, and commit preparation.
- Persistence and transport remain outside `bunting-engine`.
- `bunting-api-contract` and `browser-wire`: the Rust-owned procedure schema and bounded browser JSON transport.
- `bunting-engine::compatibility::nbc`: NBC configuration, scheduler, synchronization, and provenance; the old matcher remains only under `tests/oracles`.
- `quarcc-execution-engine`, `quarcc-bunting-adapter`, and `quarcc-execution-wasm`: participant execution and browser bindings.
- `bunting-agents`: policies composed with mandatory QUARCC execution.
- `bunting-runtime`: deterministic sans-I/O wake scheduling, authenticated
  built-in participant identities, bounded action cascades, and portable
  scheduler snapshots. Hosts remain responsible for the single authoritative
  application writer and persistence.
- `simfix-wire`, `simfix-session`, and `simfix-mapping`: transport-neutral FIX protocol layers.
- `worker-cache`: immutable Workers Cache key and snapshot operations.
- later engine modules: scenario clock, agents, products, news, tenders, assets, scoring and complete recovery. Extract a focused package only when a second real consumer proves a reusable non-authoritative boundary; FIX and native report/export tooling remain outside the engine.
- `bunting-rs`: thin portable composition boundary with curated stable re-exports and product metadata.

### Worker

- `apps/bunting-worker`: the native Rust Worker with browser dispatch and outbound FIX session objects.
- `apps/bunting-tui`: a native-only, Longbridge-derived Ratatui operator workstation that initiates FIX/TCP sessions. Its optional loopback acceptor invokes `bunting-engine` in-process for local testing and does not change the Worker's outbound-only TCP boundary.
- planned generated TypeScript SDK: declarations and official-client wrapper derived from the Rust contract, never a second hand-written contract.
- `FixSessionObject`: owns outbound FIX/TCP socket and recovery state, never market authority.

There is no authoritative `market-run-do` runtime. ADR 0016 permits an optional Rust `RunStreamCoordinator` Durable Object only after its stream-coordination gate; it never owns commands, matching or origin truth.

## 5. OrderBook-rs boundary

The production dependency, owned directly by `packages/bunting-engine` after the ADR 0019 migration, is exactly:

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
- the browser-compatible fetch/stream entrypoint, Cache API policy, committed stream content, and recovery behavior;
- Dynamic Worker strategy isolation;
- NBC translation/integration, RIT-derived venue behavior and the Rust browser contract; FIX, RITC, QUARCC and Nautilus mappings stay outside the market engine.

## 6. Public API boundary

ADR 0020 supersedes the universal RPC boundary. One native Rust Worker parses a bounded browser JSON contract and invokes the in-process Rust command transaction; internal Worker composition never crosses a protocol hop.

The Rust contract generates client schemas. The current browser envelope retains a development-only differential record against pinned tRPC fixtures, but tRPC is no longer an architecture or runtime dependency. Public streams retain the committed-sequence, reset, coalescing and backpressure rules in ADR 0011. A Rust Durable Object may coordinate committed fan-out only after the explicit ADR 0016 gate.

FIX compatibility is implemented by a Worker Durable Object that initiates outbound TCP and owns the bidirectional FIX 4.4 session. It never accepts inbound raw TCP, and FIX sequences never replace Bunting event sequences.

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

The public Worker exposes versioned browser-compatible streams. Binary transport may later use IronSBE after Wasm and compatibility review, but it remains behind the same client contract rather than becoming a second public authority path.

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
