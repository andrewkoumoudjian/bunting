# Port plan: NBC market engine to Rust

## Role

NBC is a market engine, not merely a scenario catalog. The Rust port must represent the venue/simulation side of a market run:

- market/run lifecycle;
- logical clock and step advancement;
- scenario configuration;
- seeded agent populations;
- fundamental-value and shock processes;
- order acceptance, rejection, cancellation, and execution;
- order-book and trade state;
- public market-data publication;
- snapshots, replay, scoring, and run completion;
- legacy NBC HTTP/WebSocket and `DONE` compatibility.

Bunting will expose the NBC port as an explicit market-engine implementation, alongside the current OrderBook-rs-backed Bunting engine.

See ADR 0014.

## Sources

- imported engine/assets: `ref/nbc_engine`;
- duplicate scenario assets: `ref/ritc_mm/app/src/main/resources/scenarios/`;
- legacy client/compatibility source: `ref/nbc-hft-simulation`;
- external compatibility pin: `carterj-c/NBC_HFT_Simulation@35b8050546679547dc737198ea13aa0ec8ed7db8`;
- source scenario catalog: `docs/ports/nbc-scenario-catalog.md`;
- independent design references: ABIDES and NeXosim.

## License and clean-room status

The imported NBC tree does not currently expose a confirmed repository-level license in the recorded audit. Until ownership/license is resolved:

- do not copy or mechanically translate implementation text;
- use scenario/configuration files only according to their documented provenance and permissions;
- recover behavior from public interfaces, captured messages, tests, observable behavior, and an independently written specification;
- record every source, assumption, unresolved field, and behavioral divergence.

If the project has authority to relicense or port the original code, record that decision before direct translation begins.

## Target package

```text
packages/nbc-market-engine/
  AGENTS.md
  Cargo.toml
  src/
    lib.rs
    config.rs
    engine.rs
    run.rs
    clock.rs
    command.rs
    event.rs
    market_data.rs
    snapshot.rs
    scoring.rs
    errors.rs
    agents/
      mod.rs
      fundamental.rs
      momentum.rs
      noise.rs
      market_maker.rs
      institutional.rs
      spiking.rs
    compatibility/
      mod.rs
      messages.rs
      aliases.rs
      done_barrier.rs
      mapping.rs
  tests/
    deterministic_replay.rs
    compatibility.rs
    scenarios.rs
```

This is one coherent market-engine package. Internal support crates may be introduced only when they provide a real reusable boundary; do not split NBC into disconnected scenario, scheduler, and agent packages merely for directory symmetry.

Canonical scenario documents remain separate:

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

`scenarios/nbc` owns data and provenance. `packages/nbc-market-engine` owns executable behavior.

## Relationship to shared Bunting packages

The NBC engine should reuse shared packages when behavior is compatible:

- `packages/market-types` for checked identifiers and exact units;
- `packages/market-events` for common command/event envelopes;
- `packages/orderbook` for order-book/matching behavior when it can reproduce the NBC contract;
- `packages/ledger` and `packages/risk-engine` for common accounting and controls when semantics match;
- shared deterministic random, probability, or simulation primitives only after a concrete reusable boundary exists.

NBC remains responsible for its own engine-level behavior even when it composes these packages. Shared code must not erase NBC-specific lifecycle, scenario, timing, market-data, or compatibility semantics.

If NBC requires matching behavior that the default OrderBook-rs-backed package cannot reproduce, document the gap and choose among configuration, adapter logic, an upstream contribution, or an approved engine-specific implementation. Do not silently change NBC semantics to fit the default engine.

## Market-engine contract

The NBC package must provide a deterministic, transport-neutral API capable of:

```text
create_run(config, seed) -> engine_state
apply_command(state, command) -> accepted/rejected events
advance_to(state, logical_time_or_step) -> scheduled events
snapshot(state) -> versioned snapshot
restore(snapshot) -> state
market_view(state) -> bounded snapshot/delta source
state_hash(state) -> deterministic digest
finish(state) -> score/result
```

The exact Rust traits should be designed after an inventory of the NBC engine and the current Bunting engine. Do not introduce a lowest-common-denominator abstraction that loses engine-specific capabilities.

## Engine responsibilities

### Run and clock

- explicit deterministic seed and engine version;
- exact step interval/logical-time mapping;
- bounded advancement;
- deterministic ordering of external commands, scheduled agents, market consequences, and scoring;
- snapshot/restore of clock, pending work, and random streams.

### Market configuration

