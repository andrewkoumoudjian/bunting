# Reference adoption, vendoring and oracle policy

This document is the implementation-facing decision record for every source under `ref/`. It answers four questions for each reference:

1. what Bunting may learn from it;
2. where that behavior belongs in Bunting;
3. whether it may become a production dependency or vendored source;
4. what verification is required before any code is borrowed.

`ref/` is read-only research material. It is never placed on a Cargo, Python, Java or C++ production import path. A gitlink under `ref/` is not a dependency approval and is not a license approval.

## Adoption states

| State | Meaning |
|---|---|
| **approved dependency** | May be used by the named Bunting package through a normal package-manager dependency and lockfile. |
| **dependency spike** | May be tested on an isolated branch, but cannot enter production manifests until every listed gate passes. |
| **selective source candidate** | Small files or functions may be adapted into `vendor/` or Bunting-owned modules after file-level license, attribution and divergence review. |
| **behavioral oracle** | Use tests, examples or a runnable implementation to derive fixtures and invariants. Do not link or copy its runtime architecture. |
| **contract reference** | Defines an external API or integration convention Bunting must satisfy without embedding the repository. |
| **data/provenance only** | Preserve identifiers, parameters and source hashes. Do not copy implementation code. |
| **blocked** | No code or data may be copied until ownership or licensing is resolved. |

## Repository-wide rules

### Production dependency rules

- Production crates depend on released packages or Bunting-owned crates, never paths under `ref/`.
- Git dependencies are prohibited unless an ADR records why a released package cannot be used, the exact revision, update policy and supply-chain controls.
- Worker-bound dependencies must compile for `wasm32-unknown-unknown` with the exact features Bunting enables.
- A dependency must have bounded memory behavior, no hidden wall-clock or global-randomness authority, and no filesystem, thread or socket requirement in kernel paths.
- Package size, cold-start cost and transitive dependencies are measured before approval.
- Runtime behavior affecting replay must be covered by deterministic golden vectors.

### Source borrowing and vendoring rules

Any copied or translated implementation text goes under a Bunting-owned crate or `vendor/<component>/` and must include:

- upstream repository, path and exact commit;
- SPDX license identifier and required notice text;
- a `PATCHES.md` or equivalent divergence record;
- which behavior was retained and rejected;
- the reason a normal package dependency was not used;
- equivalence, property and fuzz tests;
- an owner and update policy.

Do not copy generated FIX dictionaries or protocol specification text until the license of that generated/specification material is separately verified.

### Oracle rules

External oracles run only in development or CI jobs that are isolated from production artifacts. Oracle output is normalized into Bunting-owned fixtures under `tests/fixtures/reference/`. Each fixture records the oracle commit, invocation, input and expected output. Bunting tests must remain runnable without network access.

## Summary decision matrix

| Reference | License posture at pin | Primary role | Production dependency | Source borrowing |
|---|---|---|---|---|
| `workers-rs` | Apache-2.0 | Cloudflare runtime contract | **approved**, Worker crates only | normally no |
| `nbc-hft-simulation` | no root license found | legacy client/protocol fixtures | no | blocked |
| `ironfix` | MIT | pure FIX codec candidate | spike: core/dictionary/tag-value only | possible after spike |
| `fixer` | Apache-2.0 | native FIX interoperability oracle | no; optional dev harness only | normally no |
| `ferrumfix` | MIT OR Apache-2.0; bundled FIX material has separate terms | FIX layering/conformance comparison | no | avoid specification/generated material |
| `nautilus-trader` | LGPL-3.0 | adapter contract and reconciliation patterns | external integration, not embedded | avoid unless separately reviewed |
| `wirefilter` | MIT | optional scenario/admin rule evaluator | spike only | no need if package passes |
| `orderbook-rs` | MIT | deterministic replay lessons and matching oracle | no | tests or narrow pure logic only |
| `liquibook` | OCI BSD-style | matching lifecycle oracle | no | translated tests/semantics with notice |
| `exchange-core` | Apache-2.0 | risk/accounting/state-hash oracle | no | translated fixtures and invariants only |
| `cqrs` | Apache-2.0 | aggregate/event-store design reference | no | minimal trait ideas only |
| `nexosim` | MIT OR Apache-2.0 | scheduler/save-restore reference | no | concepts and tests only |
| `abides` | BSD-3-Clause | scenario/agent/latency reference | no | agent formulas only after provenance review |
| `quickfixj` | QuickFIX Software License 1.0 | independent FIX conformance oracle | no | no production copying planned |
| `market-maker-rs` | MIT | pure market-making decomposition donor | no as a whole | selected pure functions/tests possible |
| `quarcc-trading-engine` | unresolved | lifecycle/mapping/reconciliation behavior | no | blocked until resolved |
| `nbc_engine` | unresolved | scenario data/provenance | no | blocked; catalog only |
| `ritc_mm` | unresolved | RIT models and strategy behavior inventory | no | blocked until resolved |

