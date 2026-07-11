# Reference implementation inventory

This inventory records the role of each source in Bunting. Reference code is evidence, not production code. A submodule pin does not authorize copying: every extracted file still requires a license check, attribution, a local port note and equivalence tests.

## Selection criteria

A repository is admitted to `ref/` only when it contributes at least one of the following:

1. mature exchange or protocol semantics that can serve as a behavioral oracle;
2. implementation patterns directly relevant to a Bunting sprint;
3. substantial automated tests, replay fixtures or conformance coverage;
4. a permissive license compatible with study and possible adaptation;
5. a clear boundary that lets Bunting borrow behavior without importing an unsuitable runtime.

Popularity alone is not sufficient. Low-level performance claims are not treated as proof of correctness.

## Reference confidence classes

- **A — port donor:** selected files may be adapted after license and Wasm review.
- **B — behavioral oracle:** run or inspect it to derive fixtures and invariants; normally do not copy its architecture.
- **C — concept source:** use only for design comparison or protocol research.

## Pinned external repositories

| Local path | Upstream and pinned commit | License posture | Class | Intended use |
|---|---|---|---:|---|
| `ref/workers-rs` | `cloudflare/workers-rs@5f2d6c9192377451d43910098738624474196364` | verify pinned tree before extraction | A | Worker runtime APIs, Durable Objects, SQLite, WebSockets, Queues, RPC, examples and build conventions |
| `ref/nbc-hft-simulation` | `carterj-c/NBC_HFT_Simulation@35b8050546679547dc737198ea13aa0ec8ed7db8` | verify pinned tree before extraction | B | legacy REST/WebSocket contract, lockstep `DONE`, participant clients, scenario names and compatibility fixtures |
| `ref/ironfix` | `joaquinbejar/IronFix@6ac17a37f7c4efbdfe97a06f428809733d88b66b` | verify every selected crate/file | A | candidate FIX core types, dictionaries and tag-value codec; session algorithms require Worker-specific extraction |
| `ref/fixer` | `fixer-rs/fixer@c1c27c3287d6f275a9c33122cc2af063de7c5a08` | verify pinned tree before extraction | B | native FIX interoperability oracle and bridge reference; not a Worker dependency because it is Tokio/native-I/O coupled |
| `ref/ferrumfix` | `ferrumfix/ferrumfix@ca2bbe4c6461108646f35f7cc9245bf1848ec368` | verify pinned tree before extraction | C | FIX layering and conformance reference; upstream describes the project as unstable |
| `ref/nautilus-trader` | `nautechsystems/nautilus_trader@c28b1335c95abbf1bef2385def9a75a1b3862f76` | preserve upstream license and generated-code notices | B | current adapter architecture, normalized domain mappings, reconciliation and test patterns |
| `ref/wirefilter` | `cloudflare/wirefilter@61936e5f38523df3f80880bbc662e490b52e7f86` | verify pinned tree before extraction | C | optional conditional-rule engine after Wasm size, latency and security evaluation |
| `ref/orderbook-rs` | `joaquinbejar/OrderBook-rs@575de34260b0fce346372074b6b938df058693a8` | MIT | A/B | Rust order-book decomposition, explicit clocks, snapshots, replay, deterministic traversal fixes, risk hooks and test ideas |
| `ref/liquibook` | `enewhuis/liquibook@2427613b32f1667abae68a01df6af9ba8270f8e7` | OCI BSD-style license; retain notice | A/B | compact matching semantics, order lifecycle callbacks, depth maintenance and extensive order-condition tests |
| `ref/exchange-core` | `exchange-core/exchange-core@2f8548749839e9095c8dc597e4b61521d259fa5d` | Apache-2.0 | B | integer exchange accounting, pre-risk, matching, state hashes, journaling/snapshot invariants and stress tests |
| `ref/cqrs` | `serverlesstechnology/cqrs@b13692ce3db62b3b7fea19dddeec90a9d8af3180` | Apache-2.0 | A/C | aggregate/event/store separation, optimistic stream versions, replay and projection boundaries |
| `ref/nexosim` | `asynchronics/nexosim@42eb361c9c553e50b763524cf9087bb64f31af6c` | MIT OR Apache-2.0 | B/C | typed model ports, discrete-event scheduling, monotonic simulation time and save/restore concepts |
| `ref/abides` | `abides-sim/abides@c4bf157678928934417aba6073eb0651aeaf6d15` | BSD-3-Clause | B/C | exchange-agent boundary, latency-aware message scheduling, seeded agent configurations and market-agent taxonomy |
| `ref/quickfixj` | `quickfix-j/quickfixj@73e45dbe487be54d7c2badec2a846a45ef116ce2` | QuickFIX Software License 1.0; attribution required | B | independent FIX session oracle, resend/gap-fill/reset behavior and a large session test suite |
| `ref/market-maker-rs` | `joaquinbejar/market-maker-rs@36899f3e910997400bc95c3a8f3606776c002fbe` | MIT | A/B | Rust strategy decomposition for Avellaneda–Stoikov, inventory/risk controls, fill models and connector traits |