- instruments, tick/lot units, initial book/spread where applicable;
- fundamental value, drift, volatility, shocks, and phases;
- exact validation of all scenario fields;
- explicit handling of unknown or unresolved legacy parameters.

### Agents

NBC agent families are part of the engine behavior. Each implementation must have:

- versioned parameters and formulas;
- deterministic per-agent/per-purpose random streams;
- bounded state and output;
- immutable market observations;
- ordinary venue commands as output rather than direct book mutation;
- provenance stating whether behavior is recovered, literature-derived, or redesigned.

Known families include fundamental, long/short momentum, noise, market maker, institutional seller, and spiking agents.

### Venue/order behavior

- order validation and lifecycle;
- matching/trade generation;
- cancellation and replacement where supported;
- participant balances/positions required by the simulation;
- market-data snapshots and incremental updates;
- deterministic rejection and error behavior.

### Compatibility

Legacy NBC routes, messages, scenario aliases, authentication shape, and `DONE` barrier belong at the package compatibility boundary or a thin app adapter. `DONE` controls simulation advancement but must not obscure the canonical engine state transition.

## Scenario provenance

The five known source scenarios remain catalogued with exact Git blob SHAs:

- normal market;
- flash crash;
- mini flash crash;
- stressed market;
- HFT-dominated market.

Every canonical scenario records source path/hash, source ID, transcription commit, unit mapping, PRNG version, unresolved/redesigned fields, and agent implementation provenance.

## Port phases

### Phase 0: complete engine inventory

1. Inventory all NBC runtime components, commands, market state, lifecycle, data feeds, agent classes, scenario loaders, snapshots, scoring, and protocol behavior.
2. Separate facts proven by source/interface/tests from assumptions.
3. Resolve license/ownership or approve clean-room constraints.
4. Produce a language-neutral engine specification and state diagram.

### Phase 1: package skeleton and static configuration

1. Create `packages/nbc-market-engine` with no placeholder behavior.
2. Implement strict scenario/configuration types and provenance.
3. Define exact time, price, quantity, probability, and seed representations.
4. Parse and validate a source manifest without running agents.

### Phase 2: deterministic run kernel

1. Implement run state, logical clock/step advancement, scheduled work ordering, random-stream derivation, snapshot/restore, and state hashing.
2. Add bounds and overflow handling.
3. Add deterministic replay tests before agent formulas.

### Phase 3: first executable market

1. Implement the minimum venue/order path and market-data view.
2. Port the normal-market configuration.
3. Implement the first required agent families.
4. Run an end-to-end deterministic NBC market session.

### Phase 4: full agent/scenario coverage

Add remaining agent families, special events, four remaining scenarios, scoring, and distributional/behavioral comparisons.

### Phase 5: Bunting integration

1. Register `nbc-v1` as an explicit run engine in `bunting-rs`.
2. Translate common commands/events without discarding NBC-specific metadata.
3. Persist engine identity/version and NBC snapshot state.
4. Expose bounded market-data streaming.

### Phase 6: legacy compatibility

Implement and validate registration/start, market/order WebSockets, fills/errors, aliases, and `DONE` behavior against captured fixtures.

## Required tests

### Determinism

- same engine version, scenario, seed, and external command stream produces identical event bytes and state hash;
- snapshot plus tail replay equals uninterrupted execution;
- unrelated-agent insertion does not perturb existing named random streams where the specification requires isolation;
- same-time ordering is exact and documented.

### Market behavior

- order acceptance/rejection/cancel/fill sequences;
- price-time or NBC-specific priority rules;
- market-data snapshot/delta correctness;
- participant inventory/cash and scoring consistency;
- phase, shock, and run-completion behavior.

### Scenarios and agents

- strict field validation and exact unit conversion;
- golden vectors for recovered formulas;
- distributional tests for stochastic behavior with stated tolerances;
- all unresolved parameters reject or remain inert.

### Compatibility

- legacy registration/start and scenario aliases;
- market snapshots, order messages, fills, errors, and `DONE` sequences;
- timeout/disconnect behavior;
- exact decimal conversion or explicit rejection.

### Cross-engine integration

- Bunting can create and run both `orderbook-v1` and `nbc-v1` explicitly;
- the Bunting client and QUARCC execution engine can submit ordinary commands to NBC;
- no external execution package receives direct mutable NBC state.

## Completion criteria

The NBC port is complete when it can run its scenarios deterministically as a first-class Bunting market engine, publish compatible market data, recover from snapshots, pass behavioral/compatibility tests, and document every intentional divergence from the reference.