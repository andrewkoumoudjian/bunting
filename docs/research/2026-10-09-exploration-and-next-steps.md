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
