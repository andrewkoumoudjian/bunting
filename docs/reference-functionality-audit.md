# Reference and vendored functionality audit

Status: authoritative classification baseline for repository planning

Last reviewed: 2026-07-12

## Purpose

This document records what each checked-in or submodule reference actually implements. It replaces role guesses based on repository names and separates:

1. **observed functionality** proved by source, manifests, public contracts, or the project's own documentation;
2. **unverified functionality** that cannot be established from the recorded snapshot;
3. **Bunting disposition**: dependency, port target, adapter model, conformance oracle, design reference, test utility, or no current use.

No architecture or port plan may classify a reference without consulting this audit and the cited source paths.

## Inventory and pin discipline

The repository contains:

- 25 Git submodules declared in `.gitmodules`;
- three checked-in reference trees: `ref/nbc_engine`, `ref/quarcc-trading-engine`, and `ref/ritc_mm`;
- no vendored implementation under `vendor/`; only policy files are tracked there.

`.gitmodules` records upstream URLs and branch hints, not the authoritative checked-out gitlink commit. Before updating, comparing, or claiming equivalence against a submodule, record its exact pin with:

```bash
git ls-tree HEAD ref/<name>
git -C ref/<name> rev-parse HEAD
```

The two values must agree. Record the pin in the relevant audit or port document. Do not silently audit the current upstream default branch as though it were the repository pin.

## Classification vocabulary

- **Market/venue engine:** owns authoritative venue or simulated-market state and processes participant orders.
- **Matching kernel:** performs order-book matching but does not necessarily provide accounts, run lifecycle, protocols, persistence, or scoring.
- **Participant execution engine:** runs outside the venue; consumes market data, manages orders/risk/positions, and routes commands to a venue.
- **Protocol stack:** parses, validates, sequences, stores, transports, or generates protocol messages; it does not match orders merely because messages contain orders.
- **Simulation framework:** supplies clocks, schedulers, actors, messages, latency, or save/restore mechanics; it is not automatically a market engine.
- **Infrastructure utility:** generic runtime, data structure, serialization, policy, or testing functionality.

---

# Development-only protocol conformance references

## `@trpc/server` and `@trpc/client` 11.18.0

### Pin, license, and selected source

The official repository is `https://github.com/trpc/trpc.git`; version `11.18.0`
is source commit `6aec1578a899df50a17e4e78d5512a099b574c18` and is MIT licensed. The
license file has Git blob `f9ff685d4b60f34316f8ea28eb300351d3741725` and SHA-256
`e714dd84c8fa242600844b05d317a31003423723178c1f1603dbfad1bc68d906`.
The selected `packages/server/package.json` and `packages/client/package.json`
manifests declare version `11.18.0`, MIT, ESM/CJS exports, and TypeScript peer
requirements; their SHA-256 values are respectively
`a0a75b49ef3c231cdbc2c59c0763ceb33a5132572fb520039f8b2e88bcc620f8` and
`2bb0cfcd277781d2a2748a34b1fb0cea200ed04d472717334ae380c8aaf67453`.

Selected source entrypoints and SHA-256 evidence:

- Fetch adapter `packages/server/src/adapters/fetch/fetchRequestHandler.ts`: `f0f2922f81440e2519cdb58419c62a5d29a495f894f6828e35121748059d00c7`;
- HTTP resolver and parser `resolveResponse.ts` / `contentType.ts`: `5dd1ca29642e31f37881929cc4679e53b0b7ef08fc09459dacff02d343dd0f25` / `1566670a72baca4a37608f0342444fba519ad48ef362700aa9543d4a7683f685`;
- error shape and transformer `getErrorShape.ts` / `transformer.ts`: `a0ce73abd3b7fe5387157b4e9ddbf68fd79eb8a84ab3a201282934c540ad84c0` / `95458dc9a904d72418e74cc8133d3b9e4ec2f8e3872eccbef57f890cb7088a35`;
- SSE producer `packages/server/src/unstable-core-do-not-import/stream/sse.ts`: `849d984eafe8454bb59ba43af8a2912940c4c1461b97b758c5d68fba4980b880`;
- HTTP batch and subscription clients `httpBatchLink.ts` / `httpSubscriptionLink.ts`: `42ec21255034dc7ec5a56d8c84b75e1236541aa8468930fc3e083a387078ef97` / `e8a9b21b0c8d6804c4865460dacb282b004f069a85a750d91853ab3a7648dd1a`;
- shared client URL/body encoding `links/internals/httpUtils.ts`: `8f959d6c0216dd551c1622f475060df035116c6ac8219e34aafa8bdde4bd15c5`.

