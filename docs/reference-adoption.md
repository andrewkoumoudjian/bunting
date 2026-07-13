# Reference adoption, dependency, and source-copy policy

ADR 0013 defines the current default Worker/OrderBook-rs market path. ADR 0014 defines market-engine versus participant execution-engine authority.

The authoritative functionality inventory is [`reference-functionality-audit.md`](reference-functionality-audit.md). This document records adoption policy and disposition; it must not redefine a reference’s role without updating the source-backed audit first.

## Required evidence before a decision

For every reference or vendored component, record:

1. exact repository URL and checked-out gitlink/commit;
2. license for code, generated files, schemas, and specification-derived material;
3. manifests/workspace members and feature flags;
4. public entrypoints, core modules, tests, and example/runtime boundaries;
5. observed functionality versus inferred functionality;
6. what Bunting would depend on, port, adapt, test against, or reject;
7. native/Wasm target status and transitive dependency impact.

`.gitmodules` branch entries are not commit pins. Verify submodules with `git ls-tree HEAD` and `git -C ref/<name> rev-parse HEAD` before citing a revision.

## Global rules

- Production manifests never use paths under `ref/`.
- Prefer a released dependency and stable public API.
- Prefer an upstream contribution over a local fork.
- Copy or adapt source only after file-level license review and when a normal dependency cannot satisfy the requirement.
- A close adaptation records repository, commit, path, SPDX license, retained behavior, and local divergence.
- A whole-repository copy requires a dedicated ADR; ADR 0017 authorizes only the selected NBC JAR-derived port under its provenance rules.
- Worker-bound dependencies must pass a minimal-feature `wasm32-unknown-unknown` build and size review.
- ADR 0017 authorizes NBC JAR translation and redistribution. Other NBC material and QUARCC remain restricted without documented authority.
- Reference behavior, Bunting-added behavior, and unresolved behavior remain explicitly separated.

## Approved production dependencies

| Dependency | Approved version/use | Boundary |
|---|---|---|
| `worker` / workers-rs | `0.8.5`; Worker runtime, Router/HTTP, WebSocket, Cache API, D1 and selected bindings | Platform only; no market semantics |
| `orderbook-rs` | `0.10.3`, `default-features = false`; current default market engine’s matching/order-book kernel | Matching/book behavior; Bunting owns run, identity, accounts, persistence, protocols and deployment |
| `pricelevel` | `0.8.4`; transitive order/price-level type identity | Lower-level order and per-price queue substrate |

The first-party `packages/orderbook` path is the Bunting adapter around the released dependency. It is not an upstream source copy.

## Pending development conformance intake

| Candidate | Observed version/source | Intended boundary |
|---|---|---|
| `@trpc/server` / `@trpc/client` | `11.18.0`, npm git head `6aec1578a899df50a17e4e78d5512a099b574c18`, MIT | Development-only wire/fixture oracle for ADR 0016; not a production Worker dependency until Sprint 0 completes source, manifest and protocol-entrypoint audit |

## Audited disposition matrix

### Market/venue and matching references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `orderbook-rs` | Complete reusable matching/order-book kernel with lifecycle, risk hooks, fees, snapshots/replay helpers, depth/analytics and optional native layers | Production dependency for the default engine |
| `pricelevel` | Order-domain and per-price concurrent queue/matching substrate | Approved transitive dependency |
| `liquibook` | Embeddable C++ matching kernel with application callbacks and optional depth | Independent matching oracle and focused fixture source |
| `exchange-core` | Full Java exchange core: matching, risk/accounting, commands/reports, journaling and snapshots | Full-exchange architecture and invariant oracle; no runtime dependency |
| `option-chain-orderbook` | Options hierarchy and aggregation built on OrderBook-rs leaf books | Future options dependency candidate; evaluate API/dependencies/Wasm first |
| `nbc_engine` | Packaged NBC exchange simulator assets/config/scenarios and observable venue protocol; the direct snapshot lacks implementation source/JAR, while the pinned client tree contains the project-owner-authorized JAR | First-class authorized Rust market-engine translation target under ADR 0017; compatibility claims require JAR-linked evidence |
| `abides` | Agent-based discrete-event market simulator with exchange agent, messaging and configurable latency | Market-simulation architecture and experimental oracle |
| `fauxchange` | Reserved/planned project with no implementation API | No code adoption; roadmap reference only |

