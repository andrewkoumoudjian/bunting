# Repository exploration and next steps — 2026-10-09

Status: **research and brainstorming handoff**. No production code changed;
the only source addition is an opt-in measurement example
(`packages/bunting-engine/examples/state_cost_probe.rs`). Accepted ADRs remain
binding; nothing here supersedes them.

- Audited head: [`main@1d857d1`](https://github.com/andrewkoumoudjian/bunting/commit/1d857d135c7bb483ddd9fe95feaab8bbaebd206a)
  (ADR 0029 owned order book), 228 commits, 2026-07-11 → 2026-10-08.
- Working branch: `claude/explore-next-steps`.
- Evidence vocabulary follows `AGENTS.md`: **observed** (source, test or
  command output recorded here), **inferred** (reasoned, not proved),
  **Bunting-added** (a proposal), **unresolved**.

## 1. What Bunting is trying to be

Reading the mission, ADRs and commit history together, the target is:

> One **headless, deterministic Rust market engine** (`bunting-engine`) that
> owns matching, orders, the economic ledger, logical time, agents' effects,
> replay and market data — with every transport (FIX/TCP, browser, TUI,
> bindings) and every host (Wasmer/WASIX, native, Cloudflare publication) as a
> replaceable adapter that never holds market authority.

Two products are meant to sit on top of that engine, *after* the core is
correct (roadmap Slice 7):

1. a **RIT-class** instructor/student trading-case simulator (Rotman Interactive
   Trader parity is the reference for cases, tenders, news, scoring); and
2. a **QUARCC competition venue**: rostered teams connect over FIX, a run is
   archived and independently re-judged, public results publish to Cloudflare.

NBC (a Java HFT competition simulator, translated from an authorized JAR) and
QUARCC's participant-side execution engine are compatibility inputs, not
second venues.

## 2. How the history reads

The 228 commits fall into five phases. The recurring motion is **removing
borrowed or duplicated authorities** until one engine owns the truth.

| Phase | Dates | Commits / PRs | What happened | Authority removed or added |
|---|---|---|---|---|
| 0. Bootstrap | Jul 11–12 | `0eaafc3`…`3c3486e`, PRs #1–#3 | 25 reference submodules, ADRs 0001–0012, Cloudflare-first design, OrderBook-rs kernel behind a Worker, origin-backed command transaction | Added: Worker + D1 + Workers Cache as runtime; OrderBook-rs as matcher |
| 1. Evidence and reorganization | Jul 12–13 | `cac77c5`…`eef9ae8`, PRs #4–#17 | `packages/` layout, evidence discipline (`reference-functionality-audit.md`), NBC and QUARCC roles corrected, Rust tRPC contract, NBC config/kernel/matching translated, **unified engine** (ADR 0018) and QUARCC execution core | Removed: "selectable" NBC venue kernel; tRPC as a runtime |
| 2. Product surface | Jul 13–21 | `9483dd0`…`1fa03bc`, PR #18 | Longbridge-derived Ratatui TUI, product contract, portable application service, native server, simulation domain (tenders, OTC, news, facilities), `v0.1.0` release | Added: native server alongside the Worker |
| 3. Competition venue pivot | Jul 28–29 | `c0876d3`…`a35f184` | ADRs 0022–0027: native WASI venue is *the* authority, Cloudflare is publication only; discrete intervals, roster, archive/replay, Rust/C/Python/C++ bindings, Tokio acceptor, Wasmer runtime | Demoted: Worker to transitional |
| 4. Core-first repair | Oct 7–8 | `0fdbd13`…`1d857d1` | Independent audit + ADR 0028 + evidence-led roadmap, then implementation slices 0–10: ListingKey everywhere, venue market data/NBBO, single ledger (fees, multipliers, FX, full-roster scoring), WAL origin + writer lease, Worker deletion, **owned deterministic book** (ADR 0029) | Removed: dual ledger, Worker/D1 authority, OrderBook-rs in production |

Observations about *how* the work is done (observed from history and
`docs/implementation-log/2026-10-07-core-slice-0.md`):

- Documentation leads implementation: 29 ADRs, a 830-line reference audit, and
  dated research notes precede code. The evidence discipline is strong, but
  documents now trail the code (section 5 of this note).
- Oct 7 slices were committed through a connector **without a local Rust
  toolchain**; CI was the compiler (many `style: apply CI rustfmt` commits). Oct 8
  slices were verified locally. The implementation log is careful to say which
  checks did and did not run — keep that habit.
- Breaking changes are made deliberately (`!` commits, snapshot versions bumped,
  old snapshots rejected, goldens re-blessed only through replay).

## 3. Where the roadmap stands at `1d857d1`

Status against [the evidence-led roadmap](../plans/2026-10-07-evidence-led-core-roadmap.md)
and ADR 0028's validation list. "Done" means implemented and covered by a test
that ran (either per the implementation log or in this session, section 4).

| Roadmap item | Status | Evidence / remaining gap |
|---|---|---|
| Slice 0 — one economic truth | **Done** | Slice 9: single `Ledger`, fees/rebates, multipliers, FX, full-roster NLV scoring, tenders/OTC settle, composites fail closed |
| Slice 1 — ListingKey through core and market data | **Done** | Slices 1a/1b/4: `SubmitOrderAtListing`, per-listing L2/tape, NBBO, FIX tag 207 |
| Slice 2 — live state, atomic commit, recovery | **Partial** | Books are now plain `Clone` values (ADR 0029) and the WAL journals once per commit, but each journal record still carries the **full candidate `RunState`**, `load_run` still clones it, and recovery reads state, not commands (section 5, G5–G6) |
| Slice 3 — dispatch and competition admission | **Not started** | No committed-event distributor; makers on other connections get no fills; open-order count is connection-local and leaks; writer is sleep-then-FIFO (G1–G4) |
| Slice 4 — multi-day calendar | **Not started** | No venue calendar or session phases; DAY orders rest forever; store caps at 10,000 commands by default (G7–G8) |
| Slice 5 — calibrated agents | **Research only** | Two Oct 8 research notes; agent runtime state is not part of recovery (G3) |
| Slice 6 — integrated acceptance | **Not started** | Depends on 2–5 |
| Slice 7 — products/deployment | **Paused by design** | TUI, server and bindings exist; browser contract has no host since Worker removal |
| ADR 0028 #5 — full archive replay | **Not started** | `CompetitionArchive` replays `SimulationCommandRequest` only (G2) |
| ADR 0028 #8 — native vs WASIX parity | **Not started** | No hash-parity or benchmark run recorded |

## 4. What was executed in this session

Environment: Linux x86_64 container, 4 vCPU Intel Xeon @ 2.80 GHz, pinned
toolchain `1.88.0` (`rust-toolchain.toml`).

| Check | Result |
|---|---|
| `cargo test --locked --workspace` at `1d857d1` | **observed: 174 passed, 0 failed** across 37 test targets — matches the Slice 10 log |
| `cargo clippy -p bunting-engine --all-targets -D warnings` with the probe | observed: clean |
| Remaining `AGENTS.md` gates with the probe added: `cargo metadata --locked`, `cargo fmt --all --check`, `cargo clippy --locked --workspace --all-targets -D warnings`, no `orderbook-rs` in `bunting-engine` normal deps, `cargo check --locked --workspace --target wasm32-unknown-unknown`, `git diff --check` | observed: all pass |
| WASIX build, Wasmer smoke, QuickFIX-Go interop | not run (no WASIX toolchain or Go in this container) |

### Per-command state cost probe

`cargo run --release -p bunting-engine --example state_cost_probe` rests *N*
one-lot bids from one participant across 1,000 price levels, then measures
each cost **once** at that book size. Single sample, wall clock, no warmup
control: treat the numbers as order-of-magnitude evidence, not a benchmark.

| Resting orders | `RunState::clone` | `transition_owned` (1 order) | `serde_json` state bytes | `serde_json::to_vec` | `state_hash` |
|---:|---:|---:|---:|---:|---:|
| 1,000 | 0.5 ms | 6 µs | 0.32 MB | 0.7 ms | 2.3 ms |
| 10,000 | 3.5 ms | 14 µs | 3.2 MB | 9.3 ms | 23 ms |
| 50,000 | 18 ms | 24 µs | 16.2 MB | 51 ms | 130 ms |
| 100,000 | 51 ms | 27 µs | 32.5 MB | 116 ms | 275 ms |

What this shows (**observed** numbers, **inferred** consequences):

1. ADR 0029 worked: the matching transition is microseconds and grows
   slowly with book size.
2. Everything *around* the transition scales linearly with state. One native
   FIX order currently pays at least three full clones —
   `session_host.rs:198` (`service.recover`), `command-transaction/src/lib.rs:97`
   (`load_run`) and `:105` (`committed_state = candidate.clone()`) — plus one
   full-state JSON serialization into the `BUNTWAL1` frame, because
   `CommitRequest.candidate` is the complete `RunState`
   (`origin-store/src/lib.rs:36-37`, `commit_journal.rs:60`).
3. Inferred per-order overhead at 10k resting orders: ≈ 3 × 3.5 ms + 9.3 ms ≈
   **20 ms and a 3.2 MB fsynced write**, versus 14 µs of matching. At 100k
   resting orders: ≈ 270 ms and 32 MB per order. Because the writer
   serializes commands, that is an inferred ceiling of roughly 50 orders/s at
   10k resting orders and 4 orders/s at 100k, before fsync, the 128-command
   full checkpoint and FIX I/O — far below the engine's own
   `MAX_LIVE_ORDERS = 250_000` bound. *Unresolved* until Step 2 measures it end
   to end.
4. The overhead is a **persistence/ownership design** cost, not a matching
   cost. That makes Slice 2 (command-sourced durability, live state owned by
   the writer) the highest-leverage engine work remaining.

## 5. Verified gaps at `1d857d1`

Each item cites source at the audited head. Items marked *(new)* are not
listed as open in the implementation log's Slice 10 follow-ups.

### Venue and session host (`apps/bunting-server`)

- **G1 — Resting makers never receive their fills.** *Observed.* The session
  loop executes a command and maps only the requester's events back onto the
  requesting socket (`session_host.rs:226-246`,
  `bunting-application/src/lib.rs:558-576`). A participant whose order rests
  and is later hit by another connection — or by a built-in agent committing
  through `scenario.rs:47-55` — gets no unsolicited `ExecutionReport`. This is
  ADR 0028 item 6 / roadmap Slice 3, and it makes the competition venue
  unusable for passive strategies.
- **G2 — The open-order limit leaks.** *(new) Observed by source reading.*
  `open_orders` is a per-connection `BTreeSet` (`session_host.rs:133`), inserted
  for every *accepted* submit and removed only on an explicit accepted cancel
  (`:229-237`). Fully filled orders, IOC/FOK remainders, GTD expiries, kill
  switch and mass cancel never release it. *Inferred:* an active participant
  is falsely rejected with `max_open_orders limit 256` after 256 accepted
  orders on one connection regardless of how many are live, and can reset the
  count by reconnecting. The fix belongs in the engine: a per-participant
  live-order counter in risk admission, not adapter state.
- **G3 — Agent runtime state is outside recovery and admission.** *(new)
  Observed.* `scenario::run` constructs `DeterministicRuntime::new` on every
  process start (`scenario.rs:62`) even though `bunting-runtime` already has
  `RuntimeSnapshot` / `restore` (`bunting-runtime/src/lib.rs:158`). Agents
  commit under `writer.lock()` directly (`scenario.rs:69`), bypassing the
  interval arrival queue that FIX commands use. *Inferred:* after a restart the
  agent RNG and wake schedule diverge from the uninterrupted run (violates
  roadmap invariant 4 and ADR 0028 item 8), and agent/human interleaving is
  decided by OS scheduling rather than a recorded admission order.
- **G4 — Interval admission is sleep-then-FIFO, not a sealed batch.**
  *Observed.* `AuthoritativeWriter::execute_interval` sleeps to the next wall
  boundary and then admits by atomic arrival ticket (`writer.rs:39-75`). Known
  in ADR 0028 item 7; the test `concurrent_arrivals_commit_in_sequence` is
  timing-dependent (passed in this session; flaky per Slice 9 log).

### Persistence and replay

- **G5 — Journal records and checkpoints are full state.** *Observed + measured
  (section 4).* Each WAL frame carries the complete candidate `RunState`; every
  128 commands `persist` rewrites the full `FileState` including **all
  committed commands and events** (`storage.rs:301-302`), so checkpoint cost
  grows with run length. Recovery restores state, it does not re-execute
  commands, so the journal is not a replay log.
- **G6 — Idempotency lookup is a linear scan.** *(new) Observed.*
  `check_record` does `state.commands.iter().find(..)` per commit
  (`storage.rs:312-321`): O(n) per command, O(n²) per run. Small next to G5 but
  free to fix with a `BTreeMap<(RunId, CommandId), _>` index.
- **G7 — Default store capacity ends a competition early.** *(new) Observed
  config, inferred impact.* `max_commands: 10_000` (store-wide, all runs, agents
  included) and `max_events_per_run: 100_000` (`config.rs:137-138`); reaching
  either returns `OriginError::Unavailable` for every later commit
  (`storage.rs` `check_record`). *Inferred:* 20 teams sending one order per
  second exhaust 10,000 commands in under nine minutes. Incompatible with the
  roadmap's five-day acceptance scenario until history is paged/compacted.
- **G8 — Archive replay excludes trading.** *Observed.* `CompetitionArchive`
  stores `accepted_commands: Vec<SimulationCommandRequest>` and replays them
  through `transition_simulation` (`bunting-rs/src/archive.rs:28,111-125`).
  Ordinary orders, cancels, agent actions and admission order are absent, so a
  judge cannot recompute fills or scores from the archive. ADR 0028 item 5.

### Engine lifecycle

- **G9 — No calendar or session phases.** *Observed.* No exchange calendar,
  session state, open/close auction or day boundary exists in
  `bunting-engine`; DAY orders rest until cancelled (Slice 10 log). The FIX
  adapter stamps `logical_time` from wall-clock epoch milliseconds
  (`session_host.rs:207`). That is replay-safe because the stamp is part of the
  recorded command, but it means "logical" time is wall time for FIX traffic
  and simulated time for agents — two clocks feeding one GTD index.

### Documentation and contracts

- **G10 — Binding architecture document contradicts accepted ADRs.**
  *Observed.* `docs/architecture.md` still states principle 1 "Use
  OrderBook-rs", lists OrderBook-rs/`pricelevel` as production dependencies
  (§5, §14), and describes Worker/D1/Workers Cache command flow (§3, §7–§9).
  `AGENTS.md` declares this file binding, so it now gives wrong instructions to
  every agent that follows the precedence rules. README "Engine model",
  "Current architecture", "Current workspace" and its `cargo tree … grep -F
  'orderbook-rs v0.10.3'` check have the same drift. ADR 0028's *Decision*
  heading still reads "proposed, conditional on acceptance" under an Accepted
  status line. Outside ADRs, research and plans, 27 Markdown files still
  mention OrderBook-rs and 17 mention the Worker/D1; some are correct (the
  oracle role in `AGENTS.md`), so each needs review rather than a blind edit.
- **G11 — Orphaned browser contract.** *Observed (Slice 10 log).*
  `bunting-api-contract` browser procedures, `browser-wire` and
  `schemas/browser` have no server since the Worker was removed. Either host
  them in `bunting-server` or retire them; keeping an unserved contract
  invites drift.

## 6. Proposed next steps (Bunting-added)

> **Superseded ordering:** the owner answered section 7's questions on
> 2026-10-09. Section 8 records those answers and the revised order; the step
> descriptions below remain the detail for each step.

Ordered by dependency and by how much each unblocks. Sizes are rough
single-engineer estimates. Each step names its acceptance test so it can land
as one reviewable PR in the style of the existing slices.

### Step 0 — Quick wins (≤ 1 day total)

> **Status 2026-10-09:** the documentation rows are done by the guidance
> reconciliation (ADR 0033): `architecture.md` and README rewritten, the
> browser contract decided (retire, ADR 0031). The ADR 0028 heading is left
> unchanged on purpose — ADR text is history and its status line governs.
> The idempotency index and writer-test rows remain open.

| Change | Where | Why |
|---|---|---|
| Rewrite `docs/architecture.md` to the code as it is: owned book (ADR 0029), single WASI venue (ADR 0022), WAL origin, no Worker | `docs/architecture.md` | G10: a *binding* document currently instructs agents to use OrderBook-rs |
| Fix README engine model, current architecture, workspace list, and replace the `orderbook-rs v0.10.3` check with the `AGENTS.md` absence check | `README.md` | G10 |
| Change ADR 0028's "Decision (proposed, conditional on acceptance)" heading | `docs/adr/0028-…` | G10 |
| Index idempotency records by `(RunId, CommandId)` | `apps/bunting-server/src/storage.rs` | G6 |
| Replace the timing-dependent writer test with one that drives arrivals through an injected clock | `writer.rs` | G4 flake |
| Decide G11: host the browser contract in `bunting-server` or delete it | `packages/bunting-api-contract`, `browser-wire`, `schemas/browser` | Unserved contract drifts |

### Step 1 — Makers get their fills; engine owns live-order limits (2–4 days)

> **Done in slice 12** (2026-10-09); see the implementation log. Test (b)
> (agent fill reaches a human) is covered structurally — agents commit through
> the same publishing origin — not by its own end-to-end test.

Closes G1 and G2; the minimum for a usable competition venue.

- Add a per-participant live-order count to `RunState` (maintained where
  `ownership` changes: rest, fill, cancel, expiry, kill switch) and a
  `max_live_orders` limit to `RiskLimits` admission. Delete the
  connection-local `open_orders` set.
- Add a bounded committed-event distributor in `bunting-server`: after a
  successful commit the writer pushes `(event_sequence, events)` into
  per-participant queues; each session thread drains its participant's queue
  through the existing `committed_messages` mapping. Overflow disconnects with
  a recovery cursor (ADR 0011 rule), never unbounded buffering.
- **Acceptance:** in `apps/bunting-server/tests/path_equivalence.rs`, (a) a
  maker resting on connection A receives an `ExecutionReport` when connection B
  takes it; (b) a built-in agent fill against a human order reaches the human;
  (c) 300 submit-and-fill cycles on one connection never trip the order limit;
  (d) a reconnect cannot exceed the limit.

### Step 2 — Measurement baseline (1–2 days)

Turn `state_cost_probe` into a small bench binary covering the roadmap's
workload table (passive build-up, mixed add/cancel/match, deep sweep) at
in-process, in-process + journal, and end-to-end FIX layers. Record results in
the implementation log. No thresholds yet; the point is a before/after for
Step 3, and a native-vs-WASIX hash/latency comparison on the same frozen trace
(ADR 0028 item 8).

### Step 3 — Writer-owned live state and command-sourced journal (1–2 weeks)

Closes G5–G7; finishes roadmap Slice 2. Section 4 shows this is where the
per-order cost is.

- One writer owns the live `RunState`. Commands transition it in place; no
  `load_run` clone, no `committed_state` clone. This needs an engine entry
  point such as `transition_in_place(&mut self, ..)` with a *no mutation on
  error* guarantee. Today `transition_owned(self, ..)` consumes the state and
  an `EngineError` drops it, so either validate-then-apply or an undo record
  is needed; the guarantee must become an API contract with a test.
  *Unresolved:* which of the two is cheaper for mass cancel and deep sweeps.
- Journal record v2 = `{command or simulation command, admission metadata,
  resulting events, event-hash chain}` — kilobytes, not megabytes. Keep a full
  `state_hash` only at checkpoints (it costs 23 ms at 10k orders).
- On append/fsync failure keep the existing poison-and-restart rule; restart
  = checkpoint + **re-execute** journaled commands and compare events. Every
  restart becomes a determinism check.
- Checkpoints hold state only; events and command records go to append-only
  segments. Replace store-wide `max_commands` with per-run, disk-backed bounds.
- **Acceptance:** probe overhead becomes O(events per command) rather than
  O(state); kill/restart before and after append, during checkpoint, and with
  a torn tail all recover a committed prefix with identical hashes; a 1M-command
  run does not hit a capacity error.

### Step 4 — Full archive and recoverable agents (3–5 days, after Step 3)

Closes G3 and G8; ADR 0028 item 5.

- Archive v2 = genesis snapshot + the Step 3 journal. The replayer re-executes
  ordinary, simulation and agent-issued commands and verifies events, final
  hash and scores.
- Recommendation: record agent-issued **commands** as ordinary inputs (replay
  needs no agent code, which keeps ADR 0014's participant boundary) and
  persist `RuntimeSnapshot` alongside each checkpoint so a restart resumes the
  same RNG and wake schedule. Route agent commands through the same admission
  path as FIX.
- **Acceptance:** `tests/goldens/competition-full-run` replays from genesis
  and from a mid-run checkpoint, including orders; a restart mid-run ends with
  the same final hash as an uninterrupted run.

### Step 5 — Admission policy as a recorded engine input (ADR + 3–5 days)

Closes G4. First an owner decision (section 7, Q1), then either:

- **Sealed interval batches:** the server collects arrivals for interval *k*,
  seals them, and the writer applies `AdmitBatch { interval_id, ordering,
  commands }` as one recorded input. Ordering is arrival FIFO or a seeded
  permutation, written into the archive. No per-caller sleep.
- **Continuous FIFO:** drop the interval gate, record arrival sequence, and
  supersede ADR 0024 explicitly.

### Step 6 — Calendar and multi-day (1–2 weeks)

Closes G9; roadmap Slice 4. Scenario v3 adds a per-venue calendar in logical
time (sessions, phases, holidays), DAY expiry at close, end-of-day marks and
overnight carry. FIX admissions get logical time from the run clock at
admission rather than epoch milliseconds, so one clock drives GTD/DAY.

### Step 7 onward

Calibrated agents (Slice 5, using the Oct 8 research notes), the five-day
integrated acceptance run (Slice 6), then products: RIT REST adapter, TUI on
the event feed, Cloudflare publisher reading archives (Slice 7).

## 7. Brainstorm and open questions

### Questions for the owner (they change the order above)

1. **What does the competition promise about fairness?** Arrival-FIFO,
   sealed-interval batches with a seeded permutation, or a frequent batch
   auction. This decides Step 5 and whether ADR 0024 is clarified or replaced.
2. **What is the next real deadline?** If a live QUARCC event comes first,
   do Step 1 before anything else and cap run length; if classroom (RIT-class)
   use comes first, Step 6 (sessions/DAY orders) rises.
3. **Is Wasmer/WASIX still the primary host?** The Tokio acceptor, `flock`
   writer lease and WASIX installer friction all add cost. Step 2's parity run
   gives the data for a host ADR either way.
4. **Browser trading UI: who hosts it?** Cloudflare is read-only by ADR 0022,
   so an interactive browser client implies an HTTP/SSE adapter inside
   `bunting-server`. If none is planned, retire the browser contract (G11).
5. **NBC compatibility scope for the next quarter:** keep, or freeze until the
   core gates pass?

### Engineering ideas worth a spike

- **Deterministic simulation testing of the venue.** Put clock, sockets and
  disk behind traits so a seeded in-process harness can drive many FIX
  sessions, slow consumers, crashes and torn writes reproducibly (the
  FoundationDB/TigerBeetle approach). It fits the project's determinism goal
  better than more wall-clock integration tests.
- **Property tests for ledger conservation** over random command streams on
  the owned book: cash + fees conserve, reservations release exactly once,
  rejected commands change nothing. The matching oracle already exists; this
  adds the economic oracle.
- **`cargo fuzz` targets** for `simfix-wire` framing and scenario JSON
  loading — the two untrusted-input parsers.
- **Hot/cold state split.** Trade tapes, bars, news and score reports could be
  event-derived projections outside the authoritative state, shrinking
  checkpoints and the hashed state.
- **Read snapshots for queries.** Market-data and account requests now
  `recover()` (clone) the run per request (`session_host.rs:198`). After Step
  3, publish an immutable `Arc` view per commit for readers.
- **Event-hash chain** (hash(prev, event bytes)) as a cheap per-commit
  integrity proof, with the full state hash at checkpoints only.

## 8. Owner decisions (2026-10-09) and revised plan

### Answers

| # | Question | Owner answer | Consequence |
|---|---|---|---|
| 1 | Fairness promise | "Most realistic for one and multiple venues. Maybe a simple algorithm that actually calculates connection distance from server to client." | [ADR 0030](../adr/0030-proposed-latency-modeled-continuous-admission.md): continuous price-time matching per listing; admission ordered by measured one-way delay removed and scenario path latency added; same model outbound. Replaces ADR 0024 intervals. |
| 2 | Next deadline | Both a live competition and classroom use | Step 1 (fills reach makers) and the calendar/session work both stay high; Step 3 performance is needed by both. |
| 3 | Wasmer/WASIX primary? | "No, not necessarily if there's better ways to run the binary anywhere" | Treat the host as a distribution question (below); decide with Step 2 data, then an ADR superseding ADR 0027. |
| 4 | Browser UI host | "An app that connects to hosted server" | [ADR 0031](../adr/0031-proposed-bunting-native-client-protocol.md): app on a shared `bunting-client` crate; browser contract retired. |
| 5 | NBC scope | "NBC is only a reference for a market engine, compatibility doesn't mean anything." | [ADR 0032](../adr/0032-proposed-nbc-reference-only.md): remove the NBC runtime surface; requires the listed `AGENTS.md` edits on acceptance. |
| — | Interfaces | "Our own communication between certified server and clients as well as FIX protocol only." | ADR 0031: FIX + Bunting Native Protocol over in-process mutual TLS; nothing else. |

**Update (same day):** the owner approved these; [ADR 0033](../adr/0033-guidance-reconciliation-2026-10-09.md)
records the acceptance of ADRs 0030–0032 as **target, not yet implemented**,
amends the status lines of the ADRs they supersede, and reconciles
`AGENTS.md`, `docs/architecture.md` and the [documentation status map](../README.md).
No code on this branch implements them yet.

### Running the binary anywhere (answer 3)

Observed: `release.yml` already builds native CLI/TUI/bindings for
`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin` and
`x86_64-pc-windows-msvc`, but ships the **server only as a WASIX module**.
Durable file mode is Unix-only (`storage.rs:49-85`, `flock`; non-Unix is
refused).

Options, in recommended order (Bunting-added):

1. **Native static server binaries** — `x86_64`/`aarch64` Linux musl plus
   macOS, built in the existing release matrix. Runs on any VPS or laptop with
   no runtime install. Native also exposes `TCP_INFO` kernel RTT (useful to ADR
   0030's anti-gaming) and real threads for the session fan-out.
2. **OCI container image** of the musl binary (distroless/scratch base) for
   cloud hosts and Kubernetes; same binary as option 1.
3. **WASIX** kept as an optional sandboxed target only if Step 2 shows parity
   and the operators value the sandbox.

Windows servers would need a `LockFileEx` writer lease before durable mode
works; clients and the app run on Windows already.

The engine keeps its `wasm32-unknown-unknown` gate either way; that protects
host neutrality regardless of what the server ships as.

### Revised order

| Order | Step | Why now |
|---|---|---|
| 1 | **Step 0** quick wins + owner review of ADRs 0030–0032 | Docs currently mislead agents; decisions unblock everything else |
| 2 | ~~**NBC removal**~~ — **done in slice 11** (ADR 0032) | Shrinks the engine and schema before the Step 3 refactor |
| 3 | ~~**Step 1**~~ — **done in slice 12** (plus a cross-session ID collision fix it uncovered) | Competition is unusable for passive strategies without it |
| 4 | **Step 2** measurement baseline — **native baseline done in slice 13**; native-vs-WASIX parity still open (needs a WASIX toolchain) | Feeds the host ADR and proves Step 3 |
| 5 | ~~**Step 3**~~ writer-owned live state + command journal — **done in slice 14** | ~1000× gap between matching and per-command overhead; needed by both products |
| 6 | ~~**Latency-modeled sequencer**~~ — **done for FIX in slices 15–16**; one latency behaviour, no modes, since slice 16 (ADR 0035 supersedes ADR 0034); agents join with Step 4 | Replaces the interval writer (G4); needs Step 3's journal to record admission inputs |
| 7 | **Step 4** full archive + recoverable agents | Agents get a location in the latency model and go through the same sequencer |
| 7b | **Cross-venue public market data** ([exploration](2026-10-10-cross-venue-market-data.md)): per-venue trade and L2 feeds over the virtual path, then a consolidated feed and L3; owner decisions on data/colocation pricing and broker IDs | Makes venue arbitrage observable and realistic (owner request 2026-10-10) |
| 8 | **Step 6** calendar, sessions, opening/closing auctions, multi-day | Classroom realism; auctions are also part of "most realistic" venues |
| ∥ | **BNP + `bunting-client` + app** (ADR 0031) | Adapter work; can run in parallel from order 3 onward with a second contributor |
| 9 | Host ADR superseding 0027, native release of the server | After Step 2 data |
| 10 | Calibrated agents, integrated five-day acceptance | Roadmap Slices 5–6 |

### Smallest tangible first PRs

1. `docs`: rewrite `docs/architecture.md` and README to match `1d857d1`
   (G10) — no behavior change, unblocks every future agent session.
2. `refactor(engine)!`: remove NBC compatibility per ADR 0032.
3. `fix(server)`: engine-owned per-participant live-order count replacing
   connection-local `open_orders` (G2), with the four Step 1 acceptance tests.
4. `feat(admission)`: a host-neutral, pure `LatencySequencer` (estimator +
   priority queue + recorded admission record) with unit tests for ADR 0030's
   ordering and anti-gaming properties, before wiring it into the server.
