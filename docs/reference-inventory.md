# Reference implementation inventory

This inventory records why each source exists under `ref/`. Reference code is evidence, not production code. A submodule pin does not authorize copying: every extracted file requires a license check, attribution, a port note and equivalence tests.

Operational dependency, vendoring and borrowing decisions are normative in:

- `docs/reference-adoption.md` for the original reference set;
- `docs/rust-reference-sprint-map.md` for the Rust sprint-support additions;
- `docs/ports/` for source-target port mappings.

## Selection criteria

A repository is admitted to `ref/` only when it contributes at least one of:

1. mature exchange or protocol semantics that can serve as a behavioral oracle;
2. implementation patterns directly relevant to a Bunting sprint;
3. substantial automated tests, replay fixtures, fuzzing or conformance coverage;
4. a known license compatible with its intended use;
5. a clear boundary that lets Bunting borrow behavior without importing an unsuitable runtime.

Popularity and performance claims are not proof of correctness. Repositories without a clear license are not added as new submodules.

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
| `ref/barter-rs` | `barter-rs/barter-rs@33e56188e2095781331f85aa3d7f88e251eec65a` | MIT | A/B/C | Rust OMS/risk type flow, indexed state, execution boundaries and audit replicas |
| `ref/slotmap` | `orlp/slotmap@0d130ed5bbd6e51fbb64a6b6cd80d3adfbb04294` | Zlib | A | stable generational handles for an order arena and linked-list examples |
| `ref/intrusive-rs` | `Amanieu/intrusive-rs@e7b27a0ea9a23084a62c5896161b8805db72e5b9` | MIT OR Apache-2.0 | A/C | intrusive FIFO/list/tree invariants, cursor mutation and allocation behavior |
| `ref/rand` | `rust-random/rand@8272d49f03b4900b648df39b24d9a0343cbd45b5` | MIT OR Apache-2.0 | A/C | explicit seeded RNGs, distributions and reproducibility constraints |
| `ref/postcard` | `jamesmunns/postcard@de182557cff45f2ca9b2b67a6b93be5917612a44` | MIT OR Apache-2.0 | A/C | stable compact no-std snapshot encoding candidate |
| `ref/proptest` | `proptest-rs/proptest@85a3de393331b951e20c5da1cdac9d342fc36a6f` | MIT OR Apache-2.0 | A/C | property/state-machine generation, shrinking and persistent regressions |

The `workers-rs` submodule already contains official `examples/`; it is not duplicated separately.

## Current adoption summary

- **Approved production dependency:** released Cloudflare `worker` packages in Worker-only crates.
- **Expected dev dependency:** Proptest for generated invariant and state-machine tests, after toolchain compatibility confirmation.
- **Dependency spikes:** minimal IronFix codec crates, `wirefilter-engine`, SlotMap for the order arena, Rand with an explicitly named algorithm, and Postcard for snapshot payloads.
- **Selective source candidates:** narrow IronFix codec code, pure `market-maker-rs` formulas/tests and deterministic OrderBook-rs or intrusive-collection test material.
- **External integration contract:** NautilusTrader.
- **Architecture/oracle references:** Barter, Liquibook, OrderBook-rs, exchange-core, Fixer, FerrumFIX, QuickFIX/J, ABIDES, NeXosim and intrusive-rs.
- **Data/compatibility sources:** NBC assets and client.
- **No whole-reference vendoring is approved.**

## High-value paths

### Cloudflare runtime

- `ref/workers-rs/worker/src/durable.rs`
- `ref/workers-rs/worker/src/websocket.rs`
- `ref/workers-rs/test/src/durable.rs`
- `ref/workers-rs/test/src/sql_counter.rs`
- `ref/workers-rs/test/src/sql_iterator.rs`

### Matching, handles, risk and accounting

