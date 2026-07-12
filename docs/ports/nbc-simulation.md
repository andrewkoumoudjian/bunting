# Port note: Java/NBC simulation assets and scenarios

## Sources

- Existing assets: `ref/nbc_engine`
- Duplicate scenario assets: `ref/ritc_mm/app/src/main/resources/scenarios/`
- Legacy client/compatibility source: `ref/nbc-hft-simulation`
- External pin: `carterj-c/NBC_HFT_Simulation@35b8050546679547dc737198ea13aa0ec8ed7db8`
- Scheduler/agent comparisons:
  - `abides-sim/abides@c4bf157678928934417aba6073eb0651aeaf6d15`
  - `asynchronics/nexosim@42eb361c9c553e50b763524cf9087bb64f31af6c`
- Source catalog: `docs/ports/nbc-scenario-catalog.md`

## License status

The imported `ref/nbc_engine` snapshot does not expose a complete reviewed Java source tree or a confirmed license. Treat its binaries, configuration and scenario files as provenance-bearing reference data until ownership/license is resolved.

No root license was found in the pinned external NBC client repository. Its Python source is a protocol contract reference, not a copying target.

ABIDES is BSD-3-Clause. NeXosim is MIT OR Apache-2.0. Their runtimes are not port targets.

## Architectural conclusion

Do not port the Java runtime. Port in this order:

1. source catalog and provenance;
2. strict canonical scenario schema and exact units;
3. deterministic single-threaded scheduler;
4. independent versioned PRNG streams;
5. one pure agent family at a time;
6. legacy REST/WebSocket and `DONE` compatibility after canonical execution is stable.

The canonical scenario engine must be usable without the legacy NBC protocol, and the legacy protocol must be usable without changing canonical event semantics.

## Exact Bunting target layout

### Canonical scenario documents

```text
scenarios/nbc/
  AGENTS.md
  README.md
  source-manifest.json
  normal-market.v1.json
  flash-crash.v1.json
  mini-flash-crash.v1.json
  stressed-market.v1.json
  hft-dominated.v1.json
```

Only `AGENTS.md` is created during this preparatory PR. Canonical JSON is intentionally deferred until the schema, units and license gates pass.

### Scenario schema

```text
crates/scenario-schema/
  src/
    lib.rs
    scenario.rs
    provenance.rs
    instrument.rs
    schedule.rs
    agent.rs
    probability.rs
    validation.rs
```

Responsibilities:

- strict versioned documents with unknown-field rejection;
- exact decimal-string input and conversion to ticks/lots;
- explicit logical-time units;
- bounded agent and event declarations;
- model-version references;
- source path/blob SHA and transcription metadata;
- unresolved legacy fields recorded as metadata, never silently executed.

### Logical clock and scheduler

```text
crates/simulation-clock/
  logical time and checked duration arithmetic

crates/scenario-engine/
  src/
    scheduled_item.rs
    phase.rs
    queue.rs
    scheduler.rs
    random_stream.rs
    command.rs
    snapshot.rs
    bounds.rs
```

Every item is ordered by:

```text
(logical_time_ns, phase, priority, schedule_sequence)
```

`phase` and `priority` are versioned enums/integers with documented ranges. `schedule_sequence` is allocated by the authoritative run and is never supplied by an agent.

Recommended first phase order:

1. administration and run-state changes;
2. external participant commands;
3. market-kernel consequences;
4. agent observations and wakeups;
5. scoring and end-of-step calculations.

The scheduler is a pure single-threaded state machine. It does not use Tokio, threads, wall time or NeXosim's executor.

### Agent models

```text
crates/agent-models/
  src/
    lib.rs
    context.rs
    intent.rs
    state.rs
    fundamental/
    noise/
    market_maker/
    momentum/
    institutional/
    spiking/
```

An agent receives:

- logical time;
- immutable bounded observation;
- its own bounded state;
- versioned parameters;
- named deterministic random streams or pre-sampled values.

It returns only intents such as submit, cancel, wake-at or no-op. It cannot mutate the book, ledger, risk, score, storage, another agent or a global RNG.

### Legacy protocol adapter

```text
crates/protocol-legacy-nbc/
  src/
    routes.rs
    auth.rs
    messages.rs
    aliases.rs
    done_barrier.rs
    mapping.rs
    errors.rs

workers/edge-api/
  /api/replays/{scenario}/start
  /api/ws/market
  /api/ws/orders
```

Responsibilities:

- legacy scenario aliases;
- registration/token/run response shape;
- market snapshot and fill/error message shape;
- order message parsing;
- `DONE` lockstep barrier;
- translation into canonical commands and committed events.

The barrier controls when the adapter asks the canonical scheduler to advance. It is not itself the scheduler and does not become a canonical event type.

### Fixtures and oracles

```text
crates/test-fixtures/
  source parameter fixtures and normalized protocol payloads

tests/oracles/
  ABIDES/NeXosim comparison harness instructions

tests/fixtures/reference/nbc/
  captured legacy client messages and expected translations
```

## Verified source scenario inventory

The following five source files are catalogued with exact Git blob SHAs:

- normal market;
- flash crash;
- mini flash crash;
- stressed market;
- HFT-dominated market.

The corresponding files under `ref/nbc_engine` and `ref/ritc_mm` have identical verified blob SHAs. See `docs/ports/nbc-scenario-catalog.md` for seeds, durations, intervals and agent populations.

## Canonical scenario model

A scenario version contains:

