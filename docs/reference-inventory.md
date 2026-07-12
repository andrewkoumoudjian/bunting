# Reference implementation inventory

This inventory records why each source exists under `ref/`. Reference code is evidence, not production code. A submodule pin does not authorize copying: every extracted file requires a license check, attribution, a port note and equivalence tests.

Operational dependency, vendoring and borrowing decisions are normative in `docs/reference-adoption.md`. Detailed target mapping for the imported implementations is in `docs/ports/`.

## Selection criteria

A repository is admitted to `ref/` only when it contributes at least one of:

1. mature exchange or protocol semantics that can serve as a behavioral oracle;
2. implementation patterns directly relevant to a Bunting sprint;
3. substantial automated tests, replay fixtures or conformance coverage;
4. a permissive or otherwise understood license compatible with its intended use;
5. a clear boundary that lets Bunting borrow behavior without importing an unsuitable runtime.

Popularity and performance claims are not proof of correctness.

## Reference confidence classes

- **A — selective donor:** small reviewed files or functions may be adapted after license and platform review.
- **B — behavioral oracle:** derive fixtures and invariants; normally do not copy its runtime architecture.
- **C — concept/contract source:** use for design comparison or external integration behavior.
- **D — data/provenance only:** preserve values and hashes without copying implementation.
- **Blocked:** no copying until ownership/license is resolved.

## Pinned external repositories

| Local path | Upstream and pinned commit | Verified license posture | Class | Intended use |
|---|---|---|---:|---|
| `ref/workers-rs` | `cloudflare/workers-rs@5f2d6c9192377451d43910098738624474196364` | Apache-2.0 for `worker` crate | A/C | official Worker, Durable Object, SQLite, WebSocket, queue, RPC and build APIs |
| `ref/nbc-hft-simulation` | `carterj-c/NBC_HFT_Simulation@35b8050546679547dc737198ea13aa0ec8ed7db8` | no root license found | C/Blocked | legacy registration, WebSocket, order/fill and `DONE` compatibility fixtures |
| `ref/ironfix` | `joaquinbejar/IronFix@6ac17a37f7c4efbdfe97a06f428809733d88b66b` | MIT | A | candidate pure FIX core/dictionary/tag-value codec only |
| `ref/fixer` | `fixer-rs/fixer@c1c27c3287d6f275a9c33122cc2af063de7c5a08` | Apache-2.0 | B | native FIX interoperability and session behavior oracle |
| `ref/ferrumfix` | `ferrumfix/ferrumfix@ca2bbe4c6461108646f35f7cc9245bf1848ec368` | MIT OR Apache-2.0 code; separate terms for bundled FIX material | C | FIX layer separation and validation comparison |
| `ref/nautilus-trader` | `nautechsystems/nautilus_trader@c28b1335c95abbf1bef2385def9a75a1b3862f76` | LGPL-3.0 | C | adapter contract, normalized mappings, reconciliation and test organization |
| `ref/wirefilter` | `cloudflare/wirefilter@61936e5f38523df3f80880bbc662e490b52e7f86` | MIT | A/C | optional scenario/admin predicate engine after Wasm and security spike |
| `ref/orderbook-rs` | `joaquinbejar/OrderBook-rs@575de34260b0fce346372074b6b938df058693a8` | MIT | A/B | deterministic traversal lessons, matching, snapshots, replay and test cases |
| `ref/liquibook` | `enewhuis/liquibook@2427613b32f1667abae68a01df6af9ba8270f8e7` | OCI BSD-style; notice required | A/B | independent matching/lifecycle/depth oracle |
| `ref/exchange-core` | `exchange-core/exchange-core@2f8548749839e9095c8dc597e4b61521d259fa5d` | Apache-2.0 | B | integer risk/accounting, journaling, snapshots, state hashes and integrity tests |
| `ref/cqrs` | `serverlesstechnology/cqrs@b13692ce3db62b3b7fea19dddeec90a9d8af3180` | Apache-2.0 | A/C | aggregate/event/store/replay and projection boundaries |
| `ref/nexosim` | `asynchronics/nexosim@42eb361c9c553e50b763524cf9087bb64f31af6c` | MIT OR Apache-2.0 | B/C | typed discrete-event scheduling, monotonic time and save/restore concepts |
| `ref/abides` | `abides-sim/abides@c4bf157678928934417aba6073eb0651aeaf6d15` | BSD-3-Clause | B/C | exchange-agent boundary, seeded agents, latency scheduling and scenario composition |
| `ref/quickfixj` | `quickfix-j/quickfixj@73e45dbe487be54d7c2badec2a846a45ef116ce2` | QuickFIX Software License 1.0 | B | independent FIX sequence/resend/reset conformance oracle |
| `ref/market-maker-rs` | `joaquinbejar/market-maker-rs@36899f3e910997400bc95c3a8f3606776c002fbe` | MIT | A/B | pure market-making formula, risk/execution boundary and test donor |

The `workers-rs` submodule already contains official `examples/`; it is not duplicated separately.

## Current adoption summary

- **Approved production dependency:** released Cloudflare `worker` packages in Worker-only crates.
- **Dependency spikes:** minimal IronFix codec crates and `wirefilter-engine`.
- **Selective source candidates:** narrow IronFix codec code, pure `market-maker-rs` formulas/tests and deterministic OrderBook-rs test/helper material.
- **External integration contract:** NautilusTrader.
- **Oracles:** Liquibook, OrderBook-rs, exchange-core, Fixer, FerrumFIX, QuickFIX/J, ABIDES and NeXosim.
- **Data/compatibility sources:** NBC assets and client.
- **No whole-reference vendoring is approved.**