### Observed functionality

The Fetch adapter trims endpoint slashes, extracts the procedure path, and
delegates to the HTTP resolver. The resolver accepts GET queries/subscriptions
and POST mutations by default, rejects wrong methods, supports comma-separated
`batch=1` calls, emits structured result/error envelopes, maps error codes to
HTTP statuses, and advertises `vary: trpc-accept, accept`. Its JSON parser reads
GET input from the percent-encoded `input` parameter and POST input from a JSON
body; batch input is an object keyed by call index and mixed procedure types are
rejected. The selected client batch link constructs the same paths and inputs.

The HTTP subscription client uses EventSource/SSE, maintains tracked event IDs,
reports connecting/pending/idle states, reconnects retryable errors, and aborts
on cancellation. The server SSE producer emits connected, data, serialized-error
and ping lifecycle frames. Default transformer behavior is identity JSON;
SuperJSON, devalue, JSONL streaming, multipart, octet input, WebSockets, and
method override exist in the broader upstream surface but are outside Bunting's
selected subset.

### Worker/Wasm impact and Bunting disposition

This is a development-only differential oracle. Its TypeScript packages and
Node tooling stay under `tests/`; they are absent from production Cargo and
Worker manifests, so they add no Worker/Wasm runtime or transitive dependency.
The Bunting Rust implementation owns bounds, authentication, domain errors,
commit-before-acknowledgement, and the deliberately narrower feature set frozen
in `schemas/trpc/bunting.v1.json`. Committed normalized fixtures can be checked
offline; refreshing them requires the pinned official packages and explicit
review.

Evidence classification: the transport behavior above is **observed** from the
pinned selected source and generated fixtures; request bounds, mutation-batch
rejection, identity-only transformation, recovery semantics, and unsupported
feature policy are **Bunting-added** restrictions.

# A. Market engines, matching kernels, and market simulation

## `ref/orderbook-rs` — OrderBook-rs

### Observed functionality

A complete reusable matching and order-book kernel, not a complete Bunting deployment. Its public surface includes:

- concurrent limit-order-book state and price-time matching;
- limit and market operations plus optional/supported IOC, FOK, post-only, iceberg/reserve, pegged, trailing-stop, market-to-limit, and host-driven expiry behavior;
- direct cancel, replace, mass cancel, per-user indexing, self-trade prevention, fees, risk hooks, lifecycle tracking, kill-switch behavior, and trade/level-change results;
- depth, metrics, impact/placement analysis, snapshots, checksum validation, restore, sequencer/journal helpers, and optional wire/journal/NATS/metrics layers;
- deterministic host-supplied time paths needed for replay-sensitive operations.

The crate depends on `pricelevel` for the price-level/order substrate and exposes optional native/runtime features. Bunting currently uses released `orderbook-rs = 0.10.3` with `default-features = false`; features visible on newer upstream revisions must not be attributed to the pinned release without checking that pin.

### It is not

- a participant OMS/execution engine;
- a complete exchange with authentication, multi-run orchestration, participant accounts, D1 persistence, scoring, or Cloudflare deployment;
- a reason to copy native NATS, memory-mapped journal, thread-manager, or ambient-runtime examples into the Worker.

### Bunting disposition

Approved production matching dependency through the Bunting adapter. Prefer the released crate and upstream contributions. `packages/orderbook` should remain a first-party adapter/boundary, not an undisclosed fork.

Evidence: upstream `README.md`, `Cargo.toml`, `src/lib.rs`; Bunting ADR 0013.

## `ref/pricelevel` — PriceLevel

