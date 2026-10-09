# Evidence-led core engine roadmap — 2026-10-07

> **Status update 2026-10-09:** slice definitions remain current; the
> execution **order** and several decisions changed. Follow
> [the exploration note §8](../research/2026-10-09-exploration-and-next-steps.md#8-owner-decisions-2026-10-09-and-revised-plan)
> for ordering, ADR 0029 for matching (OrderBook-rs is a dev-only oracle), ADR
> 0030 for fairness (replaces the interval questions below) and ADR 0033 for
> hosting. Slices 0 and 1 are done per `../implementation-log/`.

Status: **proposed development roadmap**, documentation only. Audited source: main@0fdbd130a59b212ac6cae1d3b78000acd94fb9b2; no code or tests changed in this handoff. The [independent architecture audit](../research/2026-10-07-independent-core-architecture-audit.md) gives the mechanism, alternatives, exact source pointers, external literature and limits. [Proposed ADR 0028](../adr/0028-proposed-headless-run-authority.md) identifies changes requiring architecture acceptance.

**Agent and market-flow research (2026-10-08):** [source-pinned Rust/C++ implementation comparison, academic models, observed agent gaps and integration gates](../research/2026-10-08-agent-market-simulation-implementations.md). This is a proposed Slice 5 research input, not an accepted architecture change or implemented realism claim.

The [expanded market and agent algorithm survey](../research/2026-10-08-expanded-market-algorithm-survey.md) provides a proposed research matrix covering exchange allocation, auction clearing, queue-reactive/Hawkes flow, adaptive trading agents, execution and cross-venue routing. This is documentation only and does **not** modify accepted venue matching or admission batching policies.

## Objective and release definition

Release a **headless reusable Rust simulation engine** that owns matching, orders, economic posting, agent and calendar time, deterministic replay and market data. It must operate in-process with no TUI, FIX, Cloudflare, Wasmer, internet access or system wall-clock in the transition logic.

Two optional products subsequently use that engine: a RIT-class instructor/student case application and a QUARCC competition service. Neither may implement a shadow venue book, participant ledger or score.

The acceptance scenario is a **proposal**: three independently configured exchanges; two instruments including one cross-listing; five trading days; 50–100 interacting algorithmic participants plus synthetic traffic; at least one mid-run restart; concurrent inbound activity; verifiable final per-account portfolio and score. Its utility is breadth of economic cases, **not a claim of 100-agent scale or a throughput requirement**.

## Target boundaries

~~~text
                        INGRESS (outside engine)
 FIX / local API / browser / test / batch agent / external command
               authenticate, normalize, rate limit
                              |
                              v
             Run Admission / Sequencing Contract
      ordered commands + logical schedule + durable inputs
                              |
                              v
     +---------------- HEADLESS RUST RUN OWNER ----------------+
     | Canonical run, version, logical clock, RNG/state         |
     | Instruments (economic ID) / Listings (venue + ID)       |
     | Live CLOB per ListingKey / order ownership/index         |
     | Risk, reservations, ONE posting journal/position ledger  |
     | Calendar/session/auctions/halts/expiry/settlement        |
     | Agents and scheduled exogenous actions as commands      |
     | Atomic transition: validate -> match -> post -> commit   |
     +-------------------------|------------------------------+
                               |
             durable commit record / periodic checkpoints
                               |
                 committed immutable event envelope
              +----------------+----------------+
              |                |                |
      public market data  private exec/order    archive / score
      per ListingKey       participant-scoped    independent replay
              |                |                |
             transport projections and applications (never owners)
~~~

**Ownership:** a RunId partitions an independent atomic economic domain. ListingKey selects book/venue policy. InstrumentId identifies a fungible underlying instrument in the consolidated participant portfolio; model separate fungibility/settlement conversion explicitly when markets differ. OrderId uniquely identifies an owned order (with public alias if required). No foreign crate can mutate internal books directly. Runtime agents only produce ordinary participant commands. Independent runs can execute on separate worker threads/processes. Within one run, a single deterministic mutation order is the initial design, with sharding only after a justified experiment.

**Truth:** an immutable committed command/event stream and durable recovery state agree with the current mutable in-memory RunState; ledger journal has exclusive authority for economic postings. Books have exclusive authority for queue priority. Derived holdings, cash views, P&L, score, depth and subscriptions are projections, not competing stores.

**Time:** durable logical instant and ordered intra-instant phase priority determine every transition; wall clock and network time enter only via recorded admission decisions, never hidden calls in the deterministic transition.

## Core invariants to implement alongside features

1. **Economic conservation and ownership.** Each executed fill posts buyer/seller inventory and per-currency cash/fees as one atomic unit. No rejected or undurable operation changes any observable state; each cash journal balances to clearing/exchange accounts. Reserves never exceed permitted buying power/inventory after risk policies, and release exactly once on reduce/cancel/expiry.
2. **Identity and venue separation.** ListingKey is required for book-specific commands, events, quotes, halts and auction actions. Two listings of one InstrumentId have separate queues, tick/lot rules, matching prices, market hours and depth. Participant economic positions in the same fungible instrument aggregate by instrument, not book; nonfungible contracts must have different instrument IDs.
3. **Sequence and idempotency.** Every accepted or canonical rejection carries deterministic run/version, command ID, ingress sequence, logical time, effective ordering and event sequence. Identical retry returns exactly the same result without any second posting or order; altered payload under an existing idempotency key is rejected.
4. **Commit-before-observe.** Durable acknowledged command, economic journal, RNG/scheduler state and resulting event batch are all recoverable at the same sequence. No stream or FIX response claiming execution can precede the durability boundary. After process or host failure, restore is either a committed prefix or a declared fatal corruption, never silent state divergence.
5. **Replay.** A frozen scenario, engine version and complete ordered inputs (human, agent, admin, calendar, shock) produce identical fills, books, balances, event bytes, scores and hashes; replay from checkpoint and genesis is equivalent. Avoid nondeterministic float rounding in financial state; stable traversal and RNG streams.
6. **Session economics.** DAY/GTD expiry, carried GTC orders, overnight inventory, accruals, settlement and relevant corporate actions occur exactly at published logical instants. Closed/auction/halted policies reject or queue commands per venue rules.
7. **Information isolation.** Private reports and account state go exclusively to authorized owner; public projection is nonidentifying and sequenced. Drop/reconnect can't cancel resting orders by accident; bounded slow subscribers recover from cursor.

Write one or two directly relevant tests for each new invariant as implementation changes land. Do not put a separate all-purpose testing-only milestone ahead of repairs.

## Dependency-ordered implementation slices

### Slice 0 — Contract decision and first vertical economic repair (highest priority)

**Goal:** a filled limit order yields one reconciled economic result and a recoverable score, independent of UI.

- Choose and version the minimum shared ListingKey order contract alongside the unified economic posting types *before* extending cases. Publish migration semantics for current single-listing commands (e.g. temporary compatibility adapter that resolves only unique listing, rejects ambiguity); never silently route cross-listings.
- Design one posting transaction: trade sides, fee schedule, signed positions, exact per-currency balances, reservations, lot/cost-basis convention, P&L and mark policy; decide shorting and margin policy explicitly. Keep deterministic order across cross-listing fees and price conversions. Prefer balanced journal and views derived from it, but verify whether a simpler account state can provide the same verifiable invariants without duplicated authority.
- Replace direct trade-to-Ledger + disconnected PortfolioLedger accounting; route normal fills, tenders, OTC (once implemented), fines, carry and facilities through one mutation contract. Write visible economic outcome for matching, and price marks for valuation; do not use an implicit zero mark for a missing price.
- Include all scenario participants in score roster (including those with zero orders). Version scoring and distinguish realized, unrealized and fees; never claim exact Rotman formulas.
- Preserve current orderbook-rs matching behavior while addressing accounting. Upstream matcher evaluation is separate from economic correctness.
- Minimal tests **with the implementation**: limit/market match + fee then recompute NLV from postings; buyer/seller conservation; rejected fill no mutation; duplicate order/no double settlement; short-availability/risk behavior; explicit cross-listed book routing.

**Exit:** one test-driven end-to-end trade changes order states, basic account view, competition account and final NLV consistently; independent recomputation of all balances matches journal; same instrument on two venues can be explicitly selected. **No deployment change needed.**

### Slice 1 — Listing semantics throughout core, public market data and policy

- Carry ListingKey through SubmitOrder, cancellation lookup, internal fill and booking reference, trade record, level updates, per-venue instrument policy and adapter contracts; keep global InstrumentId only for explicitly consolidated views. Version schema, including backward-compatible decoding only when destination is unique.
- Compute venue-specific BBO/L2/tape; publish separate optional consolidated NBBO/last/volume with timestamp and tie policy. Avoid merging books into one queue. Halts/collars/tick/lot and auction state are listing scoped; risk may additionally enforce aggregate instrument/portfolio limits.
- Implement structured common order behavior first: price/time FIFO according to chosen venue policy, market, cancel, IOC/FOK, DAY/GTC/GTD. Implement modify/replace only with explicit queue priority rules.
- Add maker-by-upstream-ID and participant-active-order indexes instead of scanning ownership across all orders. Keep indexes within same atomic run transition or cheaply rebuilt on restore.
- Integration tests: two venues same InstrumentId with opposite queues, venue-specific halt and order, cross-listing portfolio aggregation, independent books and correct NBBO; expired order cannot match.

**Exit:** order command, market observation and fill record unambiguously identify a listing, while portfolio reports identify the economic instrument.

### Slice 2 — Persistent live state, atomic commit and recovery

**Design experiments before final storage selection, but don't delay the known performance repair.**

- Change execution from RunState clone + snapshot-restoration for every command to mutable in-memory live books and indexed economic state. Define a controlled transaction protocol: stage a compact change set or undo record, validate all invariants, durably commit the canonical record, then expose the result. On disk failure, restore previous in-memory state or enter a non-serving recover-required state. No partially visible matching.
- Keep snapshots for checkpoint/restart, not as command-call input. Version snapshots including book queue priority, fees, ownership indices (or their deterministic rebuild rules), journal balance, scenario, logical scheduler, RNG streams and policy hashes. Checkpoint asynchronously only if it cannot race the acknowledged prefix.
- Measure two durable store candidates under representative workloads: (A) SQLite WAL, foreign keys/unique idempotency constraints, synchronous=FULL, bounded checkpoint; (B) length-prefixed checksummed append-only journal plus periodic atomic checkpoint and indexed idempotency. Reject whole-file JSON rewrite as a production hot-path default once measured.
- Storage commit record must couple command, ingress order/time, canonical events, journal postings, before/after state/version hashes or equivalent proof. Atomic commit must uphold idempotency and expected-version semantics; never acknowledge a record only fsynced to an arbitrary temporary path without a tested crash protocol.
- CPU/memory improvements should be trace-driven: avoid full projection rebuild, avoid whole-ledger clone for every journal post, preallocate bounded hot-path vectors when evidence warrants, preserve stable iteration during hashing. No premature lock-free conversion.

**Exit:** genesis and mid-run-checkpoint replay yield byte-identical economic outputs after process kill/restart; disk-full and injected crash before/after commit don't lose an acknowledged command or double-apply a retry; p50/p99 CPU and storage write bytes improve over baseline at matched workload without changing semantics.

### Slice 3 — Authoritative dispatch and competition admission

- Remove connection-local open_orders as the authority. Enforce per-participant active-order counts from the committed owner index and give reconnecting clients a versioned snapshot + committed private delta cursor.
- Implement a bounded run event distributor: per-ListingKey public quote/trade channel, per-participant private order/fill/position channel, and instructor channel. On a maker fill, both maker and taker receive unsolicited, correctly sequenced reports after commit. Slow-consumer overflow produces an explicit gap/catch-up path, never unbounded buffering.
- Separate *arrival timestamp*, *admission acceptance*, *sequenced processing* and *release/market clearing*. If competition policy is interval-based, build an actual bounded shared queue sealed at each interval; persist interval ID, command arrival sequence, ordering algorithm, RNG if used, and executed batch. Do not conflate fixed sleep and batch fairness. FCFS with fixed release is deterministic but not automatically network-latency neutral; if true batching/auction is desired, specify price and priority separately.
- Keep FIX session idempotent and resynchronizable (session sequence != market sequence); test two connections, passive maker, disconnect/reconnect, slow and malformed clients, permission isolation and cancellation races.

**Exit:** a maker resting on connection A receives fill from taker connection B without reconnect; both independently recover same owner/book state from sequence cursor. Fairness trace matches published policy and survives restart.

### Slice 4 — Multi-day venue clock and clearing lifecycle

- Introduce minimal first-class ExchangeCalendar/SessionState per venue (or per listing when justified): open/continuous/closing/closed/halted, logical day boundaries, clock and holiday calendar version, auction policy, time-zone/offset interpretation frozen into scenario; avoid dependency on host timezone.
- Implement DAY/GTD cancellation at correct edge; explicit open/close/auction price discovery if required; scheduled settlement and corporate actions by product type; outstanding overnight inventory, carry, borrow/interest, margin marks and forced-close choices. Specify price-mark source/policy on illiquid or halted instruments.
- Preserve participant portfolios and agent states across days. Ingest news/shocks as scheduled canonical facts with participant audience and release instant, not nondeterministic external fetches.
- Define end-of-day reconciliation: market trade totals, clearing cash, shares outstanding where appropriate, outstanding owned-order count, pending settlements, fees and margin.

**Exit:** one RunId simulates five days with open/close and overnight economics, correct standing orders, fees/carry and deterministic final NLV; restart mid-day and across boundary is equivalent.

### Slice 5 — Empirically credible market mechanics and agents

- Begin with simplest useful agent set: market maker with inventory-sensitive quote/cancel policy; noise/value traders with state-dependent arrival/cancel rates; institution executing parent orders; seeded exogenous information/shocks. All intentions pass ordinary matching/risk/admission; no direct rewriting of synthetic trade price.
- Calibrate parameters against permitted historical order-book/event data by instrument/session regime: arrival/cancellation hazard, passive queue replenishment, size mix, spreads, top-of-book depth, returns distribution, volatility clustering, volume curves, impact and cross-venue arbitrage response. Distinguish model-fitting targets from withheld validation days. Do not claim realistic behavior from visually plausible candles.
- Use ABIDES as architecture/experiment reference, queue-reactive research for baseline estimators and market-fragmentation papers for routing experiments. Add complexity (latency network, correlated shocks, options) only when a diagnostic shows why the simpler model fails.
- Version RNG algorithms, stream partition per agent/run/listing, event priorities and logical scheduling. Checkpoint their states with authoritative origin commits.

**Exit:** calibration dataset and held-out diagnostic report show distributions by session and regime (not a single averaged price trace); seed replay and adversarial stress still satisfy ledger, matching and conservation invariants.

### Slice 6 — Integrated engine acceptance and controlled scale-up

Perform the proposed 3-venue / 2-instrument (cross-listed) / five-day / 50–100-participant scenario as an **integration acceptance of implemented features**, not an excuse to defer implementation behind large tests. Include varied arrival burstiness, multi-level sweeps, cancels, large resting books, market close, news, cross-venue routing, one crash, one reconnect, a passive maker and final independent audit. All external feeds are deterministic fixtures.

Acceptance artifacts: engine binary/crate version and Cargo.lock hash, scenario+policy+calendar hashes, RNG seeds, ordered admission trace, all canonical trading and control commands, hash-linked events, partial checkpoints, final state hash, separately recomputed cash/positions/P&L/score, benchmark machine/profile/workload, replay instructions. Replayer rejects missing commands or mismatched hashes and validates ordinary orders plus admin and agent decisions.

**Exit:** every filled trade is accounted, each participant included, genesis/checkpoint replay agrees, actual crash recovery safe, no cross-participant information leakage, and reported metrics meet thresholds that were set *from measured baseline and product load*, not arbitrary external exchange benchmarks.

### Slice 7 — Products and deployment, after core gates

- Re-evaluate native vs Wasmer/WASIX server. Keep a single Rust engine linked by either target; FIX/TCP is an adapter. Cloudflare may cache/publish immutable snapshots and leaderboard, but cannot own matching, balance or scores.
- Build casefiles, instructor controls, terminals, news/tenders/economic cases and optional RIT client/API compatibility on top of final engine commands. Do not merge terminal PRs merely to claim engine readiness.
- QUARCC competition uses recorded admission, resource isolation per run/team, bounded participants, signed verifiable archives and independently reproducible judges. Define operational runbook for crash, checkpoint, stop/resume, sealed final scores.
- Add advanced OTC/tender/composite/options workflows one by one, with complete economic posting and settlement, or clearly leave them unsupported rather than emitting success-only status.

## Performance and correctness measurement protocol

**Report workload, scale, hardware, compiler, feature flags, storage settings and engine hash for every measurement.** Bunting has no trustworthy measured peak throughput from this research. Do not reuse upstream benchmark figures as local SLOs.

| Workload | Why it matters | Observe |
|---|---|---|
| Single listing: 90% passive submits / 10% cancels | Book growth and allocator/index effects | throughput, CPU cycles/order, allocation bytes, p50/95/99/99.9 core latency, book heap |
| Single listing: 40% adds / 30% cancels / 30% aggressive matches | Mixed price-level and maker lookup cost | trades/command, per-fill latency, queue priority, index utilization |
| Deep sweep/100 fills and large resting book | Worst-case event batch/settlement | worst-case latency, number of events, queue and journal bound, exact residual |
| 3 venues / overlapping instrument / 100 agents | Sequencer contention, depth and ledger aggregation | ingress->admission->execution->commit->publish times, fairness deviations, active run memory |
| 5 simulated days plus restart | State growth / journal checkpoints and roll | bytes written/committed command, history growth, restore wall time, checkpoint duration |
| 10x short burst against advertised admission rate | Slow client and backpressure | dropped/rejected messages, bounded queue high-water, retry behavior, no starvation |
| In-memory vs JSON origin vs SQLite FULL vs framed journal | Identify actual latency sources | p99 commit, sync count, filesystem writes, throughput, durability under kill/fault |
| Identical trace native vs WASIX | Hosting overhead and determinism | canonical hash equivalence, throughput, p99, resident/peak memory, syscalls, toolchain risk |

Use release-optimized builds, warmup, reproducible pinned data, independent CPU time and I/O timing, open-loop arrivals for tail latency (avoid coordinated omission), multiple runs and variance; profile allocations and flamegraphs before changing algorithms. Measure compute-only engine, in-process + store, and end-to-end FIX separately. Use bounded fixtures; no network fetch in timed region. Run CPU profiles before optimization; don't compare unrelated servers or versions.

**Initial budget-setting procedure:** run baseline JSON, then live-state prototype at 1, 10 and 100 concurrent participants and at shallow/deep books; derive achievable sustained orders/s, p99/p99.9, recovery RTO and peak memory. Establish performance budgets from actual QUARCC contest workloads and RIT classroom interactions; a five-day simulation and 100 agents are scenario dimensions, not throughput targets. If a proposed hot-path optimization doesn't change a material cost, stop.

## Focused correctness/failure checklist

- Two identical seeds and input order -> same fills, fee postings, full hashes and scores; different wall-clock pacing -> same result.
- Same instrument on multiple venues -> books, halt and depth separate; economic positions aggregate correctly.
- FIFO / market sweep / cancel race / reserve replenishment / IOC / FOK -> chosen matcher policy exactly, including price/time and leftover shares.
- Every fill -> buyer/seller notional, fee and clearing conservation; cash and inventory reservations never stranded.
- Missing/stale/invalid mark -> explicit versioned valuation outcome, not silent zero price.
- Every day boundary -> valid order expiry and scheduled carry; GTC remains until specified cancellation/expiry.
- Duplicate command after reconnect or process restart -> same response, no extra trade/post.
- Crash (before append, after append before commit, after committed durability, before ack, after ack, during checkpoint), plus disk full -> no acknowledged transaction disappears, corrupt checkpoint rejected, no partial state visible.
- Maker/taker report and public feed -> permission-filtered event dispatch, gap detected and replayed, slow consumer bounded.
- Recovery from genesis and checkpoint -> same ledger journal, book state, agent RNG, market data, score.
- All roster members -> included in scoring even if never submitting an order.
- Zero-order round / all-cancel round / halted book / high volume / out-of-order network requests -> deterministic documented behavior.
- Host A native vs host B native vs supported WASIX -> semantic replay/hashes equivalent (if not, capture differential and investigate platform dependencies).

## Decision log with criteria

| Decision | Status on 2026-10-07 | Necessary proof / ADR |
|---|---|---|
| Reuse Bunting centralized Rust venue authority, participants external | **Supported:** direct source + clean boundaries; preserve | Keep ADR 0018 authority |
| Replace double ledger with one economic truth | **Strongly supported by code defect**; precise journal/account representation proposed | Implement slice 0 and conservation/replay checks; proposed ADR 0028 |
| Use ListingKey at venue-sensitive API boundary | **Supported by demonstrated ambiguity** | Schema/version/deprecation plan and cross-listing test; proposed ADR 0028 |
| Retain OrderBook-rs 0.10.3 | **Provisional baseline**, not assumed best | Compare matchcore/custom/upstream over semantic test+benchmark; if replacement, later explicit ADR supersedes 0018/0019 |
| In-memory mutable books + staged commit | **Reasoned likely improvement**, no timing proof | Slice 2 rollback proof and A/B benchmark; proposed ADR 0028 |
| SQLite WAL FULL versus minimal framed journal | **Undecided** | Commit latency/write amplification/fault injection; choose one, avoid dual production stores |
| One writer per run | **Provisional simplicity/correctness** | Shard only if profile shows run-level throughput limiting product load |
| Discrete matching interval policy | **Accepted ADR, behavior mismatch** | Demonstrate and record actual batching/ordering; reconsider what 'fair' means; proposed ADR 0028 |
| Replayer verifies full economic archive | **Necessary, current format partial** | Add ordinary trading commands + schedule; make full-run replay acceptance; proposed ADR 0028 |
| Native vs WASIX default | **Undecided on measurements**, current accepted runtime WASIX | Platform parity/ops benchmarks and host capability review, separate ADR superseding 0027 if changed |
| Calibrated queue-reactive agents | **Research direction**, not proven realistic | Held-out stylized-fact validation vs simpler null generators |
| RIT/QUARCC expansion | **Deferred** except adapters required to validate engine | Start after slice 6 gates |

## Concrete first implementation tickets

1. **Unified economic fill vertical slice + ListingKey schema:** update packages/market-events, packages/bunting-engine, packages/ledger, packages/risk-engine, packages/bunting-application and scoped tests; implement old single-listing compatibility only where unambiguous. Deliver reconciled fills, score and explicit venue.
2. **Listing projections and active-order indexes:** engine per-venue market projection, instrument aggregate, ownership fast lookup, FIX/competition mapping, focused cross-listed fixture.
3. **Live-state and durable-origin prototype:** replace snapshot-restoration hot path, implement atomic rollback contract, benchmark SQLite FULL vs framed journal before selecting final production store.
4. **Committed event distributor and archive completeness:** unsolicited maker reports, participant catch-up, full command-run replay.
5. **Calendar/multi-day:** day expiry, overnight balances and scheduled actions, one coherent five-day run.
6. **Market dynamics and integrated engine acceptance:** calibrate simple agents, execute reproducible failure/stress experiments, select hosting based on measured data.

These are **implementation slices**, each with corresponding verification and performance measurement, not six phases of testing. Stop and reconsider an architectural choice if source evidence or measurement contradicts it; log deviations and the superseding ADR before changing a binding contract.

## Immediate documentation maintenance after decision acceptance

Update README engine status and architecture/deployment topology; reconcile older Cloudflare-first passages; refresh the RIT engine parity matrix with *current code* and reference license boundaries; mark original core-first roadmap as historical; update implementation pathway and AGENTS.md only after a relevant ADR is accepted. Keep this proposed roadmap versioned to preserve historical evidence.