## Per-reference decisions

### `ref/workers-rs`

**Verified evidence**

- Workspace includes official examples and Worker runtime crates.
- The `worker` crate is Apache-2.0 and is designed for Cloudflare Workers/Wasm.
- High-value paths include `worker/src/durable.rs`, `worker/src/websocket.rs`, `worker/src/env.rs`, `test/src/durable.rs`, `test/src/sql_counter.rs`, `test/src/sql_iterator.rs` and the official examples.

**Bunting usage**

- Approved package dependency for `workers/edge-api`, `workers/market-run-do`, queue consumers and Worker-only support crates.
- Use official APIs for Durable Objects, SQLite, alarms, WebSockets, hibernation and bindings.
- Pin a released `worker` version and compatibility date; keep the submodule pin as implementation evidence.

**Do not borrow**

- Do not fork or vendor the runtime, macros or generated bindings unless an upstream defect makes it unavoidable and a dedicated ADR is approved.
- Do not leak Worker types into protocol-neutral crates.

**Verification gate**

- Exact Worker crates compile for Wasm.
- Durable Object SQLite transaction, restart and WebSocket tests run through the supported local Worker test path.
- Release-size and compatibility-date changes are reviewed.

### `ref/nbc-hft-simulation`

**Verified evidence**

- Provides the student Python client, registration route, market and order WebSocket URLs, message shapes, `DONE` barrier, fill handling and the five scenario names.
- No root `LICENSE` file was found at the pinned commit.

**Bunting usage**

- Contract reference and behavioral fixture source for `crates/protocol-legacy-nbc`, `workers/edge-api` and `tests/fixtures/reference/nbc/`.
- Recreate requests and responses from observed behavior; do not import the Python client.

**Vendoring decision**

- No dependency and no copied source while licensing is unresolved.
- Hand-authored protocol fixtures may record field names and example payloads required for compatibility, with source provenance.

**Verification gate**

- Registration, authentication, market snapshot, order submission, fill, error and `DONE` sequences are captured as fixtures.
- Legacy floating prices are validated and converted at the adapter boundary.
- Legacy protocol fields never enter canonical kernel events.

### `ref/ironfix`

**Verified evidence**

- MIT workspace with separate `ironfix-core`, `ironfix-dictionary`, `ironfix-tagvalue`, session, store, transport and engine crates.
- `ironfix-tagvalue` has a small pure dependency surface around bytes, small vectors, `memchr` and integer formatting.
- The complete workspace also includes Tokio, channels, locks, transport and native runtime concerns.

**Bunting usage**

- First dependency/source spike for `crates/simfix-wire` only.
- Candidate packages: `ironfix-tagvalue`, and only the minimum required portions of `ironfix-core` and `ironfix-dictionary`.
- Session, store, transport and engine crates remain references; Bunting owns Worker-compatible session and persistence traits.

**Vendoring decision**

- Prefer normal package dependencies if the exact crates are published, Wasm-compatible and sufficiently stable.
- Otherwise selectively adapt the smallest codec units with MIT attribution into `vendor/ironfix-tagvalue/` or directly into `simfix-wire` with a source header and patch record.
- Never vendor the full workspace.

**Verification gate**

- Wasm build with minimal features.
- BodyLength, CheckSum, field lookup, repeated tags, malformed delimiters and bounded-message tests.
- Fuzz corpus and measured Wasm-size impact.
- Byte-for-byte comparison against Fixer and QuickFIX/J vectors.

### `ref/fixer`

**Verified evidence**

