# Core-first Rust market simulator: development plan

Status: **proposed implementation sequence**, 2026-10-07. No code changes or tests are claimed by this plan.
Baseline: `main@a35f18490283c0865120b5bb66cb1132a36e5c00`.
Evidence and known defects: [`../core-engine-status-2026-10-07.md`](../core-engine-status-2026-10-07.md).

## Decision

Build **one correct, deterministic, performance-conscious, host-neutral Rust market simulation engine**. RIT replacement features, QUARCC-hosted competitions, FIX/WebSocket/REST, TUI/GPUI, browser publication and SDKs are consumers that must be layered on this core. Until the engine proves five-day cross-venue correctness and recovery, **pause broad UI, product-catalog and platform expansion** except for focused fixtures/adapters needed to test the core.

The existing accepted [ADR 0018](../adr/0018-unified-bunting-engine.md), [ADR 0019](../adr/0019-bunting-engine-package-owns-orderbook-rs.md), [ADR 0022](../adr/0022-native-competition-venue-and-publication-worker.md), [ADR 0024](../adr/0024-discrete-matching-interval-fairness.md), [ADR 0025](../adr/0025-run-archive-and-replay-verification.md) and [ADR 0027](../adr/0027-wasmer-wasi-server-runtime.md) remain in force. This document is not permission to swap OrderBook-rs, install a second production matcher, bypass durable commits or replace Wasmer as the supported runtime. A performance-motivated runtime/durability redesign needs an explicit ADR and evidence.

## 1. Target core boundary

```text
External FIX / native client / browser adapter / built-in actor (non-authoritative)
                |
       validate identity + bounded intent
                |
     deterministic admission / arrival sequence
                |
          ONE RUST RUN AUTHORITY
   +----------------------------------------+
   | logical clock + session calendar       |
   | immutable scenario + seeded schedules  |
   | instrument registry                    |
   |   venue A: listing books               |
   |   venue B: listing books               |
   |   venue C: listing books               |
   | unified cash/position/ledger + risk    |
   | fills / private & public event facts   |
   | deterministic ordered transaction      |
   +----------------------------------------+
          |                     |
   durable WAL/events      committed projections
   + checkpoints           -> FIX/WS/UI/archive
          |
   restart/replay/state hash/score
```

**Core owns:** run, scenarios, logical time, session/calendar, `ListingKey` book state, matching, trades, order lifecycle, cross-venue participant portfolio, funds/inventory/positions/reservations, risk admission, immutable market facts, deterministic agents' scheduled inputs, mark/settlement policy, snapshot hash and replay.

**Core does not own:** sockets, FIX protocol state, UIs, Cloudflare caches, browser request handling, language bindings, database driver, host scheduling threads, credentials or arbitrary user strategy processes. Participant QUARCC execution/OMS and all built-in strategy implementations remain clients of ordinary authenticated engine commands; they do not obtain mutable matcher or ledger handles.

Keep first-party reusable packages (`market-types`, `market-events`, `ledger`, `risk-engine`) when their responsibilities are clear. `bunting-engine` alone owns market authority and privately uses the existing version-pinned matching adapter. Don't create placeholder crates, parallel matcher kernels or a second event source.

### Required authoritative model (target, not current API)

- **Run:** `RunId`, pinned scenario/policy/engine/schema versions and RNG roots; one monotonic committed command sequence plus canonical event sequence; run lifecycle distinct from trading-session lifecycle.
- **Economic instrument:** `InstrumentId`, contract multiplier, quote/settlement currency, product and dependency metadata.
- **Listing:** `ListingKey { venue_id, instrument_id }`, tick/lot/fee rules, independent book, status/calendar, venue-specific order/trade/market-data cursor and marks. Cross-listed venues do not auto-match.
- **Participant:** verified identity, balances/positions/reservations keyed to instruments, exposure across all listings, entitlements and risk limits. No duplicate authoritative accounting store.
- **Clock/calendar:** versioned ordered logical events (session open/close, auction/continuous, order expiry, news, fees, mark, settlement, overnight). No wall-clock input determines replay semantics.
- **Match output:** exact maker/taker identities, order IDs, `ListingKey`, price, quantity, fees, event cursor and all financial postings required for both parties.
- **Journal/checkpoint:** durable versioned command/result/event stream and occasional complete engine snapshot (books, clock, ledger, agent RNG/schedules, expiries, markets, risk, versions and hashes). Transport sessions may checkpoint separately.

Suggested functional Rust contract (illustration only, not a signature to copy verbatim):