The `workers-rs` submodule already contains its official `examples/` directory; it is not duplicated separately.

## High-value files to inspect first

### Matching and order books

- `ref/orderbook-rs/src/orderbook/book.rs`
- `ref/orderbook-rs/src/orderbook/matching.rs`
- `ref/orderbook-rs/src/orderbook/snapshot.rs`
- `ref/orderbook-rs/src/orderbook/sequencer/replay.rs`
- `ref/orderbook-rs/src/orderbook/clock.rs`
- `ref/orderbook-rs/src/orderbook/risk.rs`
- `ref/liquibook/src/book/order_book.h`
- `ref/liquibook/src/book/order_tracker.h`
- `ref/liquibook/src/book/comparable_price.h`
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
- exchange-core journaling, serialization and state-hash tests

### Deterministic simulation

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

- `ref/quickfixj/quickfixj-core/src/main/java/quickfix/Session.java`
- `ref/quickfixj/quickfixj-core/src/main/java/quickfix/SessionState.java`
- `ref/quickfixj/quickfixj-core/src/test/java/quickfix/SessionTest.java`
- `ref/quickfixj/quickfixj-core/src/test/java/quickfix/SessionStateTest.java`
- `ref/quickfixj/quickfixj-core/src/test/java/quickfix/SessionResetTest.java`
- corresponding wire, dictionary and session files in IronFix, Fixer and FerrumFIX

### Market making and RITC

- `ref/market-maker-rs/src/strategy/avellaneda_stoikov.rs`
- `ref/market-maker-rs/src/strategy/interface.rs`
- risk, execution and backtest modules under `ref/market-maker-rs/src/`
- `ref/ritc_mm/src/main.rs`
- NautilusTrader adapter and strategy examples relevant to order reconciliation

## Existing in-repository references

### `ref/quarcc-trading-engine`

C++ trading engine. Useful concepts:

- sequential order-manager event dispatch;
- execution gateway, journal, order-store and market-feed interfaces;
- local/broker order identifier mapping;
- deferred fill handling;
- position keeping;
- SQLite journal and order-store tests;
- kill-switch workflow;
- protobuf contracts.

Do not port directly:

- `std::jthread`, mutex and native event-queue architecture;
- gRPC server;
- native SQLite implementation;
- wall-clock acquisition from domain code;
- floating-point order quantities and prices;
- race-recovery mechanics caused by separate gateway and dispatch threads.

The Durable Object supplies a single authoritative sequencer, so the Rust port preserves business semantics while removing thread races and native-service assumptions. See `docs/ports/quarcc-trading-engine.md`.

### `ref/nbc_engine`

Compiled Java simulator assets and calibrated scenario definitions. Useful concepts:

- normal, stressed, flash-crash, mini-flash-crash and HFT-dominated scenarios;
- fundamental, momentum, noise, market-making, institutional and spiking agents;
- deterministic seeds and step intervals;
- legacy application configuration.

The snapshot does not expose a complete reviewed Java source tree. Scenario files are behavioral inputs, not proof of implementation correctness. See `docs/ports/nbc-simulation.md`.

### `ref/ritc_mm`

Rust RITC strategy and adapter reference. Useful concepts:

- RIT API models;
- Avellaneda–Stoikov quoting;
- queue imbalance;
- GARCH volatility;
- spectral order-flow analysis;
- market-making configuration and calibration artifacts.

Do not import it as engine code. The current implementation combines blocking HTTP, threads, sleeps, wall time, floating-point analytics, order management and strategy state in one binary. See `docs/ports/ritc-market-making.md`.

## Porting rule

Every port must create or update a note under `docs/ports/` containing:

1. upstream path and exact commit;
2. upstream license and required notices;
3. behavior retained;
4. behavior rejected;
5. new Bunting abstraction;
6. Wasm compatibility and dependency impact;
7. equivalence, property and replay tests;
8. local patches and future divergence;
9. whether code was copied, translated, rewritten from behavior, or used only as an oracle.

## Dependency rule

A repository being present under `ref/` does not make it a dependency. The default decision is to rewrite the smallest deterministic behavior behind Bunting-owned interfaces. A direct dependency requires all of the following:

- pure Rust or an isolated native-only boundary;
- acceptable license and attribution plan;
- bounded memory behavior;
- no hidden wall clock, thread, filesystem, socket or global RNG in kernel paths;
- successful `wasm32-unknown-unknown` build when used by Worker code;
- measured binary-size and latency impact;
- tests proving deterministic behavior across replay.
