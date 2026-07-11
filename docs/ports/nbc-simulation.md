# Port note: Java/NBC simulation assets and scenarios

## Sources

- Existing assets: `ref/nbc_engine`
- External compatibility source: `ref/nbc-hft-simulation`
- External pin: `carterj-c/NBC_HFT_Simulation@35b8050546679547dc737198ea13aa0ec8ed7db8`
- Comparison sources:
  - `abides-sim/abides@c4bf157678928934417aba6073eb0651aeaf6d15`
  - `asynchronics/nexosim@42eb361c9c553e50b763524cf9087bb64f31af6c`

## License status

The imported `ref/nbc_engine` snapshot does not expose a complete reviewed source tree or a confirmed license. Treat its binaries/configuration as behavioral data only. The external NBC repository requires a per-file license check before copying. ABIDES is BSD-3-Clause; NeXosim is MIT OR Apache-2.0.

## What the imported assets establish

The assets identify a useful scenario and agent vocabulary:

- normal market;
- stressed market;
- flash crash;
- mini flash crash;
- HFT-dominated market;
- fundamental/value agents;
- momentum agents;
- noise agents;
- inventory market makers;
- institutional agents;
- spiking/shock behavior;
- explicit seeds, step intervals and scenario configuration.

The external NBC implementation also provides the legacy participant contract, including its REST/WebSocket shape and lockstep `DONE` behavior.

## Research conclusions

ABIDES demonstrates a strong separation between a discrete-event kernel, exchange agent, trading agents, messages, latency and scenario configuration. NeXosim demonstrates typed model ports, monotonic simulation time, a priority-scheduled event runtime and save/restore concerns.

Bunting should borrow these principles but not their runtimes:

- ABIDES is Python and includes a much broader latency/network model than the first Bunting sprint needs.
- NeXosim is optimized around a custom asynchronous multi-threaded executor, which conflicts with a single authoritative Durable Object and would add substantial Wasm/runtime complexity.
- NBC's lockstep protocol is a compatibility concern, not the canonical scheduler model.

## Canonical Bunting scenario model

A scenario version should contain:

- immutable scenario ID and version;
- schema version;
- exact instrument definitions and units;
- master seed and PRNG algorithm version;
- logical start/end time;
- market phases and administrative schedule;
- bounded agent declarations;
- typed parameter sets;
- shock/event schedule;
- scoring configuration;
- compatibility metadata identifying the source scenario and transcription notes.

Unknown fields must reject by default. Prices and quantities enter as exact decimal strings and are validated into ticks/lots. No binary Java serialization becomes a canonical Bunting format.

## Scheduler contract

Every scheduled item is ordered by:

```text
(logical_time_ns, phase, priority, schedule_sequence)
```

Required phases should be explicit and versioned, for example:

1. administration and run-state changes;
2. external/participant commands;
3. market-kernel events;
4. agent observations and wakeups;
5. scoring and end-of-step calculations.

The authoritative run allocates `schedule_sequence`. No agent may enqueue an unbounded number of same-time events.

## Agent boundary

An agent receives:

- logical time;
- its own state;
- a bounded market observation;
- a deterministic random-stream handle or pre-sampled values;
- scenario parameters.

It returns intents such as submit, cancel, replace-later, wake-at or no-op. It cannot mutate the book, ledger, score, storage or another agent.

Each random stream is derived from master seed, scenario version, agent ID and stream name. Separate streams are required for arrivals, side choice, size, price offset, cancellation and any model-specific randomness. Adding or removing an unrelated agent must not perturb an existing agent's stream.

## Port order

1. Inventory all imported scenario names, files, parameters and units.
2. Create `scenario-schema` with strict validation and provenance fields.
3. Transcribe the five known scenario families into human-reviewable JSON fixtures.
4. Implement the deterministic scheduler and random-stream derivation.
5. Implement a fundamental/value agent with no hidden clock or global RNG.
6. Implement a noise agent with bounded arrival and order-size distributions.
7. Implement an inventory market maker.
8. Add momentum and institutional agents.
9. Add spiking/shock agents and scheduled market-state transitions.
10. Add the legacy NBC protocol adapter and lockstep behavior after canonical execution is stable.

## Scenario validation requirements

- total agents and per-agent scheduled items are bounded;
- all distribution parameters are finite and valid;
- all generated quantities/prices are converted through tick/lot validation;
- wakeups cannot be scheduled before current logical time;
- scenario end time and market phases are consistent;
- duplicate agent IDs reject;
- referenced instruments exist;
- unsupported legacy fields produce a clear compatibility error;
- source provenance is retained in every transcribed scenario.

## Tests

- same scenario version, seed and command stream produces identical event bytes and state hashes;
- unrelated-agent insertion does not perturb existing random streams;
- snapshot/restore preserves scheduler and PRNG states;
- ties at the same logical time resolve by the documented total-order key;
- event queues enforce hard bounds;
- malformed and unknown scenario fields reject;
- each imported scenario has a provenance and schema-validation test;
- legacy `DONE` translation does not leak into canonical events;
- aggregate stylized metrics are checked with tolerances where exact Java equivalence cannot be established.

## Equivalence posture

The incomplete Java snapshot prevents a claim of exact implementation equivalence. The port target is:

- exact equivalence for transcribed static parameters and legacy protocol fixtures;
- deterministic Bunting behavior for a given version/seed;
- qualitative or distributional comparison for agent-generated market behavior;
- explicit documentation whenever a model formula is reconstructed rather than recovered from source.

## Copy status

- Code copied: none.
- Scenario data transcribed: not yet.
- Current use: vocabulary, compatibility inventory and implementation pathway.