- Apache-2.0 Rust FIX engine.
- Core crate depends on Tokio networking/time, TLS, concurrent maps and optional database/telemetry stacks.
- Session implementation lives under `fixer/src/session/`.

**Bunting usage**

- Native interoperability oracle for `clients/fix-bridge` and session tests.
- May be used as a dev-only executable or test dependency in a native CI job.

**Vendoring decision**

- Do not use in Worker crates and do not vendor its engine.
- Do not make it the implementation of Bunting's native bridge; the bridge remains a small transport adapter over Bunting-owned FIX wire/session behavior.

**Verification gate**

- Initiator and acceptor logon.
- Resend, gap-fill, duplicate and reset cases.
- Reconnect without silent sequence reset.
- Partial TCP reads and exact raw-message forwarding through the bridge.

### `ref/ferrumfix`

**Verified evidence**

- Dual MIT/Apache-2.0 codebase with explicit transport/session/presentation/application layering.
- Upstream describes the project as heavily developed and unstable before 1.0.
- Bundled FIX-related intellectual property has separate attribution/no-derivatives terms.

**Bunting usage**

- Concept and test-case reference for separation among `simfix-wire`, `simfix-session` and `simfix-mapping`.

**Vendoring decision**

- No production dependency.
- Do not copy bundled specification text or generated dictionaries without a separate legal review.
- Small dual-licensed code fragments are theoretically adaptable, but IronFix is the preferred Rust codec donor.

**Verification gate**

- Compare parsing and validation behavior only after input dictionaries and message vectors are confirmed legally reusable.

### `ref/nautilus-trader`

**Verified evidence**

- LGPL-3.0 repository.
- Adapter guide prescribes Rust HTTP/WebSocket/parsing clients, PyO3 bindings, Python providers/data/execution/factories/configuration, mock-server tests and startup reconciliation.

**Bunting usage**

- Contract reference for `clients/nautilus-adapter`.
- Bunting exposes its native HTTP/WebSocket API; the Nautilus package integrates externally through normal Nautilus extension mechanisms.

**Vendoring decision**

- Do not vendor NautilusTrader into Bunting.
- Do not copy adapter implementations by default. Implement Bunting-specific adapter code against the installed NautilusTrader interfaces and preserve required notices for any adapted LGPL material.

**Verification gate**

- Instrument mapping, market-data snapshot/delta sequence, order/fill/position reports, reconnect and reconciliation.
- Rust client and Python layer tests against a Bunting mock server.

### `ref/wirefilter`

**Verified evidence**

- MIT workspace with a separate engine crate and a Wasm crate.
- `wirefilter-engine` has a Wasm-specific randomness dependency and optional regex support.

**Bunting usage**

- Optional evaluator for administrator-authored scenario predicates or scoring filters.
- Never used for order matching, accounting, risk admission or canonical event validity.

**Vendoring decision**

- Package dependency spike only; no source vendoring is presently justified.
- Default implementation remains typed Rust rules until Wirefilter demonstrates material value.

**Verification gate**

- Wasm size and cold-start benchmark with and without regex.
- Deterministic evaluation tests for the allowed field schema.
- Parse depth, expression length, runtime and allocation bounds.
- Security review for untrusted expressions.

### `ref/orderbook-rs`

**Verified evidence**

- MIT Rust order book using `DashMap`, crossbeam skip lists, atomics and Tokio.
- Provides matching, snapshots, sequencer/replay, risk hooks, host-driven clock/expiry and extensive tests.
- Its changelog documents replay divergences caused by iteration through order-unstable concurrent structures and fixes them with explicit deterministic traversal.

**Bunting usage**

- Behavioral oracle and selective test donor for `crates/orderbook`, `crates/matching-engine` and replay tests.
- Adopt explicit host time, stable traversal, snapshot restoration checks, partial-fill queue preservation and mass-cancel ordering.

**Vendoring decision**

- No direct dependency in the kernel and no wholesale vendoring.
- Selective MIT-licensed pure test cases or serialization helpers may be adapted after path-level attribution.

**Verification gate**

- Differential vectors for FIFO, price priority, partial fills, IOC, market remainders, cancellation, mass cancel and snapshot/restore.
- Prove Bunting output ordering is independent of hash-map seeds.

### `ref/liquibook`

**Verified evidence**