```rust
fn apply(&mut self, command: AuthorizedCommand) -> Result<StagedTransition, EngineError>;
fn commit(&mut self, staged: StagedTransition) -> Result<CommittedBatch, EngineError>;
fn snapshot(&self) -> Result<EngineSnapshot, SnapshotError>;
fn restore(snapshot: EngineSnapshot, tail: &[CommittedCommand]) -> Result<Self, ReplayError>;
```

The actual design must ensure **no publication or success acknowledgement precedes durable commit**; an in-memory commit is not sufficient for the hosted competition durability contract. Prefer one authoritative writer per run, optionally many runs in parallel, with read-only snapshots for subscribers. Design transaction rollback from touched-resource staging, not from copying every book/participant on each message.

## 2. Non-negotiable invariants

1. **One source of financial truth:** cash, inventory, reservation, cost basis, fee, mark, P&L, risk and NLV reconcile with executed fills; every debit/credit and acquisition/disposition is exact and explainable. No double-counted portfolio state.
2. **Complete deterministic replay:** same pinned scenario, named RNG seeds, ordered external intents, scheduler/calendar decisions and engine version imply byte-equivalent canonical events and final hash across restore paths.
3. **Explicit venue separation:** different listings of one instrument are independently addressable and have independent order priority, depth, trade history and session policy; only explicitly defined consolidated views aggregate them.
4. **Atomic matching and settlement:** orders either commit both parties' fills, fees and reservations with book state and events, or commit none. Rejection, overflow and disk failure never create phantom exposure.
5. **Authorization and privacy:** only the verified actor may trade/manage their permitted participant; public event projections reveal no private participant/account data; all agents and participant strategies use normal admission.
6. **Logical time over days:** session transitions and corporate/financing actions have deterministic priorities; overnight state carries according to the policy; canceled/expired orders cannot revive after restart.
7. **Bounded resource consumption:** finite message, order, depth, queue, event, history, journal, snapshot and agent cascade limits are explicit; bounds are observable and do not silently mutate market semantics.
8. **Commit before publish:** acknowledged result, participant fills, market streams and score/report projections derive from one durable committed sequence. Duplicate/retry returns original result without re-execution.
9. **No premature realism claims:** seeded simulation agents submit ordinary orders; market paths are outcomes, not hidden price overrides. RIT-equivalence and stylized-fact claims need fixtures/data.
10. **Single matching foundation:** preserve released OrderBook-rs behind `bunting-engine` unless a measured, separately reviewed ADR authorizes a change.

## 3. Dependency-ordered milestones and exit gates

### M0 — Freeze baseline and make the core observable

Scope: documentation/fixtures/tests/profiling only; no new participant product features.

- Capture current `main` golden state/event hashes for simple limit/market/cancel, two listing isolation, snapshot/replay, rejected command and fine/score paths.
- Add **failing first** integration cases for trade→portfolio NLV, maker/taker reports, same instrument/two venues, multi-day expiry/settlement, replay after kill/restart and a full participant roster in score.
- Add a benchmark harness for engine-only latency/allocations (no sockets, no disk) and for durable native/WASI submission and recovery. Use repeatable inputs and fixed release flags; preserve results with platform/hardware metadata.
- Inspect `main` CI/toolchain failures separately from engine regression. Don't label code “green” from a failed job that never reached tests.

**Gate:** reproducible baseline and every known correctness gap represented by a test; benchmark protocol committed. Red tests are acceptable **only before** their corresponding repair, not at release.

### M1 — One financial truth and correct market commands (highest priority)

Primary source: `packages/bunting-engine/src/{lib.rs,simulation.rs}`, `packages/ledger/src/lib.rs`, `packages/market-events/src/lib.rs`, `packages/risk-engine/src/lib.rs`.

- Introduce a versioned exact trade/fill/fee posting contract that updates a single authoritative cash/position ledger. Fix NLV/cost basis/realized P&L/marks from that same truth; treat any old projection as derived and verified.
- Reconcile risk reservations before and after partial fills, cancel, STP, market remainder, expiry and session close.
- Use every enrolled participant in score/report generation, including inactive teams. Define final-marks policy and funding/fee convention; preserve `bunting.score.nlv-rank.v1` only when its semantics are implemented exactly.
- Correct `ListingKey` throughout order-entry, fill events, per-venue ownership/market data and relevant client mappings; version schema and build migrations. Do not implement consolidated routing logic as a side effect.
- Mark incomplete composite/OTC/product actions explicitly unsupported until they genuinely match, settle and replay.