### Observed functionality

A lower-level order-domain and per-price queue library used by OrderBook-rs. It provides:

- price-level queues and matching at one price;
- order/order-type data structures, IDs, prices, quantities, timestamps, validation, and partial-fill behavior;
- maintained visible/hidden/total quantity and order-count aggregates;
- concurrent maps/queues, snapshots, serialization, and focused property/concurrency tests.

### It is not

A complete order book across prices, market engine, exchange, scheduler, protocol server, or trader OMS.

### Bunting disposition

Approved transitive production dependency pinned for type identity with OrderBook-rs. Do not build a parallel Bunting price-level implementation.

Evidence: upstream `README.md`, `Cargo.toml`.

## `ref/liquibook` — Liquibook

### Observed functionality

A header-only C++ matching kernel with optional depth tracking. It supports:

- buy/sell limit and market orders;
- stop price, all-or-none, immediate-or-cancel, and FOK through flag combinations;
- submit, cancel, and cancel/replace;
- callbacks for accepted/rejected/filled/replaced/cancelled orders;
- trade, security-change, depth-change, and BBO notifications;
- application-owned order objects and identifiers.

### It is not

A complete exchange account/risk/persistence/run system. The embedding application must execute settlement, publish market data, persist state, and provide connectivity.

### Bunting disposition

Independent matching oracle and source of focused semantic fixtures. No runtime dependency is planned.

Evidence: upstream `README.md`.

## `ref/exchange-core` — exchange-core

### Observed functionality

A full Java exchange core, not merely an accounting oracle. It includes:

- matching engines and multiple order-book implementations;
- pre/post-trade risk control and user/account balance accounting;
- direct-exchange and margin modes;
- maker/taker fees and fixed-point arithmetic;
- deterministic atomic command processing;
- event-sourced disk journaling, snapshots, compression, restore, and reports;
- trading, administration, symbol/user/balance, and report APIs;
- pipelined/sharded high-throughput processing.

### It is not

A Cloudflare/Wasm-ready dependency, a participant execution engine, or a network gateway implementation. Its README lists market-data feeds, clearing/settlement, and FIX/REST gateways among unfinished or external concerns.

### Bunting disposition

Full-exchange architecture and differential-invariant oracle, especially for account/risk atomicity, deterministic commands, reports, persistence, and state consistency. Do not describe it only as an accounting reference.

Evidence: upstream `README.md`.

## `ref/option-chain-orderbook` — Option-Chain-OrderBook

### Observed functionality

An options hierarchy and aggregation system built on OrderBook-rs. It organizes:

- underlying → expiration → chain → strike → call/put leaf books;
- leaf matching through OrderBook-rs;
- option instrument registration and lifecycle;
- Greeks/mark-price and chain-level views;
- scoped commands, mass cancellation, expiry processing, snapshots, and optional sequencer/NATS layers.

### It is not

An independent leaf matching implementation or a general equity exchange core.

### Bunting disposition

Future options-package dependency candidate. Adopt through its API only after options scope, dependency size, numerical policy, and Wasm suitability are approved.

Evidence: upstream `README.md`, `Cargo.toml`.

## `ref/nbc_engine` — packaged NBC Exchange Simulator snapshot

### Observed functionality

The checked-in tree proves the existence and external shape of an NBC **market/exchange simulator application**:

- the application README names an `Exchange Simulator` and instructs running `exchange-simulator-0.0.1-SNAPSHOT.jar` on port 8080;
- `application.yml` configures a Spring application, SQLite/JPA persistence, `/api` context path, team registration/passwords, JWT authentication, run rate limits, and simulator step settings;
- five scenario JSON files configure deterministic seeds, durations, step intervals, fundamental-value parameters, tick size/spread, and populations of fundamental, long/short momentum, noise, market-maker, institutional, and spiking traders;
- the observed NBC protocol, recorded by its client and RIT adapter, exposes scenario listing/start, per-team runs, history and leaderboards; market-data and order WebSockets; new limit orders, cancellation, fills/errors, and a mandatory `DONE` message that advances the simulation step;
- observable score/run fields include PnL, inventory, notional, trade count, aggressive quantity, blow-up state, and decision time.

