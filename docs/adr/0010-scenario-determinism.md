# ADR 0010: Immutable, deterministic and replayable scenarios

- Status: Accepted
- Date: 2026-07-11

## Context

Bunting scenarios combine instruments, microstructure, risk, background agents, shocks, clocks and scoring. The Java scenario assets already use seeds and parameterized trader populations, but deterministic replay also requires stable serialization, iteration order, random stream derivation, logical time and versioned algorithms.

A single global random generator is fragile: adding one agent changes the random path of every later agent. Wall-clock scheduling and unordered map iteration can also change results.

## Decision

Scenario lifecycle:

```text
draft -> validated -> canonicalized -> published immutable version -> run instance
```

Publishing produces:

- canonical JSON with stable key and collection ordering;
- schema version;
- scenario identifier and version;
- content hash;
- engine compatibility range;
- explicit defaults materialized;
- agent and scoring implementation versions.

A run pins:

- scenario hash;
- run seed;
- engine build identifier;
- agent versions;
- scoring version;
- PRNG derivation version;
- quantization version;
- clock mode and pacing parameters.

The kernel orders all events by explicit logical time, priority and deterministic insertion sequence. Stable ordered collections are used whenever iteration can affect output.

Each agent receives independent deterministic random streams derived from a cryptographic hash of:

```text
run seed || agent id || instrument id || stream name || derivation version
```

Adding or removing one agent therefore does not perturb unrelated streams.

Administrators may pause, resume, halt or inject events, but each action becomes a canonical event with actor, logical time, parameters and correlation identifier.

Paced and accelerated modes change only when batches execute in wall time. They do not change logical ordering or random draws.

## Consequences

Positive:

- exact replay and score verification;
- isolated agent changes;
- reliable regression tests;
- auditable administrator intervention;
- fair competition runs.

Negative:

- scenario and agent evolution requires version management;
- published scenarios cannot be edited in place;
- unordered data structures and implicit defaults are prohibited in output-sensitive paths;
- some floating-point model behavior may require pinned algorithms and tolerances.

## Rejected alternatives

### One global RNG

Rejected because unrelated configuration changes perturb all later randomness.

### Use wall time as simulation time

Rejected because runtime scheduling and network latency would change outcomes.

### Mutable published scenario documents

Rejected because historical runs would become unverifiable.

### Administrator changes outside the event log

Rejected because replay would not reproduce the observed market.

## Validation

- same published scenario, seed and command stream produce identical event/state hashes;
- adding a disabled or unrelated agent does not change other agent streams;
- lockstep, paced and accelerated executions produce the same logical result when participant commands are identical;
- canonicalization has golden cross-language fixtures;
- every admin intervention appears in replay.

## References

- `ref/nbc_engine/app/src/main/resources/scenarios/`
- `ref/ritc_mm/artifacts/calibration/`
- `ref/wirefilter` at `61936e5f38523df3f80880bbc662e490b52e7f86`