- immutable scenario ID and version;
- schema version;
- exact instrument definitions and units;
- master seed and PRNG derivation version;
- logical start/end time;
- market phases and administrative schedule;
- bounded agent declarations and model versions;
- typed parameter sets;
- explicit shock/event schedule;
- scoring configuration;
- source provenance and transcription notes.

No Java binary serialization, class name or floating-point value becomes a canonical Bunting format.

## Random-stream contract

Each stream is derived from a versioned domain-separated input:

```text
prng_version
scenario_version
master_seed
agent_id
stream_name
```

Separate streams are required for at least:

- wake/arrival timing;
- side choice;
- quantity;
- price offset;
- cancellation;
- each model-specific shock/noise process.

Adding, removing or reordering an unrelated agent must not perturb an existing agent's streams. Stream state is included in snapshots.

## Reference adoption decisions

### NeXosim

Use as a scheduler/save-restore design reference only. Do not depend on or port its custom async multi-threaded executor. Translate tests for monotonic time, scheduled priority and pending-event restoration into Bunting's single-threaded scheduler.

### ABIDES

Use as an exchange/agent/message/latency separation reference and as a source of independently licensed agent-model ideas. Do not embed Python. Any adapted formula records the exact ABIDES path/commit and is implemented as a pure Bunting agent model with independent PRNG streams.

### External NBC client

Use as the compatibility contract for routes, WebSockets, authentication, order/fill messages and `DONE`. Do not copy its threading, wall-clock latency measurement, local inventory/P&L authority or floating-price handling.

### Imported Java/NBC assets

Use only as source data and provenance until licensing and formulas are established. No decompilation or speculative reconstruction is accepted as proof of semantics.

## Port order

### Stage 0: provenance, completed in this PR

- inventory five scenario files;
- record blob SHAs, seeds, duration, step interval and agent populations;
- verify duplicate scenario blobs across imported trees;
- record unresolved units and formulas;
- establish target directories and ownership boundaries.

### Stage 1: schema and source manifest

1. Implement strict `scenario-schema` types.
2. Define exact probability, time, tick and lot input formats.
3. Add `scenarios/nbc/source-manifest.json` with source hashes only.
4. Add validation tests without executing agents.

### Stage 2: scheduler and randomness

1. Implement the total-order key.
2. Enforce queue/per-agent/same-time bounds.
3. Implement versioned stream derivation.
4. Snapshot/restore queue and stream state.
5. Add unrelated-agent isolation tests.

### Stage 3: first canonical scenario and agents

1. Transcribe `normal_market` after license/unit review.
2. Implement fundamental and noise agent model version 1.
3. Run deterministic replay and distributional sanity tests.
4. Add inventory market maker only after book/risk/ledger are stable.

### Stage 4: remaining models

1. Long- and short-horizon momentum.
2. Institutional schedule.
3. Spiking/shock agent.
4. Remaining four scenario documents.

### Stage 5: compatibility adapter

1. Add registration and scenario alias mapping.
2. Add market/order WebSocket translations.
3. Add `DONE` barrier and timeout policy.
4. Validate against captured external-client fixtures.

## Scenario validation requirements

- total agents, total scheduled items and per-agent same-time actions are bounded;
- all distribution parameters are finite and valid;
- prices and quantities convert through tick/lot validation;
- wakeups cannot precede current logical time;
- scenario end and market phases are consistent;
- duplicate agent IDs reject;
- referenced instruments and model versions exist;
- unsupported legacy fields produce explicit compatibility errors;
- source provenance is retained;
- unresolved parameters cannot drive behavior;
- legacy names do not leak into canonical IDs.

## Required tests

### Determinism

- same scenario version, seed and command stream produces identical event bytes and state hashes;
- unrelated-agent insertion does not perturb existing random streams;
- tie order follows time, phase, priority and sequence exactly;
- snapshot plus tail replay equals uninterrupted execution.

### Bounds and validation

- queue, agent, observation and output limits reject deterministically;
- malformed/unknown scenario fields reject;
- invalid probabilities/distributions reject;
- time and arithmetic overflow reject.

### Provenance

- each source blob matches the catalog;
- every canonical field maps to an explicit source field or documented redesign;
- duplicate imported copies do not create separate versions.

### Compatibility

- registration/start, market snapshot, order, fill, error and `DONE` fixture sequences;
- legacy decimal prices convert exactly or reject;
- adapter timeout/disconnect behavior is explicit;
- no legacy field appears in canonical command/event payloads.

### Model comparisons

- deterministic unit/golden vectors for recovered formulas;
- literature vectors for independently reconstructed formulas;
- distributional or stylized-fact tests with stated tolerances;
- no claim of exact Java equivalence without reviewed source.

## Equivalence posture

The incomplete/unlicensed Java snapshot prevents an exact implementation-equivalence claim. The port target is:

- exact provenance and static-parameter transcription;
- exact compatibility for captured legacy protocol fixtures;
- deterministic Bunting behavior for a canonical scenario version and seed;
- qualitative/distributional comparison for agent-generated behavior;
- explicit model provenance whenever a formula is reconstructed or redesigned.

## Copy and implementation status

- Java code copied: none.
- External NBC Python code copied: none.
- Scenario data duplicated into canonical directories: none, intentionally pending license/schema gates.
- Source catalog and duplicate-hash verification: complete for five scenarios.
- Scheduler/agent code: not implemented.
- Legacy protocol code: not implemented.
- Current use: source provenance, compatibility contract, exact target layout and implementation gates.