- Mature C++ matching engine under an OCI BSD-style license requiring notice retention.
- Compact order tracker, price comparator, matching, lifecycle callback and depth-book implementations.

**Bunting usage**

- Primary independent matching oracle for `orderbook` and `matching-engine`.
- Translate its lifecycle and edge-case tests into Bunting command/event fixtures.

**Vendoring decision**

- No C++ runtime or FFI dependency.
- Algorithmic behavior and tests may be reimplemented in Rust. Any close translation must retain the OCI notice and be recorded in the relevant port note.

**Verification gate**

- Accept/reject/fill/cancel/replace and depth-change vectors.
- Maker-price, partial-fill, IOC and later AON/stop behavior comparisons.
- Explicitly document where Bunting intentionally differs.

### `ref/exchange-core`

**Verified evidence**

- Apache-2.0 Java exchange core with integer arithmetic, matching, pre-risk, accounting, journaling, snapshots, state hashes and integrity/stress tests.
- Runtime architecture uses Disruptor pipelines, sharding, object pools and native threading assumptions.

**Bunting usage**

- Primary oracle for `risk-engine`, `ledger`, replay/state hashes and atomic command semantics.
- Translate scale, reserve, balance-conservation and state-hash scenarios into Bunting fixtures.

**Vendoring decision**

- No Java dependency, no port of the Disruptor runtime and no mechanical class-by-class translation.
- Apache-licensed test logic may be translated with provenance when useful.

**Verification gate**

- Pre-risk rejection leaves book and ledger unchanged.
- Base/quote conservation, fees, reservation release and position limits.
- Live, replayed and snapshot-restored state hashes match.

### `ref/cqrs`

**Verified evidence**

- Apache-2.0 Rust CQRS/event-sourcing framework.
- Core crate uses async traits, Tokio and serde around aggregate, event, store, replay and projection abstractions.

**Bunting usage**

- Design reference for the `decide`/`apply` split, expected stream version, event envelopes, replay and projections.

**Vendoring decision**

- No direct dependency: Bunting's single-run aggregate needs a smaller synchronous pure interface and a Durable Object-specific transaction boundary.
- Minimal trait shapes may be adapted with Apache attribution if they remain useful after implementation.

**Verification gate**

- Command decisions have no persistence side effects.
- Atomic append checks expected version and idempotency.
- Projection failure cannot change canonical events.

### `ref/nexosim`

**Verified evidence**

- MIT OR Apache-2.0 Rust discrete-event framework with typed model ports, monotonic time, scheduled priority queues and save/restore.
- Runtime uses a custom asynchronous multi-threaded executor and channels.

**Bunting usage**

- Scheduler and snapshot test reference for `simulation-clock` and `scenario-engine`.

**Vendoring decision**

- No dependency and no executor port.
- Borrow concepts for typed scheduled items, monotonic time, explicit event injection and save/restore tests.

**Verification gate**

- Total-order tie breaking, monotonic scheduling, bounded queues and snapshot/restore of pending events.
- No async runtime or thread dependency in scenario crates.

### `ref/abides`

**Verified evidence**

- BSD-3-Clause agent-based market simulation.
- Separates the kernel, exchange, agents, messages, configurable latency and scenario configuration.

**Bunting usage**

- Behavioral and model reference for `agent-models`, `scenario-engine` and scenario fixtures.
- Useful sources include the kernel, exchange/order-book, trading/value/noise agents and scenario configurations.

**Vendoring decision**

- No Python runtime dependency.
- Agent formulas and configuration ideas may be reimplemented with BSD attribution after confirming the exact source path and model assumptions.

**Verification gate**

- Agent outputs are intents only.
- Independent random streams and deterministic schedule order.
- Latency, if added, is an explicit scheduled parameter rather than wall-clock delay.
- Distributional tests state tolerances and do not claim exact ABIDES equivalence.

### `ref/quickfixj`

**Verified evidence**

- Mature Java FIX engine under the QuickFIX Software License 1.0 with attribution and naming conditions.
- Extensive session, reset, connector and sequence-number tests.

**Bunting usage**

- Independent conformance oracle for `simfix-session`, `simfix-wire` and `clients/fix-bridge`.

**Vendoring decision**

- No Java production dependency and no copied session engine.
- Use a CI harness to exchange messages and capture Bunting-owned conformance fixtures.

