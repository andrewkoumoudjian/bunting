# NBC executable behavior evidence

This record applies only to the JAR at SHA-256
`80afc2816970b2538dcaff808008bfebdce5426ac248c074859626605547e254`
from gitlink `35b8050546679547dc737198ea13aa0ec8ed7db8`. The complete bounded
class/resource inventory and dispositions are in
[`nbc-jar-inventory.v1.tsv`](nbc-jar-inventory.v1.tsv). Generated archive,
`javap`, database, log and HTTP files remain under ignored
`out/nbc-evidence/`.

## Reproduction environment

- JAR manifest: Spring Boot `4.0.0`, build JDK specification `25`, start class
  `ca.mc.exchange_simulator.ExchangeSimulatorApplication`.
- Inspection/runtime JDK: Oracle Java `25.0.2`; `javap` `25.0.2`; `jar`
  `25.0.2` on macOS arm64. No source decompiler was used in this slice, so no
  decompiler version or generated decompiler text exists; bytecode inspection
  used only the versioned `javap` command.
- Commands: `tools/nbc-evidence/inventory.sh`,
  `tools/nbc-evidence/inspect-bytecode.sh`, and
  `tools/nbc-evidence/capture-runtime.sh` from repository root.
- Bounds: at most 10,000 archive entries, 1 MiB per selected class/resource,
  64 KiB per runtime response/log, loopback binding, empty inherited
  environment, temporary HOME/database, and forced process cleanup.

The archive contains 269 entries, including 40 NBC application classes and 77
bundled libraries. Unsafe absolute, parent-traversal and backslash paths are
rejected before selected extraction.

## Evidence classifications

**Externally observed.** The credential-free isolated runtime started Spring
Boot on loopback and `GET /api/replays` returned HTTP 200 with five scenarios:
`hft_dominated`, `stressed_market`, `flash_crash`, `mini_flash_crash`, and
`normal_market`. The normalized response is committed as
`tests/fixtures/nbc/runtime/scenario-list.v1.json`. No authenticated run was
started, so REST start/history, WebSocket, `DONE`, matching, fill, scoring and
termination behavior remain without external runtime traces in this sprint.

**Bytecode observed.** `ScenarioConfig` stores `id`, `name`, `description`, a
`long` seed, `int` duration/step interval, generic market/trader maps and a list
of special events. `SimulationEngine` owns active run contexts, starts a run
from `ScenarioService`, initializes the event manager and traders, processes an
initial step, advances steps, submits/cancels through `OrderBook`, records fills,
publishes snapshots, applies termination tests and finishes runs. `OrderBook`
uses sorted price maps with queues, an order-id map and per-student open-order
counts; bytecode proves synchronized submit/cancel/snapshot operations but does
not by itself prove every edge-case ordering. `DeterministicRandom` wraps
`java.util.Random` and exposes double, bounded integer, boolean and log-normal
draws. `MetricsCalculator` exposes notional and aggregate trading-metric
calculations. These are implementation-shape observations, not Rust-equivalence
claims.

**Translated.** Sprint 7.1 translates strict configuration and provenance.
Sprint 7.2 translates step-zero initialization, active run lifecycle,
exact-step event selection, source-list ordering, logical increment, completion
and explicit termination. Event effects, matching, traders, market publication,
scoring and persistence remain outside this slice.

**Inferred.** The class relationships strongly suggest one coherent venue
engine boundary spanning configuration, run context, scheduler, matching,
market publication, agents, scoring and persistence. The direct snapshot and
selected JAR share scenario names and the normal-market resource hash, but that
does not prove a complete source/build relationship.

**Bunting-added.** Strict unknown-field rejection, checked fixed-point units,
bounded buffers, deterministic state hashes, snapshot/replay recovery,
canonical events and commit-before-publication remain Bunting requirements.
They are not attributed to this JAR.

**Unresolved.** Original source/license/build provenance; the complete
relationship to `ref/nbc_engine`; exact configuration units and validation;
matching edge cases and OrderBook-rs compatibility; ordering between scheduler
events, participant commands and traders beyond the observed event-before-trader
shape; repeated or late events outside normal sequential advancement;
authenticated REST/WebSocket/`DONE` behavior; scoring formulas and units;
termination policy; persistence/recovery semantics; and agent formulas/random
stream compatibility require later bytecode review and JAR-versus-Rust tests.

## Sprint 7 translation sequence

Sprint 7.1 may implement strict configuration using `ScenarioConfig.class`,
`ScenarioService.class`, the five scenario resources and their recorded hashes.
It must treat the Java generic maps and permissive binding as bytecode-observed
input shape, then label strict unknown-field rejection, exact checked units and
configuration hashing as Bunting-added. Later slices must use the inventory
dispositions for run kernel, matching, market data/`DONE`, agents, scoring and
recovery, and each translated Rust module must cite the exact class/resource
hash plus a reproducible JAR-versus-Rust fixture.

## Sprint 7.2 run-kernel evidence

`SimulationContext.class` initializes `currentStep` to zero.
`SimulationEngine.startRun` installs the context and calls `processStep` without
incrementing it. `advanceStep` reads the current step, invokes
`EventManager.triggerAtStep(currentStep)`, performs later-slice trader and market
work, increments the step by one, and finishes when it reaches the configured
duration. `EventManager` scans its Java `List` and executes every event whose
trigger step equals the current step, so same-step events retain source-list
order. Explicit termination marks the context and removes it from active runs.

The fixture under `tests/conformance/nbc/run-kernel/` normalizes those
bytecode-observed transitions. Bounded identifiers and event counts, rejection
of unreachable events, checked clock arithmetic, typed inactive-run errors and
retained terminal status are Bunting-added. Event payload effects are inert in
this slice. Wall-clock decision timing is omitted because it is not logical
simulation authority. Duplicate run-ID behavior, concurrent advancement and
post-completion persistence remain unresolved or deferred.