### Missing from the direct recorded snapshot

The checked-in tree does **not** contain the Java source or the JAR named by its README. Therefore the repository cannot currently prove:

- the exact matching algorithm, priority rules, partial-fill/cancel races, or internal book representation;
- the exact agent formulas and random distributions;
- scheduler ordering within a step;
- internal snapshot/replay/state-hash mechanics;
- database schema and transaction behavior;
- whether every API field in the separately recorded API reference is implemented by the same binary revision.

The scenario values are evidence of configuration, not proof of formula semantics or units beyond their field names.

The separately pinned `ref/nbc-hft-simulation` client tree contains `app/exchange-simulator-0.0.1-SNAPSHOT.jar`. Its SHA-256 and gitlink pin are recorded in `docs/ports/nbc-evidence-manifest.v1.json`; ADR 0017 selects it as the authorized source/reference runtime for the Rust port and records project-owner authority for bytecode inspection, translation and redistribution. Its original source, license metadata, build provenance and relationship to the direct snapshot remain unresolved.

Bounded Sprint 6 inspection records 40 NBC application classes and their exact hashes in `docs/ports/nbc-jar-inventory.v1.tsv`. Bytecode directly establishes configuration, run-context, scheduler, order-book, market-data, agent, metrics and persistence class boundaries, while one isolated credential-free runtime observation establishes that `GET /api/replays` returns HTTP 200 with five scenario identifiers. These observations narrow the implementation evidence; they do not establish authenticated lifecycle, WebSocket/`DONE`, matching edge cases, scoring/termination equivalence, or a complete relationship to the direct snapshot.

Sprint 7.1 translates the bytecode-observed `ScenarioConfig`/`ScenarioService`
input boundary and the JAR-embedded normal-market resource into the first real
`packages/nbc-market-engine` slice. The translation ledger and differential
fixture preserve exact class/resource hashes. Strict rejection, explicit units,
bounded input, inert unresolved parameters, and deterministic configuration and
provenance hashes are Bunting-added; they are not attributed to the JAR.

### It is not

Merely scenario data. The observable package and authorized JAR are a venue-side simulator. The direct snapshot alone is insufficient for an exact internal translation; translated claims require JAR bytecode or reproducible runtime evidence.

### Bunting disposition

Authorized compatibility translation input to the single `packages/bunting-engine` target under ADR 0018. The existing `packages/nbc-market-engine` crate is transitional evidence-bearing code until its proven behavior is integrated. The port document must label each behavior as externally observed, bytecode-observed, translated, inferred, Bunting-added or unresolved. Exact compatibility claims require reproducible JAR-versus-Rust evidence.

Evidence: `ref/nbc_engine/app/README.md`, `application.yml`, scenario JSON; `ref/nbc-hft-simulation`; `ref/ritc_mm/API_REFERENCE.md` and adapter.

## `ref/abides` — ABIDES

### Observed functionality

An agent-based interactive discrete-event market simulation environment. It provides:

- many trading agents interacting through an exchange agent;
- a message-driven simulation kernel;
- configurable pairwise network latency;
- market protocols modeled after published equity order-entry and market-data protocols;
- configurations and an environment intended for AI/market-agent research.

### It is not

Only a collection of agent formulas. It includes simulation, exchange-agent, messaging, and latency architecture. It is also not a Rust/Wasm dependency.

### Bunting disposition

Market-simulation architecture, latency, agent/exchange separation, and experimental-oracle reference. Any adapted model must be independently licensed, unit-specified, deterministic under Bunting’s seed policy, and clearly distinguished from NBC behavior.

Evidence: upstream `README.md` and project documentation.

## `ref/fauxchange` — fauxchange

### Observed functionality

No implementation API exists at the recorded early release. The repository reserves a crate name and describes a future local exchange-simulation direction.

### Bunting disposition

Concept/roadmap reference only. Nothing can be depended on, ported, or copied as implementation.

Evidence: upstream `README.md`.

---

# B. Participant execution engines, trading platforms, and strategies