**Verification gate**

- Logon/logout, heartbeat/TestRequest, low/high sequence, ResendRequest, SequenceReset gap fill, reset, PossDupFlag, OrigSendingTime and reconnect.
- Preserve required attribution in any redistributed harness documentation.

### `ref/market-maker-rs`

**Verified evidence**

- MIT Rust library with pure strategy modules, `rust_decimal`, risk/execution/backtest layers and optional runtime/API features.
- It currently depends on OrderBook-rs even with default features, so importing the whole crate would pull an unsuitable order-book/runtime model into the strategy layer.

**Bunting usage**

- Selective source and test donor for `crates/market-making` and `crates/order-reconciliation`.
- High-value areas: Avellaneda–Stoikov/GLFT formula decomposition, strategy interface, risk separation, fill models and connector boundaries.

**Vendoring decision**

- No whole-crate dependency.
- Selected pure MIT functions and tests may be adapted after removing unrelated dependencies and converting outputs through Bunting's tick/lot boundary.

**Verification gate**

- Independent published/golden formula vectors.
- Parameter-domain and finite-value checks.
- Side-aware rounding, inventory skew, stale-data halt and quote reconciliation.
- No strategy module owns authoritative position or fill state.

### `ref/quarcc-trading-engine`

**Verified evidence**

- Contains order manager, execution gateway/handler, journal/order store, local/external identifier mapping, deferred fill handling, position keeper, kill switch and unit tests.
- Also contains threads, mutexes, callbacks, gRPC, native SQLite and floating-point boundaries that do not fit the Worker kernel.

**Bunting usage**

- Behavior-only source for `market-events`, `order-reconciliation`, `ledger`, `risk-engine`, native adapters and test fixtures.

**Vendoring decision**

- Blocked until ownership/license is documented.
- Even after resolution, port state transitions and fixtures rather than thread/service architecture.

**Verification gate**

- Complete lifecycle transition table, local/external ID collision rules, out-of-order fill reconciliation, position projection and kill-switch behavior.

### `ref/nbc_engine`

**Verified evidence**

- Contains five JSON scenario families with exact seeds, step intervals, market parameters and agent parameter sets.
- A complete reviewed Java source/license set has not been established.

**Bunting usage**

- Data/provenance source for `scenarios/nbc`, `scenario-schema`, `agent-models` and compatibility tests.
- The verified source catalog is in `docs/ports/nbc-scenario-catalog.md`.

**Vendoring decision**

- Do not copy Java binaries or source.
- Do not publish duplicated scenario JSON as canonical Bunting scenarios until ownership/license and unit semantics are resolved.
- Preserve source blob hashes and transcribe reviewed values into a versioned Bunting schema with explicit provenance.

**Verification gate**

- Field-by-field transcription review, exact decimal/unit conversion and schema validation.
- Each reconstructed agent formula identifies whether it came from source, literature or an explicit Bunting redesign.

### `ref/ritc_mm`

**Verified evidence**

- Monolithic Rust RIT client/strategy with REST models, blocking transport, wall-clock polling, Avellaneda–Stoikov, GARCH, FFT, queue analysis, quote reconciliation and risk constants.
- Also contains scenario JSON blobs identical to those under `ref/nbc_engine` for the verified five files.
- No confirmed license has been recorded.

**Bunting usage**

- Behavior and API inventory for `clients/ritc-adapter`, `crates/market-making` and `crates/order-reconciliation`.

**Vendoring decision**

- Blocked from source copying until licensing is resolved.
- Reimplement behavior from documented formulas, independently verified vectors and MIT comparison sources.

**Verification gate**

- Recorded RIT payload fixtures, desired/live quote state machine, partial-fill/reconnect behavior, exact tick/lot conversion and strategy golden vectors.

## Current approved dependency set

At this stage only the official Cloudflare `worker` packages are approved for production use from the reference set. All other references are either:

- a dependency spike (`ironfix` pure codec crates, `wirefilter-engine`);
- an external integration contract (`nautilus-trader`);
- a selective source candidate (`market-maker-rs`, narrowly scoped IronFix/OrderBook-rs material);
- or an oracle/data source.

Changing a disposition in this document requires a PR that includes the relevant build, license, size, determinism and security evidence.