# Reference adoption, dependency, and source-copy policy

Reconciled 2026-10-09 (ADR 0033). ADR 0018 requires one production `bunting-engine` and carries ADR 0014's market-versus-participant authority split. ADR 0029 replaced OrderBook-rs with the engine's own book; OrderBook-rs is now a development-only oracle. ADR 0022 removed the Cloudflare command Worker, so workers-rs is not a current dependency. ADR 0032 makes NBC reference evidence only.

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
- Dependencies of host-neutral packages (`bunting-engine`, protocol packages) must pass a minimal-feature `wasm32-unknown-unknown` build and size review. Server dependencies must build natively and must not be WASIX-only (ADR 0033).
- ADR 0017 authorizes NBC JAR translation and redistribution; ADR 0032 limits NBC to reference evidence, so new NBC translation into production code needs a new ADR. Other NBC material and QUARCC remain restricted without documented authority.
- Reference behavior, Bunting-added behavior, and unresolved behavior remain explicitly separated.

## Approved production dependencies

| Dependency | Approved version/use | Boundary |
|---|---|---|
| `ratatui` / `crossterm` | `0.30.2` / `0.29.0`; native local-test terminal UI and terminal event backend | Native app presentation/input only; no market semantics |
| `rustls` / `tokio-rustls` / `rustls-native-certs` / `rustls-pemfile` | `0.23.42` / `0.26.4` / `0.8.4` / `2.2.0`; native FIX initiator TLS, platform trust roots and optional PEM CA loading | Native client transport only (`bunting-tui`); no FIX sequencing and no market semantics. Server-side in-process mutual TLS has its own row below |
| `rustls` / `rustls-pemfile` (BNP) | `0.23.42` (`default-features = false`, features `ring`, `std`) / `2.2.0`; Apache-2.0 OR ISC OR MIT / Apache-2.0 OR MIT | Native targets only (`cfg(not(target_arch = "wasm32"))`). In `bunting-server`: TLS 1.3 termination, client CA chain and CRL verification for the BNP listener (ADR 0040). In `bunting-client`: the client side of the same handshake. Identity mapping, framing and every protocol decision stay first-party; no market semantics |
| `clap` (`env` feature) | `4.6.1`, MIT OR Apache-2.0, features `derive`, `env` | Command-line parsing for `bunting-trader` (and `derive` only in `bunting-cli`, `bunting-tui`); no market semantics |
| `rustyfix-dictionary` | exact crates.io release `0.7.4`, Apache-2.0, upstream source commit `2f0ef7830553d482765c14e3c4b32be3432d57b0`; features `fix50sp2`, `fixt11` only | Production standard message/field/datatype lookup in `simfix-wire`; no engine, session, transport or copied dictionary resources, and FIX Latest Orchestra remains normative |
| Wasmer / cargo-wasix / WASIX Rust | Wasmer `7.2.1` at `c14032594b893b40e9b71456d504cf55c141c8f6`; cargo-wasix `0.1.28` at `b2d0e1c874fc6ac5dbaf71715b12c6809104767f`; toolchain `v2026-07-07.3+rust-1.96` | Current packaging and runtime for the server under ADR 0027; not a binding long-term host (ADR 0033); no market semantics, and filesystem/network capabilities remain explicit |

The engine's order book (`packages/bunting-engine/src/book.rs`, ADR 0029) is first-party code, not an upstream source copy. The former OrderBook-rs adapter, the `packages/orderbook` crate and the workers-rs Worker are removed.

## Approved development-only conformance oracles

| Candidate | Observed version/source | Intended boundary |
|---|---|---|
| `@trpc/server` / `@trpc/client` | `11.18.0`, source git head `6aec1578a899df50a17e4e78d5512a099b574c18`, MIT; historical manifests and transport entrypoints remain recorded in the functionality audit | Retired oracle. The Node harness and fixtures were removed on 2026-07-28 after tRPC ceased to be an architecture or runtime dependency; this row preserves identity only and authorizes no active dependency. |
| `orderbook-rs` / `pricelevel` | `=0.10.3` / `=0.8.4`, `default-features = false`, `[dev-dependencies]` of `bunting-engine` only | Differential matching oracle for the owned book (ADR 0029). CI asserts it is absent from `bunting-engine`'s normal dependency tree. |
| `rcgen` | `=0.14.10`, MIT OR Apache-2.0, `default-features = false`, features `crypto`, `pem`, `ring` | `[dev-dependencies]` of `bunting-server` only: generates a throwaway CA, server and client certificates and a CRL per BNP test run, so no key material is committed. Absent from every production dependency tree. |
| `quickfixgo/quickfix` | exact Go module `v0.9.10`, QuickFIX Software License 1.0, `github.com/quickfixgo/quickfix` | Development-only external FIXT.1.1/FIX 5.0 SP2 serializer/parser oracle under `tests/interop/quickfixgo`; it drives the native TCP acceptor and is absent from every production manifest. |

## Audited disposition matrix

