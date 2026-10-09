# ADR 0032: NBC is reference evidence only, not a compatibility target

- Status: **Proposed** (2026-10-09). Drafted from the owner's direction on
  2026-10-09; not binding until accepted.
- Would supersede: the NBC-compatibility clauses of ADR 0018 (NBC as a
  complete compatibility input/profile of the unified engine) and the NBC
  paragraphs of `AGENTS.md` "Engine roles" listed below.
- Keeps: ADR 0017's licensing record (the JAR authorization remains a fact
  about what may be read and translated); `ref/` evidence and `docs/ports/`
  NBC notes as historical provenance.

## Context

Owner direction (2026-10-09): *"NBC is only a reference for a market engine,
compatibility doesn't mean anything."*

`AGENTS.md` currently says the opposite — "NBC is a complete compatibility
input to the unified engine" and "NBC-specific scenario, scheduler, agent,
scoring and protocol behavior remains visibly provenance-linked inside the
unified engine". Under the repository's precedence rules this cannot be
changed silently; it needs an ADR and an explicit `AGENTS.md` edit.

Observed NBC footprint at `1d857d1`:

| Path | Size | Role |
|---|---:|---|
| `packages/bunting-engine/src/compatibility/nbc/` (config, scheduler, synchronization, profile, mod) | 861 lines | NBC scenario config, step scheduler, participant "done" barrier |
| `packages/bunting-engine/src/lib.rs` | ~30 references | `RunState.nbc_compatibility`, `with_nbc_compatibility`, `CommandPayload::NbcDone` handling, `EngineError::NbcCompatibility`, one unit test |
| `packages/market-events/src/lib.rs` | `NbcDone` command; `NbcParticipantDone`, `NbcStepAdvanced` events | canonical schema surface |
| `packages/bunting-engine/tests/nbc_{config,scheduler}_conformance.rs` | 185 lines | conformance against extracted evidence |
| `tests/conformance/nbc`, `tests/fixtures/nbc`, `schemas/nbc`, `scenarios/nbc` | fixtures/schema | inputs to the above |
| `tests/oracles/nbc-matcher` | 651 lines | translated NBC matcher kept as a differential oracle |
| `tools/nbc-evidence`, `docs/ports/nbc-*` | scripts, notes | evidence extraction and provenance |
| adapters (`bunting-application`, `session_host.rs`) | 3 match arms | pass-through of `NbcDone` |

No production scenario in `apps/bunting-server/config/` uses NBC
compatibility (*observed*: the server boots `scenario.json`, schema v2).

## Decision

1. NBC is **reference evidence** for how one market simulator was built —
   equivalent in status to the other entries in `ref/` — and **not** a
   behavior Bunting must reproduce or a profile the engine must offer.
2. Remove the runtime compatibility surface: the `compatibility::nbc` module,
   `RunState.nbc_compatibility`, `CommandPayload::NbcDone`,
   `EventPayload::NbcParticipantDone`/`NbcStepAdvanced`,
   `EngineError::NbcCompatibility`, the adapter match arms, the two
   conformance test files and their fixtures/schema/scenario inputs. Bump
   `ENGINE_VERSION`/`ENGINE_SNAPSHOT_VERSION` and reject older snapshots, as
   Slices 3, 9 and 10 did.
3. Keep `docs/ports/nbc-*`, `tools/nbc-evidence` and `ref/` entries as
   provenance, marked historical.
4. `tests/oracles/nbc-matcher`: retire it unless a test shows it catches a
   matching difference the OrderBook-rs oracle and the owned book's own tests
   miss (*unresolved*; owner's call).
5. Ideas worth keeping from NBC (e.g. a lockstep "all participants done"
   barrier for teaching cases) re-enter only as Bunting-native features with
   Bunting-level semantics and tests, without NBC naming.

### `AGENTS.md` changes on acceptance

- "Engine roles → Market engine": delete the three NBC bullets (currently
  lines 38–40) and replace with: "NBC is reference evidence only (ADR 0032);
  it is not a compatibility target."
- "Repository organization": drop "Runtime NBC-compatible logic belongs in
  `packages/bunting-engine`" (line 60).
- "Binding architecture decisions": drop the NBC-on-the-same-book bullet
  (line 84).
- "Source and license rules": keep the ADR 0017 sentence (line 104); it still
  governs any reading of the JAR.
- "Required checks": drop "active documentation of NBC/QUARCC roles" for NBC
  (line 132), keep QUARCC.
- Instruction precedence: replace the ADR 0018 NBC reference with ADR 0032.

## Consequences

- Smaller engine and canonical schema (~1,000 lines and one command/two event
  types fewer) before the Step 3 live-state refactor, which then has less to
  carry.
- Snapshots containing `nbc_compatibility` stop loading (none are in active
  use per the observation above).
- Docs that describe Bunting as "NBC-compatible" become historical; README and
  the product contract need a line each.

## Rejected alternatives

- **Keep the module unused:** dead canonical-schema variants still have to be
  versioned, hashed, tested and explained.
- **Move it behind a cargo feature:** keeps a second semantic mode alive in
  the engine for no stated user.

## Validation

- `rg -i nbc packages apps bunting-rs bindings` finds no runtime references
  after removal (documentation and `ref/` excepted).
- Full workspace tests, Clippy and the wasm32 check pass; the competition
  golden is re-blessed only through replay.
- `AGENTS.md` no longer instructs agents to preserve NBC compatibility.

## Operational impact

None for current deployments; no shipped scenario depends on NBC mode.

## Security impact

Removes one command type from the authenticated surface.

## References

- [ADR 0017](0017-authorized-nbc-jar-port.md), [ADR 0018](0018-unified-bunting-engine.md)
- [`docs/ports/nbc-simulation.md`](../ports/nbc-simulation.md), [`docs/reference-functionality-audit.md`](../reference-functionality-audit.md)
- [Exploration note, 2026-10-09](../research/2026-10-09-exploration-and-next-steps.md) — owner decisions
