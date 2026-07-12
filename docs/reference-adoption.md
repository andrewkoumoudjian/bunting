# Reference adoption, dependency, and source-copy policy

ADR 0013 is authoritative for the market kernel and Worker runtime.

## Rules

- Production code uses released package dependencies, not paths under `ref/`.
- `ref/` records exact source revisions, examples, tests, licenses, and upgrade evidence.
- Prefer calling upstream APIs over copying implementation files.
- Any close copy of an example, test, or implementation unit records repository, path, commit, SPDX license, retained behavior, and local changes.
- A whole-repository copy requires a separate ADR and is currently prohibited.
- Worker dependencies must pass the exact `wasm32-unknown-unknown` build.

## Approved production dependencies

| Reference | Pin | Approved use |
|---|---|---|
| `workers-rs` | released `worker = 0.8.5`; source reference `5f2d6c9192377451d43910098738624474196364` | plain Worker runtime, Router, WebSocket, bindings, and Workers Cache API |
| `orderbook-rs` | released `0.10.3`; audited source `575de34260b0fce346372074b6b938df058693a8` | production order book and matching kernel |
| `pricelevel` | released `0.8.4`; audited source `a5b61671391295783d0e35ba68fdf4a9702dee60` | transitive upstream order, price-level, trade, and validation types |

OrderBook-rs is no longer an oracle-only reference. Bunting must not maintain a parallel production matching implementation.

## OrderBook-rs adoption

Use directly:

- limit, market, IOC, FOK, post-only, and supported special-order paths;
- price-time matching and partial-fill priority;
- per-call trade-result APIs;
- direct cancel, replace, mass cancel, STP, fees, lifecycle, and typed rejects;
- operational kill switch and drain behavior;
- book-level risk configuration;
- host-driven expiry;
- engine sequence and listeners;
- snapshot package, checksum validation, restore, and replay helpers;
- depth, iterators, metrics, impact, placement, and enriched snapshots.

Copy/adapt only when Bunting needs a protocol or recovery fixture. High-value MIT candidates are documented in `docs/orderbook-rs-example-adoption.md`.

Do not copy into the Worker:

- thread/channel managers;
- sleep-based loops;
- NATS publishers;
- file or memory-mapped journals;
- native socket/runtime scaffolding;
- demo logging and ambient time helpers.

## Dependency and reference matrix

| Reference | Role | Production disposition | Source-copy disposition |
|---|---|---|---|
| `liquibook` | independent matching oracle | no runtime dependency | translate focused tests with notice |
| `exchange-core` | accounting, risk, and state-hash oracle | no runtime dependency | translate fixtures/invariants with Apache attribution |
| `barter-rs` | OMS, audit, and connector architecture | native/reference only | narrow MIT traits/tests when useful |
| `market-maker-rs` | market-making formulas and strategy decomposition | no whole-crate dependency currently | pure functions/tests after unit review |
| `option-chain-orderbook` | future options hierarchy built on OrderBook-rs | future dependency candidate | prefer dependency/API use |
| `IronSBE` | future SBE codec and market-data recovery | isolated spike only | core/schema/codegen only after Wasm review |
| `fauxchange` | exchange-composition design intent | no code exists to depend on | nothing to copy at current pin |
| `matchbook` | Solana CLOB architecture reference | no | no production copy planned |
| `ironfix` | FIX codec candidate | minimal codec spike | narrow MIT codec units only if necessary |
| `fixer` | native FIX conformance peer | dev/native only | no engine copy |
| `ferrumfix` | FIX layering reference | no | avoid bundled specification material |
| `quickfixj` | FIX conformance oracle | test harness only | no production copy |
| `nautilus-trader` | external adapter contract | external integration | avoid copying implementation by default |
| `wirefilter` | optional admin/scenario predicates | spike only | no need if package works |
| `nexosim` | scheduler/save-restore concepts | no runtime dependency | concepts/tests only |
| `abides` | agents, latency, and scenario models | no runtime dependency | formulas/configs after provenance review |
| `rand` | explicit scenario random streams | future named-algorithm dependency | no vendoring |
| `postcard` | compact internal snapshot experiment | spike only | no vendoring |
| `proptest` | generated invariants | dev/test dependency | no production inclusion |
| `slotmap` | historical custom-arena candidate | not needed for production book | reference only |
| `intrusive-rs` | historical FIFO invariant reference | not needed for production book | reference only |
| `cqrs` | expected-version/event projection concepts | no | minimal ideas only |
| `quarcc-trading-engine` | lifecycle and reconciliation behavior | license unresolved | blocked |
| `nbc_engine` | scenario data and provenance | license unresolved | blocked pending review |
| `ritc_mm` | RITC behavior and formulas | license unresolved | blocked pending review |

## Joaquín repository conclusions

The focused audit is in `docs/joaquin-repository-audit.md`.

- OrderBook-rs and PriceLevel are adopted now.
- Option-Chain-OrderBook is the preferred future options composition.
- market-maker-rs is a selective formula/test donor, not a whole dependency today.
- IronSBE is a future binary-codec candidate, excluding native transport/runtime crates.
- fauxchange currently contains no implementation.
- matchbook is architecturally interesting but Solana-specific.

## Fork policy

A Bunting fork of OrderBook-rs is permitted only for a demonstrated release-blocking Wasm issue that cannot be solved with features or an upstream patch in time.

A fork must include:

- upstream commit and release;
- complete MIT notice;
- changed-file inventory;
- `PATCHES.md` with rationale;
- native and Wasm tests;
- snapshot compatibility tests;
- upstream synchronization owner and schedule.

A fork cannot become a pretext for replacing the upstream core with a new Bunting engine.