### Market/venue and matching references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `orderbook-rs` | Complete reusable matching/order-book kernel with lifecycle, risk hooks, fees, snapshots/replay helpers, depth/analytics and optional native layers | Development-only differential oracle (ADR 0029); not a production dependency |
| `pricelevel` | Order-domain and per-price concurrent queue/matching substrate | Approved transitive dependency |
| `liquibook` | Embeddable C++ matching kernel with application callbacks and optional depth | Independent matching oracle and focused fixture source |
| `exchange-core` | Full Java exchange core: matching, risk/accounting, commands/reports, journaling and snapshots | Full-exchange architecture and invariant oracle; no runtime dependency |
| `option-chain-orderbook` | Options hierarchy and aggregation built on OrderBook-rs leaf books | Design reference only: it builds on OrderBook-rs leaf books, which ADR 0029's single owned book excludes from production. Options need Bunting-native book semantics |
| `nbc_engine` | Packaged NBC exchange simulator assets/config/scenarios and observable venue protocol; the direct snapshot lacks implementation source/JAR, while the pinned client tree contains the project-owner-authorized JAR | Reference evidence only (ADR 0032); the engine's compatibility surface was removed in slice 11. ADR 0017 still governs reading the JAR |
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
| `workers-rs` | Official Rust bindings, macros and build tooling for Cloudflare Workers | Not a current dependency (Worker removed, ADR 0022); candidate only for a future read-only publisher |
| `cqrs` | Generic CQRS/event-sourcing aggregate and persistence framework | Mirror deregistered; consult published crate/upstream pin recorded in the audit |
| `nexosim` | General component-based discrete-event simulator with custom async executor and save/restore | Mirror deregistered; consult published crate/upstream pin recorded in the audit |
| `wirefilter` | Typed filter parser, compiler and execution engine | Mirror deregistered; consult published crate/upstream pin recorded in the audit |

### Generic utility and test references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `slotmap` | Stable generational-key containers and secondary maps | Mirror deregistered; use the published crate only for a concrete ownership requirement |
| `intrusive-rs` | Intrusive lists and red-black trees | Mirror deregistered; no current production need |
| `rand` | RNG traits, generators, distributions and sampling | Mirror deregistered; any dependency still requires a versioned algorithm/stream contract |
| `postcard` | Compact stable-format Serde serializer/deserializer | Mirror deregistered; evaluate the published crate only after snapshot versioning design |
| `proptest` | Property-based generation, shrinking and failure persistence | Mirror deregistered; use the published dev dependency when tests justify it |

### Terminal UI references

| Reference | Actual implemented role | Disposition |
|---|---|---|
| `makeev/alphai-tui` | MIT Rust/Ratatui stock dashboard with split market views, semantic key mapping and isolated application/UI modules at `f814697c6159d76b2dfb503ba5201b8c3fb702ad` | Historical adaptation input for the superseded first CLI; no AlphaAI source remains active after the Longbridge-first `bunting-tui` rewrite, but its retained license and provenance record the removed adaptation |
| `longbridge/longbridge-terminal` | Apache-2.0 Rust/Ratatui trading terminal with explicit application, input, navigation, popup, rendering, view, UI-helper and widget layers plus an MIT `cli-candlestick-chart` package exposed as version `0.24.0` at `05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3` | Approved source adaptation of the complete `src/tui` tree and the chart package's required MIT modules into `apps/bunting-tui`; remove the network-bound git dependency, exclude its CLI/examples/optional integrations, feed the chart only bounded Bunting FIX/TCP book projections, and retain Apache-2.0 and MIT licenses, exact commit/path provenance, Longbridge attribution, and prominent modification notices in changed files |

## Local port-source restrictions

### RIT installer corpus

The supplied RIT MSI files are proprietary binary evidence, not dependencies or redistributable source. Static extraction may inform clean-room external contracts and conformance fixtures, but no installer, payload, resource, decompiled body, credential, or proprietary byte sequence may enter Git or a production manifest. Exact hashes, static methods, derived protocol inventories, feature coverage, and unresolved evidence are recorded in [`research/rit-binary-audit/`](research/rit-binary-audit/); any behavior beyond that evidence remains unresolved or Bunting-added.

### NBC

The current `ref/nbc_engine` snapshot proves the packaged application and observable interface but does not include its Java source or named JAR. A separate pinned client tree contains the selected same-named JAR. ADR 0017 authorizes inspection, decompilation, Rust translation and redistribution. The bounded class/resource inventory and selected bytecode observations now live in `docs/ports/nbc-jar-inventory.v1.tsv` and `docs/ports/nbc-behavior-evidence.md`; exact translated behavior still requires the cited class/resource hashes and reproducible JAR-versus-Rust evidence. See `docs/ports/nbc-simulation.md`.

### QUARCC

The C++ source and protobuf contracts prove a participant-side execution/OMS architecture. No repository-level license is recorded. Use interface/behavior evidence or documented authorization; do not mechanically translate implementation text. See `docs/ports/quarcc-trading-engine.md`.

### RITC market maker

The Rust source is a participant market-making application and adapter. It does not supply venue matching. License status must be resolved before source adaptation.

## References not currently present

`matchbook`, `OptionStratLib` as a standalone ref, `OptionChain-Simulator`, `deribit-fix`, `alpaca-rs`, `ig-client`, `DXlink`, `otc-rfq`, and `quant-trading-system` have appeared in prose but are not in the current submodule or checked-in reference inventory. They are excluded from the authoritative matrix until added with a URL, exact pin, license and functionality audit.

## Fork and vendoring policy

A release-blocking issue in any production dependency should be handled in this order:

1. feature/configuration change;
2. upstream issue and contribution;
3. released upstream fix;
4. dedicated pinned fork repository;
5. narrowly vendored source under `vendor/<name>` only when repository or build constraints require it.

Do not place copied upstream source under `packages/`. `packages/` contains first-party Bunting packages and adapters; `vendor/` contains approved copied/patched third-party source.

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