- `ref/orderbook-rs/src/orderbook/book.rs`
- `ref/orderbook-rs/src/orderbook/matching.rs`
- `ref/orderbook-rs/src/orderbook/snapshot.rs`
- `ref/orderbook-rs/src/orderbook/sequencer/replay.rs`
- `ref/liquibook/src/book/order_book.h`
- `ref/liquibook/src/book/order_tracker.h`
- `ref/exchange-core/src/main/java/exchange/core2/core/processors/RiskEngine.java`
- `ref/exchange-core/src/main/java/exchange/core2/core/common/SymbolPositionRecord.java`
- `ref/barter-rs/barter/src/risk/mod.rs`
- `ref/barter-rs/barter/src/risk/check/`
- `ref/barter-rs/barter/src/engine/state/`
- `ref/slotmap/src/basic.rs`
- `ref/slotmap/src/dense.rs`
- `ref/slotmap/examples/doubly_linked_list.rs`
- `ref/intrusive-rs/src/linked_list.rs`
- `ref/intrusive-rs/src/adapter.rs`
- `ref/intrusive-rs/src/rbtree.rs`

### Event sourcing, audit and replay

- `ref/cqrs/src/aggregate.rs`
- `ref/cqrs/src/event.rs`
- `ref/cqrs/src/store.rs`
- `ref/cqrs/src/persist/replay.rs`
- `ref/barter-rs/barter/examples/engine_sync_with_audit_replica_engine_state.rs`
- `ref/barter-rs/barter/tests/test_engine_process_engine_event_with_audit.rs`
- `ref/postcard/spec/`
- `ref/postcard/src/ser/`
- `ref/postcard/src/de/`

### Simulation and randomness

- `ref/nexosim/nexosim/src/time.rs`
- `ref/nexosim/nexosim/src/simulation/queue_items.rs`
- `ref/abides/Kernel.py`
- `ref/abides/agent/NoiseAgent.py`
- `ref/abides/agent/ValueAgent.py`
- `ref/rand/src/rngs/mod.rs`
- `ref/rand/src/rngs/std.rs`

### Property and state-machine testing

- `ref/proptest/proptest/src/strategy/`
- `ref/proptest/proptest/src/collection.rs`
- `ref/proptest/proptest/src/test_runner/`
- `ref/proptest/proptest-state-machine/`
- `ref/slotmap/fuzz/fuzz_targets/target.rs`

### FIX

- `ref/ironfix/ironfix-tagvalue/src/encoder.rs`
- `ref/ironfix/ironfix-tagvalue/src/decoder.rs`
- `ref/fixer/fixer/src/session/mod.rs`
- `ref/quickfixj/quickfixj-core/src/main/java/quickfix/Session.java`
- QuickFIX/J session/reset/sequence tests

### Market making and adapters

- `ref/market-maker-rs/src/strategy/avellaneda_stoikov.rs`
- `ref/market-maker-rs/src/strategy/glft.rs`
- `ref/market-maker-rs/src/strategy/interface.rs`
- `ref/barter-rs/barter-execution/`
- `ref/barter-rs/barter-integration/`
- `ref/nautilus-trader/docs/developer_guide/adapters.md`
- `ref/ritc_mm/src/main.rs`

## Existing in-repository references

### `ref/quarcc-trading-engine`

Use only as an internal behavioral source for lifecycle transitions, identifier mapping, deferred/out-of-order fills, position projection and kill-switch semantics. Reject its threads, callbacks, gRPC, native SQLite, wall-clock and floating-point architecture. Source copying remains blocked until ownership/license is recorded.

### `ref/nbc_engine`

Contains five verified scenario families and parameter vocabulary. Treat the tree as data/provenance only until licensing and unit semantics are resolved. See `docs/ports/nbc-scenario-catalog.md`.

### `ref/ritc_mm`

Contains RIT API models, blocking transport, market-making analytics and duplicate NBC scenario files. Use only as behavior/API inventory while its license remains unresolved. See `docs/ports/ritc-market-making.md`.

## Rejected Rust candidate

`cmd05/lob-engine-rs` was technically relevant—integer ticks, FIFO matching, deterministic replay and latency scheduling—but was not added because no repository license was present at the audited revision. It must be re-audited if a license is added.

## Port and dependency rules

Every port note records the upstream path/commit, license, retained and rejected behavior, target crate, platform impact, tests, divergence and whether material was copied, translated, independently reimplemented or used only as an oracle.

A direct dependency additionally requires:

- an acceptable license and attribution plan;
- pure Rust or an explicitly isolated native boundary;
- bounded memory and input behavior;
- no hidden time, thread, filesystem, socket or global RNG authority in kernel paths;
- minimal-feature Wasm success for Worker code;
- measured binary-size and latency impact;
- deterministic replay tests where behavior affects canonical state.

Vendored source follows `vendor/README.md`; production manifests never use paths under `ref/`.