## `ref/quarcc-trading-engine` — QUARCC C++ trading/execution engine

### Observed functionality

A participant-side execution/OMS service, not venue matching. The checked-in C++ and protobuf contracts provide:

- a `TradingEngine` service with submit, cancel, replace, position queries, global kill switch, and market-data streaming;
- one `OrderManager` per strategy/account;
- strategy-signal conversion to orders;
- local order ID generation and local/broker ID mapping;
- execution gateways, simulated/paper and broker gateway boundaries;
- market-data feeds and feed registry;
- participant-side risk checks;
- order store and journal interfaces with SQLite implementations;
- execution-report handling, position projection, deferred handling when fills arrive before ID mapping, and sequential event dispatch per order manager;
- gRPC service contracts and a Python client/strategy surface.

### It is not

An exchange matching engine or authoritative market ledger. It routes to gateways and reconciles execution reports produced elsewhere.

### Bunting disposition

First-class optional Rust port target: `packages/quarcc-execution-engine`. Preserve a portable execution core and isolate native gRPC, sockets, broker SDKs, and SQLite/filesystem adapters. The existing Rust compatibility crate implements only part of the public contract and is not a complete port.

Evidence: `contracts/execution_service.proto`, `engine-cpp/include/trading/core/trading_engine.h`, `order_manager.h`, gateway/store/journal interfaces, tests, Python client.

## `ref/ritc_mm` — RIT market-making strategy and NBC↔RIT adapter

### Observed functionality

A participant-side Rust market-making application and compatibility adapter:

- the main strategy polls the RIT REST API and implements queue-imbalance/fill estimates, FFT order-flow analysis, GARCH volatility, and Avellaneda–Stoikov-style reservation-price/spread/inventory logic;
- it maintains local history, position limits, quote placement/cancellation, and API response types;
- `rit_sim_adapter` connects to the NBC simulator’s REST/WebSocket protocol, maintains a local projection of market data, orders, history, position, cash, and PnL, and exposes a RIT-compatible local REST surface;
- calibration scripts and artifacts estimate microstructure parameters.

### It is not

A market/venue engine. It does not authoritatively match orders; it sends orders to RIT or NBC and projects the resulting data/fills.

### Bunting disposition

Participant strategy, analytics, adapter, and conformance-fixture reference. A future first-party port should be split by real responsibility—for example pure market-making models versus an RIT client/adapter—not represented as an NBC market-engine component.

Evidence: `ref/ritc_mm/Cargo.toml`, `src/main.rs`, `src/bin/rit_sim_adapter.rs`, `API_REFERENCE.md`, calibration scripts.

## Supplied RIT installer corpus — proprietary static evidence

### Observed functionality

Static extraction of the two supplied MSI installers establishes the packaged file, feature, COM registration, managed type/member, import/export, resource, literal, and embedded protocol surfaces recorded in [`research/rit-binary-audit/`](research/rit-binary-audit/). The evidence includes local named-pipe endpoints, the versioned REST route inventory, the Excel RTD ProgID/topic catalog, default ports, and the public API facade shape. The source manifest records exact SHA-256 and SHA-512 identities for the installers; the binaries and extracted payloads remain outside Git.

### It is not

Source authorization, a redistributable dependency, or proof of runtime semantics hidden behind the observed surfaces. Static strings and metadata do not establish matching formulas, scheduler ordering, field units, limit behavior, timing, persistence, or trust status on a current Windows host.

### Bunting disposition

Clean-room external-contract and conformance evidence only. No proprietary byte sequence, decompiled method body, installer, extracted payload, credential, or resource may enter production code or Git. Derived requirements remain linked to stable `RIT-FEATURE-*` entries, and unresolved behavior must be observed independently or implemented as an explicit Bunting addition.

## `ref/nbc-hft-simulation` — NBC student/client template

### Observed functionality

A Python participant client/template:

- registers a team for a named scenario and receives token/run ID;
- connects to separate market-data and order-entry WebSockets;
- receives step, bid/ask, depth/trades where present;
- allows a user to implement `decide_order`;
- sends limit orders and `DONE` step acknowledgements;
- receives authentication, fill, and error messages;
- maintains local inventory, cash-flow, PnL, and latency measurements;
- includes a manual trader.