### Participant execution, trading, and strategy references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `quarcc-trading-engine` | Participant OMS/execution service: strategy signals, order manager, gateways, risk, IDs, journal/store, positions, market data and gRPC/Python clients | First-class optional Rust execution-engine port target |
| `ritc_mm` | Participant market-making strategy plus NBC-to-RIT compatibility adapter and calibration tooling | Pure-model, adapter and conformance reference; not a market engine |
| `nbc-hft-simulation` | Student/manual participant client for NBC REST/WebSocket/DONE protocol | External compatibility and UX fixtures |
| `nautilus-trader` | Large participant trading platform with execution, risk, portfolio, data, backtest/live, persistence and adapters | QUARCC/client/execution architecture reference; no wholesale adoption |
| `barter-rs` | Modular participant engine, market/private data, execution clients, OMS, risk and audit state | Execution/client architecture reference |
| `market-maker-rs` | Participant market-making models and optional runtime/API/options layers | Selective formula/test reference after exact unit and version review |

### Protocol references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `ironfix` | Multi-crate FIX/FAST stack: core, dictionary, tag-value, session, stores, transport, codegen/derive and engine | Primary Rust candidate evaluated per subcrate; core/codec spike first |
| `fixer` | Rust FIX engine with generated messages, runtime specs, sessions, stores/logging, scheduling and HA features | Native conformance/session reference and possible component candidate |
| `ferrumfix` | Layered FIX/FAST parser/session/presentation/application implementation with unstable/incomplete areas | Layering/error/conformance reference; specification-data license caution |
| `quickfixj` | Mature Java FIX initiator/acceptor/session engine and generated message model | External conformance oracle and fixture generator |
| `ironsbe` | Multi-crate SBE codec/schema/codegen plus channels, transports, client/server and market-data recovery | Evaluate per subcrate; core/schema/codegen separately from native runtime layers |

Do not create one generic `packages/fix` or `packages/sbe` dumping ground before choosing actual codec, dictionary, session, store and transport boundaries.

### Platform, simulation, persistence and policy references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `workers-rs` | Official Rust bindings, macros and build tooling for Cloudflare Workers | Production platform dependency |
| `cqrs` | Generic CQRS/event-sourcing aggregate and persistence framework | Concept/test/persistence-boundary reference; not a D1 implementation |
| `nexosim` | General component-based discrete-event simulator with custom async executor and save/restore | Simulation-runtime design reference; target-specific spike required |
| `wirefilter` | Typed filter parser, compiler and execution engine | Optional policy/predicate candidate |

### Generic utility and test references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `slotmap` | Stable generational-key containers and secondary maps | Use only for a concrete ownership/arena requirement |
| `intrusive-rs` | Intrusive lists and red-black trees | Data-structure reference; no current production need |
| `rand` | RNG traits, generators, distributions and sampling | Dependency candidate, but simulation algorithms/streams must be explicitly versioned |
| `postcard` | Compact stable-format Serde serializer/deserializer | Snapshot/wire experiment only after versioning and compatibility design |
| `proptest` | Property-based generation, shrinking and failure persistence | Development/test dependency |

## Local port-source restrictions

### NBC

The current `ref/nbc_engine` snapshot proves the packaged application and observable interface but does not include its Java source or named JAR. A separate pinned client tree contains the selected same-named JAR. ADR 0017 authorizes inspection, decompilation, Rust translation and redistribution; exact internal matching, scheduler, agent, database or replay claims still require cited bytecode or differential evidence. See `docs/ports/nbc-simulation.md`.

### QUARCC

The C++ source and protobuf contracts prove a participant-side execution/OMS architecture. No repository-level license is recorded. Use interface/behavior evidence or documented authorization; do not mechanically translate implementation text. See `docs/ports/quarcc-trading-engine.md`.

### RITC market maker

The Rust source is a participant market-making application and adapter. It does not supply venue matching. License status must be resolved before source adaptation.

## References not currently present

`matchbook`, `OptionStratLib` as a standalone ref, `OptionChain-Simulator`, `deribit-fix`, `alpaca-rs`, `ig-client`, `DXlink`, `otc-rfq`, and `quant-trading-system` have appeared in prose but are not in the current submodule or checked-in reference inventory. They are excluded from the authoritative matrix until added with a URL, exact pin, license and functionality audit.

## Fork and vendoring policy

A release-blocking OrderBook-rs issue should be handled in this order:

1. feature/configuration change;
2. upstream issue and contribution;
3. released upstream fix;
4. dedicated pinned fork repository;
5. narrowly vendored source under `vendor/orderbook-rs` only when repository or build constraints require it.

Do not place copied upstream source under `packages/orderbook`. `packages/` contains first-party Bunting packages; `vendor/` contains approved copied/patched third-party source.

Any fork or vendored source requires:

- exact upstream release and commit;
- complete license/notice files;
- changed-file inventory and `PATCHES.md`;
- native and Wasm verification where relevant;
- snapshot/wire compatibility tests;
- update owner and synchronization cadence;
- an exit/upstreaming plan.

## Required review on every reference update

- verify gitlink and worktree commit;
- review changelog and public API changes;
- rerun license and dependency metadata checks;
- rerun relevant conformance/differential tests;
- update `reference-functionality-audit.md` when functionality or package boundaries change;
- never infer a role from a repository name alone.
