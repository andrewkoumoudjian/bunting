# Bunting core-engine status and gap audit (2026-10-07)

> **Historical — do not follow as instructions.** Superseded by the independent 2026-10-07 audit and `implementation-log/`; most listed defects were repaired in slices 0–10.
> Current guidance: [`AGENTS.md`](../AGENTS.md) and the [documentation status map](../docs/README.md).

Status: **source-backed assessment and proposed priorities**, not a statement that the proposed repairs are implemented.
Audited `main`: [`a35f18490283c0865120b5bb66cb1132a36e5c00`](https://github.com/andrewkoumoudjian/bunting/commit/a35f18490283c0865120b5bb66cb1132a36e5c00).
Companion execution plan: [`plans/core-first-market-simulator.md`](plans/core-first-market-simulator.md).

## Decision and product goal

**Build and validate one reliable, high-throughput, deterministic Rust market/exchange simulator first.** The RIT-class student/instructor terminal and the QUARCC-hosted algorithmic-trading hackathon are applications of this engine, not independent engine implementations. The engine must be useful and testable without a TUI, GUI, FIX socket, Cloudflare, Wasmer, a particular strategy or an online service.

Long-term products:
1. **RIT functional replacement:** configurable cases, trading sessions, multi-period decision making, authoritative account/risk/marks, news, tenders, instructors, reports, and progressively broader product and institutional workflows. An optional RIT protocol/UI compatibility layer is separate from functional parity; proprietary server formulas are not assumed.
2. **QUARCC hackathon venue:** many isolated participant identities running external algorithms against shared books; multiple venues, instruments and simulated trading days; consistent private fills and public depth; deterministic competition settlement and replay; explicit and enforceable resource/fairness limits.

This note is an engineering direction, **not a new ADR**. In particular, accepted [ADR 0018](adr/0018-unified-bunting-engine.md) and [ADR 0019](adr/0019-bunting-engine-package-owns-orderbook-rs.md) require one production `bunting-engine` around released OrderBook-rs. [ADR 0027](adr/0027-wasmer-wasi-server-runtime.md) currently selects a Wasmer-hosted WASI production server. Changing those binding choices requires an evidence-backed superseding ADR, not an undocumented refactor. Native Rust performance comparisons are recommended, but do not silently change the selected deployment.

## Evidence boundaries

- **Observed in checked-in code:** source paths and implementation mechanisms identified below. This is a source review, not fresh execution of an end-to-end venue, stress test or independently measured benchmark.
- **Existing test evidence:** unit, conformance, deterministic golden and TCP/interop tests exist; they prove their asserted scenarios only. They are not proof of five-day, multi-venue, production-load correctness.
- **Unresolved:** exact Rotman/RIT internal financial formulas, actual sustainable order throughput, latency/CPU/RSS under load, engine behavior after sustained restart/soak, and fidelity to empirical market microstructure.
- **Bunting-added recommendations:** the architecture and acceptance gates in this audit and the companion plan. Proposed numerical test sizes are test profiles, not measured capacity.

Do not rely on historical planning documents as current-state evidence without reconciling them against this commit. In particular, [`docs/research/rit-binary-audit/engine-parity-matrix.md`](research/rit-binary-audit/engine-parity-matrix.md) and [`docs/specs/rit-tui-parity-matrix.md`](specs/rit-tui-parity-matrix.md) include older status snapshots; [`plans/production-readiness-plan.md`](plans/production-readiness-plan.md) explicitly audits an earlier commit. See the source evidence below before promoting a capability to “complete.”

## What is already worth preserving

| Capability | Observed implementation | What is and is not established |
|---|---|---|
| Rust and exact boundaries | `packages/market-types`, `packages/market-events`; fixed-point prices, lots and money; checked IDs | Good foundation. Per-case valuation and settlement are not thereby complete. |
| One venue authority | `packages/bunting-engine` and private `matching.rs` dependency on `orderbook-rs = 0.10.3` | Basic limit/market/advanced matching paths, cancel and snapshot exist. Don't replace this matcher without benchmarks and ADR review. |
| Multi-listing state | `RunState.listings: BTreeMap<ListingKey, ListingState>`; scenario validations; two-listing isolation test | Distinct books exist; **cross-listed order routing and per-listing data are not complete**. |
| Simulation domain | `bunting-engine/src/simulation.rs`: logical clock, scheduled actions, account projection, news, tenders, OTC, facilities, scores | Domain types and several transitions exist; financial and temporal semantics vary in completeness. |
| Risk, ledger and canonical events | `packages/risk-engine`, `packages/ledger`, engine transitions, expected-version origin commits | Several operations are exact; end-to-end ledger consistency is not yet proven. |
| Deterministic recovery | `EngineSnapshotEnvelope`, scenario hashes, `bunting-rs/src/archive.rs`, replay/golden tests | Important invariants exist; normal-path state copying and file persistence are expensive. |
| Participant software | `quarcc-execution-engine`, built-in agents, `simfix-*`, `bunting-runtime` | Keep OMS/agents outside market authority. Full remote feed and live reconciliation need independent proof. |
| Venue and operations | `apps/bunting-server` with bounded, rostered FIX/TCP sessions and operator endpoints; Ratatui CLI | One shared market is supported for basic cases; a multi-day, multi-venue event is not established. |

### P0-A — Explicit venue selection is missing for cross-listed instruments

**Observed:** `packages/market-events/src/lib.rs::SubmitOrder` specifies `instrument_id`, not a `ListingKey` or `venue_id`. `RunState::listing_key_for_instrument` returns `AmbiguousListing` if more than one listing carries the instrument. The normal command path uses that function; `TradeExecuted` contains instrument identity but no venue identity. `SimulationState.market` and `set_depth` are keyed by `InstrumentId` rather than listing.

**Consequence:** the implementation can hold multiple isolated books but cannot safely expose a traded cross-listing instrument as two independently addressable exchange destinations. Market data, trade history and risk attribution can lose the venue dimension.

**Required repair:** use stable listing identity in all executable order commands and resulting fills, venue-specific market-data streams, snapshots, history and affected-order indexing. Keep instrument identity for consolidated portfolio exposure. Explicitly define `(venue, instrument)` market/fee/trading-status policies; derive consolidated BBO from per-venue facts without implied smart routing. Version/migrate canonical commands/events/snapshots and FIX mappings.

**Proof:** submit/cancel/trade the same economic instrument on two venues, including simultaneous differing bids/asks; verify no book/history leakage, per-venue replay and a correctly derived consolidated top of book.

Source: [`packages/bunting-engine/src/lib.rs`](../packages/bunting-engine/src/lib.rs) (`listing_key_for_instrument`, `transition`), [`packages/market-events/src/lib.rs`](../packages/market-events/src/lib.rs) (`SubmitOrder`, `TradeExecuted`), [`packages/bunting-engine/src/simulation.rs`](../packages/bunting-engine/src/simulation.rs) (`SimulationState.market`, `set_depth`).

### P0-B — The exchange and competition ledger paths diverge

**Observed:** `bunting-engine::apply_trades` calls `ledger::Ledger::settle_trade`, modifying the basic `accounts` and `holdings` projections. `SimulationState::project_event` handles `TradeExecuted` by updating market history, not `portfolio_ledger`. `SimulationState::score_iteration` and `bunting-application::competition::account` read the separate `PortfolioLedger` for NLV, P&L and cash details. No ordinary-fill bridge into the scored portfolio ledger is visible in this source path.

**Consequence:** an order can be executed while competition-scored financial state does not reflect the same transaction. This is a release-blocking integration defect until disproven by an end-to-end test. The existing `full_competition_run_matches_ledger_score_and_transcript_golden` fixture covers a fine and run advances; it does **not** exercise matched exchange fills.

**Required repair:** establish **one authoritative posting/ledger truth**, including reservations, filled cash, inventory, cost basis, realized and unrealized P&L, fees, accruals and settlement. Either refactor to one ledger representation or make one a strictly derived, invariant-checked view; never maintain two independently mutable truths. Define mark policy, price source, currency, contract multiplier, zero-fee baseline and exact rounding. Score every enrolled participant, including those with no private news/tenders/orders.

**Proof:** two participants cross orders at a price different from cost basis, including partial fill and cancel; verify conservation, reservations, book ownership, both account projections, marks, fees/fines, exact NLV and deterministic final rank. Run again after archive replay and snapshot/restart.

Source: [`packages/bunting-engine/src/lib.rs`](../packages/bunting-engine/src/lib.rs) (`apply_trades`), [`packages/ledger/src/lib.rs`](../packages/ledger/src/lib.rs) (`settle_trade`, `PortfolioLedger`), [`packages/bunting-engine/src/simulation.rs`](../packages/bunting-engine/src/simulation.rs) (`project_event`, `score_iteration`), [`packages/bunting-engine/tests/simulation_domain.rs`](../packages/bunting-engine/tests/simulation_domain.rs).

### P0-C — Durable correctness currently causes excessive per-command work

**Observed:** `RunState::transition` clones the full run state and reconstructs the relevant `KernelBook` from a snapshot; after execution, it writes a new matching snapshot and refreshes market projections, including scans over owned orders. `FileOriginStore::commit` clones its entire `FileState`, serializes it to JSON and syncs/renames the complete file on every commit. It also retains command and event history subject to bounds.

**Consequence:** command cost scales with unrelated accounts/books/orders and accumulated history; per-command serialization, copying, disk bytes and recovery overhead impede realistic long-lived markets. This is an architectural scaling risk established by code shape, **not a measured orders/sec result**.

**Required repair:** keep authorized live books and ledger in a long-lived in-memory run; stage only touched transactional entities; append accepted commands and canonical events to a durable ordered journal; checkpoint periodically; replay from the last verified checkpoint plus tail. Preserve commit-before-ack, duplicate suppression, failure atomicity and snapshot format compatibility. WAL technology and fsync/group-commit policy must be justified by measurement and written as an ADR before changing durability guarantees.

**Proof:** no full-run clone, book deserialization or complete history-file rewrite on the normal order path; deterministic fault-injection tests around every durability/ack boundary, state hash equality on restart, and measured amortized per-command CPU/allocations/disk bytes.

Source: [`packages/bunting-engine/src/lib.rs`](../packages/bunting-engine/src/lib.rs) (`transition`, `restore_book`, `replace_snapshot`, `refresh_market_projection`), [`apps/bunting-server/src/storage.rs`](../apps/bunting-server/src/storage.rs) (`FileOriginStore::commit`, `persist`).

### P0-D — Venue event fan-out and participant fills are incomplete

**Observed:** `apps/bunting-server/src/session_host.rs` processes a message on its socket, executes a command and sends `committed_messages` on that same connection. Market-data requests return snapshots. No corresponding shared native-server subscriber fan-out is visible in that path, including an unsolicited private maker fill when another participant crosses the maker's order.

**Consequence:** remote market makers may see stale books and miss execution reports for resting orders. FIX session recovery alone does not provide a complete committed market-data subscription.

**Required repair:** let the core emit a sequenced, typed committed-event batch with public per-listing and private per-participant projections. A non-authoritative transport fan-out layer delivers updates to all authorized subscribers, including both trade counterparties; support initial snapshot, cursor catch-up, reset, bounded backpressure and reconnect. No raw private event envelope may appear on a public feed.

**Proof:** concurrent authenticated clients observe equal published venue depth after each match; resting maker and aggressor each receive their own fill; slow/disconnected clients catch up or explicitly reset without changing market truth.

Source: [`apps/bunting-server/src/session_host.rs`](../apps/bunting-server/src/session_host.rs), [`packages/bunting-application/src/lib.rs`](../packages/bunting-application/src/lib.rs), [ADR 0023](adr/0023-concurrent-participant-fix-sessions.md).

### P0-E — Logical time does not yet implement multi-day market sessions

**Observed:** the simulation uses `LogicalClock { now, step_ns, mode }`, `RunLifecycle` and scheduled actions. `Advance` adds step duration and applies actions due by a cutoff. There is no complete trading-day/calendar transition contract for venue-specific open/close, pre-open auctions, DAY expiry, overnight carry, marks or close/settlement; the scheduled-action `ExerciseOption` / `Deliver` arms are currently empty.

**Consequence:** advancing logical time over several days does not by itself create realistic five-day exchange operation. Resting orders, order permissions, marks, rollovers and cashflows lack a single authoritative market-session boundary.

**Required repair:** version a simulation calendar (timezone/calendar rules, date and session phase per venue), schedule deterministic open/close/rollover transitions, specify GTC/DAY/GTD behavior, preserve permitted positions and cash overnight, and apply marked/settled corporate actions on the pinned logical timeline. Separate **run** termination from **session/day** close.

**Proof:** replay a five-day scenario with venue calendars, day orders, resting GTC orders, overnight holdings, scheduled news, halts, settlements, and restart across each boundary; hash and score remain identical.

Source: [`packages/bunting-engine/src/simulation.rs`](../packages/bunting-engine/src/simulation.rs) (`LogicalClock`, `advance`, `apply_scheduled`), [`packages/market-events/src/lib.rs`](../packages/market-events/src/lib.rs) (`TimeInForcePolicy`).

### P0-F — Some financial workflows are only states or events, not executed markets

**Observed:** `SimulationState::apply_composite` validates legs and returns `CompositeCompleted` without matching or settling them. OTC accept/counter/break currently mutate negotiation state; the complete trade/credit/settlement path is not supplied by that transition. The initial tender and score policies are explicitly narrow Bunting-defined competition policies; a broader set of product/facility lifecycle semantics is not proven.

**Consequence:** a broad advertised RIT-class action surface cannot be treated as end-to-end financial functionality, and scoring cannot rely on semantic placeholders.

**Required repair:** mark incomplete actions unavailable in participant capabilities until a complete state transition includes matching/contract formation, reservations, posting, risk, replay and reporting. Finish these only **after** the core CLOB and ledger are correct. Keep exact Rotman compatibility claims behind actual evidence, not assumptions.

**Proof:** every available participant action changes authoritative financial state or returns an explicit unsupported result; each exposed feature has a full replayable vertical test.

Source: [`packages/bunting-engine/src/simulation.rs`](../packages/bunting-engine/src/simulation.rs) (`apply_composite`, OTC transitions), [`specs/competition-policies-v1.md`](specs/competition-policies-v1.md), [`specs/rit-class-market-simulation.md`](specs/rit-class-market-simulation.md).

## Additional production uncertainties to resolve after P0

- **Ingress fairness:** [`apps/bunting-server/src/writer.rs`](../apps/bunting-server/src/writer.rs) assigns arrivals and sleeps each submitted command to an interval boundary; [ADR 0024](adr/0024-discrete-matching-interval-fairness.md) describes shared interval batches. Verify exact arrival/boundary recording, uniform release timing, deterministic ordering and replay under concurrent clients; don't assume a correct batch abstraction merely because a test checks arrival order.
- **Rate/open-order accounting:** [`session_host.rs`](../apps/bunting-server/src/session_host.rs) keeps a connection-local `open_orders` set, updated for local accept/cancel paths. It needs authoritative maker-fill/expiry/reconnect reconciliation before it is a reliable per-participant cap.
- **Scoring and marks:** the current `bunting.score.nlv-rank.v1` policy is not Rotman-equivalent and requires a fully specified final-mark and fee convention; participant enrollment must not depend on activity in a private projection.
- **Market realism:** agent classes exist, but calibrated spread, queue survival, market impact, regime and stress benchmarks are not established by the source-reviewed implementation.
- **Server portability and test status:** [ADR 0027](adr/0027-wasmer-wasi-server-runtime.md) is binding. The audited `main` [CI run](https://github.com/andrewkoumoudjian/bunting/actions/runs/30462205477) failed during WASIX toolchain installation (Rust-version conflict), **before** tests; it does not prove engine test failure or success.
- **Branches:** the GPUI terminal [PR #19](https://github.com/andrewkoumoudjian/bunting/pull/19) and canonical terminal rollout [PR #20](https://github.com/andrewkoumoudjian/bunting/pull/20) are unmerged. Do not attribute their capabilities to `main`.

## Release judgment

At `a35f184`, Bunting has a substantial **engine foundation**, not a completed RIT replacement or production-ready multiple-venue/multiple-day venue. Core financial correctness, explicit listing semantics, deterministic financial lifecycle, efficient persistence, and end-to-end replay must be resolved **before** feature expansion or a QUARCC event depends on its economic results.

Next: follow [`plans/core-first-market-simulator.md`](plans/core-first-market-simulator.md). Treat each milestone's tests and measured evidence as the exit gate, not the presence of data structures or documentation.