### It is not

The NBC exchange simulator or its internal market engine.

### Bunting disposition

External compatibility fixture and user-experience reference for NBC clients. Do not use its local PnL/inventory as authoritative venue accounting.

Evidence: upstream `README.md`, `student_algorithm.py`, `manual_trader.py`.

## `ref/nautilus-trader` — NautilusTrader

### Observed functionality

A broad production-oriented participant trading platform with a Rust-native event-driven core and Python control plane. Its workspace contains model, data, execution, portfolio, risk, trading, backtest, live, persistence, event-store, serialization, infrastructure, network, analysis, indicators, adapters, testkit, and Python binding layers. It supports research/backtest and live execution through normalized venue/data adapters.

### It is not

A single matching kernel or an implementation to embed wholesale in a Cloudflare Worker.

### Bunting disposition

Architecture and integration reference for QUARCC/client/execution packages, research-to-live parity, normalized adapter boundaries, portfolio/risk separation, and testkit design. Its LGPL license and very large native dependency graph require careful isolation; no wholesale source adoption.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/barter-rs` — Barter

### Observed functionality

A Rust participant-side algorithmic-trading ecosystem composed of:

- an engine with indexed state management and audit streams;
- instrument/asset definitions;
- public market-data streams;
- private account data and execution clients;
- REST/WebSocket integration primitives;
- strategy and risk-manager plug-ins;
- live, paper, and backtest modes;
- OMS functionality, commands, state replicas, and performance summaries.

### It is not

An exchange matching engine.

### Bunting disposition

Reference for modular execution clients, market-data streams, OMS/audit state, and participant-engine composition. Prefer concepts or normal released dependencies over source copying.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/market-maker-rs` — market-maker-rs

### Observed functionality

A participant-side quantitative market-making library containing pure models plus optional runtime layers:

- Avellaneda–Stoikov and related quoting models, grid/adaptive behavior, inventory/risk controls, analytics and VPIN-like measures;
- quote/order intent generation and backtesting;
- optional data feeds, persistence, event, API/metrics, options, option-chain, and multi-underlying features;
- an older direct dependency on OrderBook-rs than Bunting’s current production version.

### It is not

A venue market engine.

### Bunting disposition

Selective formula/test and participant strategy architecture reference. Any formula adaptation requires exact units, rounding, source/license, and golden vectors. Do not import the whole crate into the Worker kernel.

Evidence: upstream `README.md`, `Cargo.toml`.

---

# C. FIX, SBE, and protocol infrastructure

## `ref/ironfix` — IronFix

### Observed functionality

A multi-crate FIX/FAST stack, not a single codec. The workspace includes:

- core FIX types;
- data dictionaries;
- tag-value parsing/encoding;
- session management;
- stores;
- transports;
- FAST support;
- code generation and derive macros;
- a high-level engine and examples.

Its transport/session/store/engine layers pull in native async/runtime concerns that are distinct from the pure codec/dictionary layers.

### Bunting disposition

Primary Rust FIX candidate, evaluated per subcrate. First spike should test dictionary + tag-value/core on native and Wasm. Session, store, transport, and engine adoption require separate decisions and conformance tests.

Evidence: upstream `README.md`, workspace `Cargo.toml`.

## `ref/fixer` — fixer

### Observed functionality

A Rust FIX engine workspace with:

- core engine/session behavior;
- FIX message packages and generated typed messages;
- runtime specification validation and customizable specs;
- sequence management, persistence/logging, scheduling, failover/high-availability features, TLS, and multiple store backends;
- generator and test packages.

### Bunting disposition