See `docs/reference-adoption.md` for required gates and per-source prohibitions.

## High-value paths

### Cloudflare runtime

- `ref/workers-rs/worker/src/durable.rs`
- `ref/workers-rs/worker/src/websocket.rs`
- `ref/workers-rs/test/src/durable.rs`
- `ref/workers-rs/test/src/sql_counter.rs`
- `ref/workers-rs/test/src/sql_iterator.rs`

### Matching, risk and accounting

- `ref/orderbook-rs/src/orderbook/book.rs`
- `ref/orderbook-rs/src/orderbook/matching.rs`
- `ref/orderbook-rs/src/orderbook/snapshot.rs`
- `ref/orderbook-rs/src/orderbook/sequencer/replay.rs`
- `ref/orderbook-rs/src/orderbook/clock.rs`
- `ref/liquibook/src/book/order_book.h`
- `ref/liquibook/src/book/order_tracker.h`
- `ref/liquibook/src/book/depth_order_book.h`
- `ref/exchange-core/src/main/java/exchange/core2/core/orderbook/IOrderBook.java`
- `ref/exchange-core/src/main/java/exchange/core2/core/orderbook/OrderBookDirectImpl.java`
- `ref/exchange-core/src/main/java/exchange/core2/core/processors/RiskEngine.java`
- `ref/exchange-core/src/main/java/exchange/core2/core/common/SymbolPositionRecord.java`
- `ref/exchange-core/src/main/java/exchange/core2/core/common/StateHash.java`

### Event sourcing and replay

- `ref/cqrs/src/aggregate.rs`
- `ref/cqrs/src/event.rs`
- `ref/cqrs/src/store.rs`
- `ref/cqrs/src/persist/replay.rs`
- `ref/orderbook-rs/src/orderbook/sequencer/types.rs`
- `ref/orderbook-rs/src/orderbook/sequencer/replay.rs`

### Simulation

- `ref/nexosim/nexosim/src/time.rs`
- `ref/nexosim/nexosim/src/simulation/sim_init.rs`
- `ref/nexosim/nexosim/src/simulation/queue_items.rs`
- `ref/abides/Kernel.py`
- `ref/abides/util/OrderBook.py`
- `ref/abides/agent/TradingAgent.py`
- `ref/abides/agent/NoiseAgent.py`
- `ref/abides/agent/ValueAgent.py`
- `ref/abides/config/rmsc01.py`
- `ref/abides/config/rmsc02.py`

### FIX

- `ref/ironfix/ironfix-tagvalue/src/encoder.rs`
- `ref/ironfix/ironfix-tagvalue/src/decoder.rs`
- `ref/ironfix/ironfix-core/src/message.rs`
- `ref/fixer/fixer/src/session/mod.rs`
- `ref/quickfixj/quickfixj-core/src/main/java/quickfix/Session.java`
- `ref/quickfixj/quickfixj-core/src/main/java/quickfix/SessionState.java`
- QuickFIX/J session/reset/sequence tests
- corresponding FerrumFIX tag-value/session layers after dictionary-license review

### Market making and adapters

- `ref/market-maker-rs/src/strategy/avellaneda_stoikov.rs`
- `ref/market-maker-rs/src/strategy/glft.rs`
- `ref/market-maker-rs/src/strategy/interface.rs`
- risk, execution and backtest modules under `ref/market-maker-rs/src/`
- `ref/ritc_mm/src/main.rs`
- `ref/nautilus-trader/docs/developer_guide/adapters.md`

## Existing in-repository references

### `ref/quarcc-trading-engine`

Useful behavior:

- order lifecycle and sequential dispatch semantics;
- execution gateway, journal, order-store and feed interfaces;
- local/external order identifier mapping;
- deferred and out-of-order fill handling;
- position projection;
- kill switch;
- unit tests and protobuf contracts.

Rejected architecture:

- threads, mutexes and native queues;
- recursive callbacks;
- gRPC service boundary;
- native SQLite implementation;
- wall-clock domain reads;
- floating-point order units;
- race recovery caused by separate dispatch/gateway threads.

License status is unresolved, so source copying is blocked. See `docs/ports/quarcc-trading-engine.md` and the `order-reconciliation` target in `docs/ports/ritc-market-making.md`.

### `ref/nbc_engine`

Contains five verified scenario families and parameter vocabulary. The source catalog, blob hashes, duplicate verification and unresolved units are in `docs/ports/nbc-scenario-catalog.md`.

A complete reviewed Java source/license set is not established. Treat the tree as data/provenance only. See `docs/ports/nbc-simulation.md`.

### `ref/ritc_mm`

Contains RIT API models, blocking transport, Avellaneda-Stoikov, GARCH, spectral analysis, queue signals, quote management and scenario duplicates. The implementation is monolithic and license status is unresolved.

Use it only as behavior/API inventory. Exact target crates and primitive mapping are in `docs/ports/ritc-market-making.md`.

## Port record requirements

Every port note records:

1. upstream path and exact commit/blob;
2. license and required notices;
3. retained behavior;
4. rejected behavior;
5. Bunting target crate/module;
6. platform and dependency impact;
7. equivalence, property, replay and fuzz tests;
8. local patches and future divergence;
9. whether material was copied, translated, independently reimplemented, or used only as an oracle.

## Dependency rule

A direct dependency requires:

- acceptable license and attribution plan;
- pure Rust or an explicitly isolated native boundary;
- bounded memory and input behavior;
- no hidden time, thread, filesystem, socket or global RNG authority in kernel paths;
- minimal-feature Wasm success for Worker code;
- measured binary-size and latency impact;
- deterministic replay tests where behavior affects canonical state.

Vendored source additionally follows `vendor/README.md`.