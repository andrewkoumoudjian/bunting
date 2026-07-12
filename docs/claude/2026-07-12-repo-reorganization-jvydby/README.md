# Claude session — repository reorganization planning (2026-07-12)

Branch: `claude/bunting-repo-reorganization-jvydby`
Date: 2026-07-12
Scope: **exploration, planning, and codex-handoff instruction design** for the Bunting
repository reorganization, plus branch-cleanup recommendations. No source was reorganized
in this session; the deliverable is this archived record.

## What this directory is

An archive of everything this session produced: the codebase findings, the independently
derived reorganization plan (as approved in-session), the branch/PR analysis, and the
chronological session log.

## Relationship to the authoritative plan on `main`

**Important for future readers:** by the time this archive was committed, `main` already
carried an authoritative, more elaborate reorganization plan from a parallel effort:

- [`docs/repository-reorganization.md`](../../repository-reorganization.md) — the governing plan.
- [`docs/adr/0014-market-and-execution-engine-boundaries.md`](../../adr/0014-market-and-execution-engine-boundaries.md) — formalizes NBC as a *market engine* and QUARCC as an *execution engine*.

This session's plan was derived independently and **agrees with the authoritative plan** on
every major decision:

- one virtual Cargo workspace at the root (not a two-workspace split);
- `packages/` holds the reusable compose-libraries; `bunting-rs/` is the integrated project;
- keep the released `orderbook-rs = 0.10.3` — **no fork** unless a new ADR approves one;
- `out/` is gitignored; Wasm/release bundles go to GitHub Releases;
- identical branch disposition (delete the two merged `feat/*`; archive-then-delete
  `feat/deterministic-kernel-vertical-slice`).

Where they differ, **defer to `docs/repository-reorganization.md` + ADR-0014**:

| Topic | This session's draft | Authoritative on `main` | Resolution |
|---|---|---|---|
| `worker-cache` home | `bunting-rs/crates/worker-cache` | `packages/worker-cache` (reusable adapter) | Use `packages/worker-cache`. |
| Deployable worker | move `workers/edge-api` → `bunting-rs/` | `apps/edge-api` + a separate thin `bunting-rs/` composition lib | Use `apps/edge-api` + `bunting-rs/` lib (cleaner lib/bin split). |
| Extra top-level dirs | not covered | adds `schemas/`, `tools/` | Adopt `schemas/`, `tools/`. |
| Engine framing | reorg-only | NBC = market engine, QUARCC = execution engine (ADR-0014) | Adopt ADR-0014 framing. |

The doc-consistency fixes this session identified (stale `matching-engine`, lingering
Durable-Object / MarketRun prose, `market-run-do` references) remain valid follow-ups and are
recorded in [`findings-docs-and-instructions.md`](findings-docs-and-instructions.md).

## Contents

- [`plan.md`](plan.md) — the reorganization + codex-handoff plan as approved in this session.
- [`findings-repo-structure.md`](findings-repo-structure.md) — full map of the workspace, crates, WASM build, workers, references, CI.
- [`findings-docs-and-instructions.md`](findings-docs-and-instructions.md) — inventory of all 48 AGENTS.md, docs, ADRs, and the inconsistencies to fix.
- [`findings-reorg-design.md`](findings-reorg-design.md) — the Cargo-mechanics design writeup (single- vs two-workspace verdict, move map, sequencing).
- [`findings-branches-and-prs.md`](findings-branches-and-prs.md) — branch/PR state and delete/archive recommendations.
- [`session-log.md`](session-log.md) — chronological log of the session's actions.
</content>