Native FIX session/conformance reference and possible component candidate after license, maturity, target, and dependency review. It is not simply interchangeable with a tag-value codec.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/ferrumfix` — FerrumFIX

### Observed functionality

A layered Rust FIX/FAST implementation separating transport, session, presentation, and application concerns. It provides tag-value and JSON-related representations, generated definitions for several FIX versions, validation, parsing/serialization, and recovery-oriented protocol structure, while documenting incomplete/unstable areas.

Its code license and bundled specification-derived material have different obligations; specification data must not be copied casually.

### Bunting disposition

Layering, parser/error, and conformance reference. Do not import bundled specification material without license review.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/quickfixj` — QuickFIX/J

### Observed functionality

A mature Java FIX messaging/session engine with:

- initiator and acceptor roles;
- session lifecycle, logon/logout, sequence numbers, resend/recovery, heartbeats, validation, stores, and logs;
- generated messages and message crackers across FIX/FIXT versions;
- extensive configuration and tests.

### Bunting disposition

High-value external conformance oracle and fixture generator for FIX session/message behavior. It is not a Bunting runtime dependency or market engine.

Evidence: upstream `README.md` and modules.

## `ref/ironsbe` — IronSBE

### Observed functionality

A multi-crate SBE and low-latency transport stack containing:

- core zero-copy encoding/decoding;
- XML schema parsing/validation;
- code generation and derive support;
- channels;
- general transports plus optional specialized native transports;
- client/server layers;
- market-data sequencing, gap detection, snapshot/recovery patterns;
- benchmarks.

### Bunting disposition

Evaluate per subcrate. Core/schema/codegen may be candidates for compact binary protocols. Channel/transport/client/server/market-data layers are native systems and require separate architecture, target, size, and security review. Do not call the whole repository “an SBE codec.”

Evidence: upstream `README.md`, root `Cargo.toml`.

---

# D. Platform, event sourcing, filtering, and simulation frameworks

## `ref/workers-rs` — workers-rs

### Observed functionality

The official Rust binding/build ecosystem for Cloudflare Workers. Its workspace includes the worker API crate, generated low-level bindings, macros, build tooling, tests, examples, and benchmarks. The API covers HTTP/router integration, Cache API, D1, KV, queues, WebSockets, Durable Objects and other Worker bindings according to selected features.

### Bunting disposition

Approved production platform dependency. Use official APIs; do not reimplement platform bindings.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/cqrs` — cqrs-es

### Observed functionality

A generic CQRS/event-sourcing framework:

- aggregate command handling and event application;
- separate write/read models and queries/views;
- framework/test abstractions;
- PostgreSQL, MySQL, and DynamoDB persistence packages, with SQLite available separately.

### It is not

Market-specific persistence, a matching engine, or a ready-made D1 adapter.

### Bunting disposition

Event-sourcing vocabulary, aggregate tests, and persistence-boundary reference. Bunting’s exact D1 expected-version transaction remains its own contract.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/nexosim` — NeXosim

### Observed functionality

A general Rust discrete-event simulation framework with:

- component/actor-like models and typed ports;
- message passing and scheduled events;
- simulation time and next-event advancement;
- a custom asynchronous multithreaded executor;
- save/restore, event injection, real-time support, macros, and utilities.

### It is not

A market engine or an NBC implementation.

### Bunting disposition

Simulation-runtime and save/restore design reference. Its custom native executor is not automatically suitable for deterministic single-threaded Wasm execution; adopt concepts or narrow packages only after a target-specific spike.

Evidence: upstream `README.md`, root `Cargo.toml`.

## `ref/wirefilter` — Wirefilter

### Observed functionality

A generic expression engine that:

- defines a typed field scheme;
- parses Wireshark-like filters into an AST;
- compiles an executable intermediate representation;
- evaluates filters against runtime values;
- supports optional regex and fuzz testing.

### Bunting disposition

Potential policy/scenario/admin predicate component. It does not supply matching, risk, scheduling, or protocol transport.

Evidence: upstream `README.md`, `engine/Cargo.toml`.

---

# E. Generic Rust utilities and test infrastructure

## `ref/slotmap` — slotmap

Generational/stable-key containers (`SlotMap`, `HopSlotMap`, `DenseSlotMap`) with secondary maps and O(1) insert/access/remove. Historical candidate for safe handle-backed state; not required by the current upstream matcher unless another package has a concrete ownership need.