**Gate:** crossed-maker/taker, multi-listing, partial-fill, mark, portfolio accounting and post-restart score tests pass with exact identities/conservation; no duplicate mutable ledger truth.

### M2 — Fast live Rust state with bounded durable recovery

This is a potentially binding architecture change: **draft and accept an ADR before** altering snapshot, commit or runtime semantics.

- Use long-lived in-memory `RunState` and private `KernelBook` instances for routine commands, avoiding snapshot reconstruction on each message.
- Replace whole-run clone/whole-file rewrite with transaction-safe touched-entity staging, an append-only sequential durable journal/WAL and configurable checkpoint cadence. Choose journaling and group-commit policy using crash-atomicity analysis.
- Preserve exact idempotency keys, command/rejection results, expected version and commit-before-ack semantics.
- Keep published book data as derived bounded projections; a materialized public depth snapshot must never become authoritative state.
- Test power-loss stages, partial writes, torn/corrupt tails, duplicate retry, checkpoint version migration and exact recovery. Compare native and pinned WASI behavior; do not silently replace ADR 0027.

**Gate:** validated replay after injected failure at all externally meaningful commit points; no per-command whole-file JSON save, no per-command book restore, no unexplained allocations proportional to total historical events; benchmark results committed with regressions and limits.

### M3 — Complete multi-venue market mechanics

- Support at least two venues trading the **same** instrument and one venue with additional instruments; explicit routing and independent fee/tick/lot/halts/order priority.
- Expose per-listing L1/L2/trades/history and a derived consolidated BBO; one venue's cancel/halt/auction must not mutate another venue's resting book.
- Establish consolidated account risk and clearing/settlement per economic instrument while retaining venue-specific fees and execution provenance.
- Test economically meaningful cross-venue arbitrage executed as separate ordinary orders; do not add an implicit smart order router to the matcher.

**Gate:** canonical multi-venue scenario passes fill isolation, consolidated quote derivation, independent venue policy, cross-venue portfolio accounting and restart/replay with identical events/hash.

### M4 — Multi-day logical-market lifecycle

- Define versioned session calendar and phase priority: closed, pre-open (if supported), open/continuous, halt, closing, overnight. Auction behavior is disabled until implemented/tested.
- Define `DAY` vs `GTC` vs `GTD` expiration at per-venue session boundaries; handle remaining orders and cancel/expire events.
- Mark positions and carry cash/positions across days; define overnight accruals, dividends, corporate actions and financing as explicit policy-driven events. Unsupported products remain unavailable.
- Allow lockstep, paced and accelerated execution of the **same logical schedule**; wall clock affects when work is attempted, not event order. Record all externally admitted commands plus deterministic scheduler decisions.
- Include recovery/restart just before/after close and just before/after next open.

**Gate:** deterministic **five-trading-day** scenario with persistent inventory/cash, timed news, conditional halts, two venues, session expiry, overnight marks, repeated restart and identical final scores.

### M5 — Market-dynamics correctness, not cosmetic complexity

- Keep seeded agents as normal participant commands through ledger, risk and matching; persist RNG/agent state and wake ordering with checkpoints.
- Implement one demonstrably useful liquidity-provider/noise/informed-agent profile first, then extend with institutional and stress flows only when validated.
- Test spreads, queue survival, participation, arrival-size distributions, fill probability, volatility clustering and liquidity withdrawal on fixed documented scenario seeds.
- Separate **market-simulation fidelity** from **RIT behavioral parity**; each is evaluated against its own evidence. Never claim Rotman private formulas from static API metadata.

**Gate:** reproducible agent runs with independent RNG streams, no bypass of ordinary risk, no unbounded cascade; publish metric distributions and deviations across a frozen set of seeds.

### M6 — Engine-core acceptance before feature/platform expansion

Create a versioned **core release scenario**, not an invented benchmark claim:

- 3 venues, with at least 2 sharing one economic instrument;
- at least 2 distinct instruments and market phases;
- 5 consecutive simulated trading sessions, with overnight account carry;
- 50–100 active algorithmic participants as a **proposed load-test profile**, not currently demonstrated scale;
- mixed maker/taker order flow, partial fills, expiration, news/stress and venue halt/resume;
- pause, forced termination and repeated process-kill/restart checks.

Acceptance must compare every maker/taker execution, fee, mark, cash balance, position, book, venue quote, event sequence, final ledger and score to deterministic replay. Measure p50/p95/p99 match time, durable-ack latency, message throughput, queue depth, allocations, resident memory, bytes written/command and restart recovery time. Fix budgets from measured baseline and event requirements; do not invent an orders-per-second number in documentation.