Evidence: upstream `README.md`, `Cargo.toml`.

## `ref/intrusive-rs` — intrusive-collections

Intrusive singly/doubly linked lists and red-black trees with cursor-based mutation, optional allocation support, and `no_std` compatibility. A data-structure reference, not an order book or scheduler.

Evidence: upstream `README.md`, `Cargo.toml`.

## `ref/rand` — Rand

RNG traits, system/thread/seeded generators, distributions, sequence operations, and portability/reproducibility guidance. Reproducibility depends on selecting and versioning a concrete algorithm and sampling contract; `rand` alone is not a stable simulation stream specification.

Evidence: upstream `README.md`, `Cargo.toml`.

## `ref/postcard` — Postcard

A compact Serde serializer/deserializer designed for constrained and `no_std` environments, with a documented stable wire format, varint integer encoding, and configurable serialization flavors. It is a format candidate, not a snapshot/versioning policy by itself.

Evidence: upstream `README.md`, workspace `Cargo.toml`.

## `ref/proptest` — Proptest

Property-based test generation and per-value shrinking, failure minimization/persistence, strategies, and optional fork/timeout features. It is a dev/test dependency for invariants, not production engine code.

Evidence: upstream `README.md`, `proptest/Cargo.toml`.

---

# F. Vendored source

## `vendor/`

No third-party implementation is currently vendored. The directory contains only policy files. A vendored component must carry license/notice/upstream/patch records and pass the admission gates in `vendor/README.md`.

`packages/` is for first-party maintained Bunting packages. A copied or patched upstream tree does not become first-party merely by being moved there. Prefer, in order:

1. released dependency;
2. upstream contribution and released fix;
3. exact git dependency/fork in a dedicated upstream repository when justified;
4. narrowly vendored source under `vendor/` with complete provenance.

---

# G. Names mentioned in documentation but not present in the reference inventory

The following have appeared in prose audits but are not declared by the current `.gitmodules` inventory and are not checked-in reference trees:

- `matchbook`;
- `OptionStratLib` as a standalone reference tree;
- `OptionChain-Simulator`;
- `deribit-fix`, `alpaca-rs`, `ig-client`, `DXlink`, `otc-rfq`, and `quant-trading-system`.

Some may be transitive dependencies or previously researched repositories. They must not appear in the authoritative reference matrix until added with an exact URL, gitlink/pin, license record, and functionality audit.

---

# H. Corrected architecture implications

1. **Bunting has one production market engine.** OrderBook-rs supplies its matching kernel, while NBC contributes complete provenance-linked venue compatibility behavior; NBC cannot be reduced to scenarios or registered as a second production kernel, and its current snapshot does not prove internal matching or agent formulas.
2. **QUARCC, RITC market making, NautilusTrader, Barter, market-maker-rs, and the NBC student client are participant-side systems.** They belong around the client/execution/strategy boundary.
3. **OrderBook-rs, PriceLevel, and Liquibook are matching/order-book components of different scope.** exchange-core is a full exchange core.
4. **FIX and SBE repositories are layered protocol workspaces.** Adoption must be per subcrate/layer.
5. **ABIDES is a complete market-simulation environment; NeXosim is a general simulation runtime.** They are not interchangeable.
6. **No third-party source is currently vendored.** `vendor/` remains the only in-repository location for approved copied/patched upstream source.
7. **Every port plan must maintain an evidence table:** observed, inferred, Bunting-added, unresolved, and prohibited-to-copy.

## Required follow-up before the mechanical reorganization

- make this audit required reading in root `AGENTS.md` and the reorganization contract;
- replace the old reference matrix with the classifications above;
- revise NBC and QUARCC port documents to separate observed behavior from proposed Bunting features;
- record exact gitlink SHAs for all 25 submodules in a generated or reviewed manifest;
- remove absent references from authoritative matrices or add them properly;
- do not create `packages/fix` or another catch-all package before selecting concrete codec/session/transport boundaries;
- do not create an OrderBook-rs source copy under `packages/`; keep the first-party adapter there and place any approved patched upstream source under `vendor/` or a dedicated fork repository.