**Gate:** one documented reproducible script exercises the complete scenario natively and under the currently supported server runtime; no financial mismatch, privacy breach, lost fill or non-deterministic replay, and a published capacity ceiling established by the test.

### M7 — Build products on the accepted core

Only after M6, expand in focused vertical slices:
- **QUARCC hackathon:** authenticated teams, complete FIX market subscriptions/private maker and taker fills, reconnect/reset, bounded fairness, practice venue, tournament scheduling, scoring and independent archive verification.
- **RIT replacement:** instructor case configuration, terminals, reports, tender and OTC life cycles, products/derivatives, casefile import and optional REST/RTD compatibility. Every workflow uses the same engine and has a traceable end-to-end test.
- **UI and deployment:** Ratatui/GPUI, hosted publication and language SDKs consume authoritative projections and never host a second matching engine. Review draft [PR #19](https://github.com/andrewkoumoudjian/bunting/pull/19) and [PR #20](https://github.com/andrewkoumoudjian/bunting/pull/20) for dependency order, not automatic merger.

## 4. Engineering rules during this program

- **Small changes, evidence first.** Each milestone lands in separate reviewable PRs: one invariant/contract plus failing test, then implementation, then regression and migration evidence. Do not combine schema, storage, runtime and UI rewrites.
- **Stable compatibility.** Version public commands/events, scenario policies, snapshots and archive readers when semantics change. Golden hashes may intentionally change only with a documented version migration; never silently overwrite archived market history.
- **Measured optimization.** Profile native engine versus hosted runtime and matching, accounting, journal and fan-out separately. Remove large copies and redundant serialization before experimenting with alternative matching structures, threading or unsafe code.
- **No speculative new packages.** Keep module boundaries in `bunting-engine` until a proven second consumer warrants a reusable crate; don't create synthetic “protocol/common/algorithms” buckets.
- **Host-neutral engine.** Reusable Rust packages cannot depend on Cloudflare, Wasmer, filesystem, sockets or desktop frameworks. Cross-compile correctness is useful, but native execution should be benchmarked; **replacing** the ADR-0027 Wasmer production target requires a new ADR.
- **Source-backed compatibility.** RIT/NBC port records remain evidence, not authority. Respect `docs/reference-functionality-audit.md`, `docs/reference-adoption.md` and the `docs/ports/` provenance/license rules.
- **Operational integrity.** Persist private full archives with access control; serve audience-redacted projections. A replayable archive proves financial results independently of live UI state.

## 5. First concrete development PRs

1. **`test/core-accounting-venue-regressions`:** add end-to-end crossing trade→account/score tests (expected red), cross-listed routing tests (expected red), maker-side fill delivery test (expected red), and baseline benchmark harness without modifying production behavior.
2. **`fix/single-ledger-and-listing-identity`:** unify postings and marks, add explicit venue identities, fix score roster inclusion, migrate canonical schema/snapshots, and green the corresponding tests.
3. **`design/core-transaction-wal-adr`:** evaluate native live-state/WAL/checkpoint architecture, durability and compatible recovery. Include benchmark and fault model. No implicit supersession of an accepted ADR.
4. **`impl/core-live-state-journal`:** implement the accepted transaction design and demonstrate exact crash/replay equivalence plus improved bounded per-command work.
5. **`feat/core-multivenue-multiday`:** implement venue-specific market data, calendar/rollover semantics, exact overnight ledger behavior and the five-day golden fixture.
6. **`test/core-simulation-acceptance`:** accelerated deterministic soak, concurrent admissions, fault/restart, replay and performance/capacity report. Promote the engine only after it passes.

The first PR is **tests and an evidence ledger**, not a new terminal, WASM runtime or agent family. Keep the planned statuses explicit until their gates are satisfied.

## Related repository contracts

- [Architecture](../architecture.md) and [product contract](../specs/bunting-product-contract.md)
- [RIT simulation requirements](../specs/rit-class-market-simulation.md) and [binary evidence](../research/rit-binary-audit/market-feature-ledger.md)
- [Competition policies](../specs/competition-policies-v1.md)
- [Competition redesign history](competition-platform-reorganization.md) and [older readiness checklist](production-readiness-plan.md)
- [NBC port/evidence](../ports/nbc-simulation.md) and [RITC market-making boundary](../ports/ritc-market-making.md)
