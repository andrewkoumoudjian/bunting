# Port plan: NBC market engine to Rust

## Role

NBC is a venue-side market/exchange simulator. It is not merely a scenario catalog and it is not the participant client in `ref/nbc-hft-simulation`.

The Rust port is intended to become a first-class Bunting market-engine package. This document separates the recorded reference evidence from new Bunting requirements and unresolved internals.

See ADR 0014 and [`../reference-functionality-audit.md`](../reference-functionality-audit.md).

## Evidence baseline

### Direct NBC snapshot

`ref/nbc_engine` currently contains:

- `app/README.md`;
- `app/application.yml`;
- five scenario JSON files under `app/src/main/resources/scenarios/`.

The README instructs running `exchange-simulator-0.0.1-SNAPSHOT.jar`, but that JAR and the Java implementation source are not present in the recorded tree.

### External protocol evidence

The external behavior is additionally evidenced by:

- `ref/nbc-hft-simulation`, the participant/student Python client;
- `ref/ritc_mm/API_REFERENCE.md`;
- `ref/ritc_mm/src/bin/rit_sim_adapter.rs`, which consumes the NBC API and WebSockets;
- captured or future conformance fixtures derived from actual runs.

The RIT adapter/API reference corroborates the protocol used by that integration. It is not proof of undocumented NBC internals or proof that every example field exists in every binary revision.

### Scenario evidence

The five scenario files are:

- normal market;
- flash crash;
- mini flash crash;
- stressed market;
- HFT-dominated market.

They record IDs, descriptions, seeds, duration steps, step interval, market configuration, trader families/parameters, and special-event arrays. Their exact Git blob SHAs are catalogued in `nbc-scenario-catalog.md`.

## Observed reference functionality

### Packaged application/runtime

The recorded application is identified as an Exchange Simulator. Its configuration proves:

- a Spring application named `hackathon-simulator`;
- HTTP service on port 8080 under `/api`;
- SQLite/JPA persistence configuration with WAL and a single-connection pool;
- team registration/password configuration;
- JWT authentication and expiry;
- per-team run rate limiting;
- default simulator step interval and total-step settings.

### Scenario configuration

The scenario files prove configurable:

- deterministic seed;
- step count and interval;
- initial fundamental value;
- tick size and, in normal market, initial spread;
- fundamental drift and volatility fields;
- populations/parameter records for fundamental, long/short momentum, noise, market-maker, institutional and spiking participants depending on the scenario.

The files do not prove the formulas, units, random distributions, wake-up rules, cancellation-selection rules, or intra-step ordering associated with those parameter names.

### Observable HTTP behavior

The recorded client/protocol evidence supports:

- scenario listing;
- starting a named scenario for a team and receiving `run_id`, token, seed/duration metadata;
- listing a team’s runs;
- leaderboard and student/run history views;
- authentication through team identity/password and a run token.

### Observable market/order WebSockets

The recorded client/protocol evidence supports:

- a market-data WebSocket keyed by run ID;
- an authenticated order-entry WebSocket keyed by token and run ID;
- connection/authentication notifications;
- market snapshots containing at least step and best bid/ask, with the documented integration also handling depth, trades, sizes and last trade;
- client limit-order messages containing client order ID, side, price and quantity;
- client cancellation messages;
- fill notifications with quantity, price, remaining quantity and maker/taker attribution where emitted;
- error messages;
- a mandatory `DONE` action after a participant processes a market snapshot, used by the observed protocol to advance the simulation.

No replace-order message is proven by the recorded NBC client contract. Replacement must therefore be an explicit unsupported capability or a Bunting extension until evidence establishes otherwise.

### Observable limits and run results

The recorded protocol documents:

- quantity-lot validation;
- maximum open-order behavior;
- per-team run limits;
- run termination on specified limit failures;
- result/leaderboard fields including PnL, inventory, notional, trade count, maximum inventory, aggressive quantity, blow-up state/reason and decision time.

Treat limits as versioned compatibility evidence, not timeless constants, until verified against the selected reference binary/configuration.

## Internals not established by the current snapshot

The repository does not currently prove:

- exact order-book data structures;
- matching and price/time priority semantics;
- simultaneous command versus agent ordering;
- partial-fill, cancellation and disconnect race behavior;
- agent formulas or distribution families;
- random-number generator and stream derivation;
- fundamental-value transition equations;
- internal database schema and transaction boundaries;
- persistence/restart semantics;
- internal snapshots, event journals, replay, or state hashing;
- scoring equations;
- timeout policy for the `DONE` barrier;
- whether runs are fully lockstep across all participants or advanced under another policy.

Do not turn field names or API examples into invented implementation claims.

## License and clean-room status

No repository-level license or port authorization is recorded for the NBC application/assets. Until authority is documented:

- do not decompile the missing JAR;
- do not copy or mechanically translate unlicensed implementation text obtained elsewhere;
- use observable interfaces, authorized configuration/scenario data, captured traces, independently written specifications, and independently licensed literature/reference systems;
- label every behavior as observed, independently specified, literature-derived, Bunting-added or unresolved;
- document all intentional divergences.

## Bunting-added requirements for the Rust port

The following are requirements of the new Bunting implementation. They are not claims about the reference Java internals:

- transport-neutral deterministic engine API;
- checked fixed-point price, quantity and money boundaries;
- bounded command, event, queue, market-data and snapshot sizes;
- versioned engine/scenario configuration;
- explicit engine capability metadata;
- deterministic state hashing;
- snapshot/restore and replay sufficient for Bunting recovery;
- exact source/provenance records for scenarios and model implementations;
- typed errors instead of implicit connection termination;
- canonical Bunting event/ledger translation without discarding NBC-specific metadata;
- native and Wasm test coverage for the selected deployment graph.

These additions can intentionally improve recoverability and auditability while preserving the externally specified NBC compatibility profile.

## Target package

```text
packages/nbc-market-engine/
  AGENTS.md
  Cargo.toml
  src/
    lib.rs
    capabilities.rs
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
    compatibility/
  tests/
    external_contract.rs
    deterministic_replay.rs
    scenarios.rs
```

This is one coherent market-engine package. Extract a support package only after a second real consumer proves a reusable boundary. Do not split NBC into disconnected packages merely to mirror directory categories.

Scenario documents remain separate under `scenarios/nbc/`; executable behavior belongs in the engine package.

## Relationship to shared packages

The NBC engine may reuse:

- `packages/market-types` for checked IDs and units;
- `packages/market-events` for common envelopes;
- `packages/orderbook` when its behavior can satisfy a verified NBC matching contract;
- shared ledger/risk components when semantics match;
- a selected deterministic RNG/distribution package under an explicit versioned stream specification.

Reuse is conditional. Do not alter NBC compatibility to fit the default OrderBook-rs-backed engine silently.

If matching semantics cannot be proven from the current reference, the first implementation should define an explicit `nbc-v1` clean-room matching specification and state that it is Bunting-defined pending stronger evidence. An engine-specific matcher requires an ADR only when it duplicates functionality that could otherwise be shared.

## Capability model

The market-engine contract must not assume every engine supports every operation. Store typed capabilities such as:

- submit limit;
- cancel;
- replace;
- market order;
- explicit time advance;
- lockstep `DONE` compatibility;
- depth levels;
- private execution reports;
- snapshot/restore;
- scoring.

For the currently observed NBC profile, submit-limit, cancel, market-data, fills/errors, run lifecycle and explicit `DONE` are evidenced. Replace and other order types remain unsupported/unverified until proven or added as versioned extensions.

## Port phases

### Phase 0: evidence manifest and external contract

1. Record the exact reference-tree commit and every source file/hash.
2. Record the absence of implementation source/JAR as an explicit limitation.
3. Resolve ownership/license or establish the authorized clean-room process.
4. Turn the observed REST/WebSocket messages and error cases into a language-neutral external contract.
5. Capture black-box traces from an authorized reference deployment when available.
6. Record which API-reference fields are observed in traces versus documented only.

### Phase 1: strict configuration and provenance

1. Implement strict scenario/config types with unknown-field rejection.
2. Define exact time, tick, lot, quantity, money, probability and seed representations.
3. Add a source manifest containing file hashes and unresolved semantic fields.
4. Do not execute an unresolved parameter.

### Phase 2: Bunting-defined deterministic run kernel

1. Define total ordering for external commands, agent wakeups, matching consequences, publication and scoring.
2. Select/version RNG algorithms and domain-separated streams.
3. Implement bounded run state, step advancement, snapshot/restore and state hashing.
4. Mark these as Bunting-added unless proven equivalent to the reference.

### Phase 3: minimum executable market

1. Implement the externally observed limit-order/cancel/fill/error contract.
2. Specify and test a clean-room matching policy.
3. Implement market snapshots and `DONE` advancement.
4. Run one deterministic normal-market scenario with explicitly versioned model behavior.

### Phase 4: agents, scenarios and scoring

Add agent families and remaining scenarios one at a time. Each model needs provenance, formula/units, RNG streams, bounds, golden vectors and distributional tests. Implement scoring from observed/authorized rules or label it Bunting-defined.

### Phase 5: Bunting integration

1. Register `nbc-v1` explicitly per run.
2. Persist engine identity, version/config and recovery state.
3. Translate common commands/events while retaining NBC metadata.
4. Expose bounded streaming and private reports.
5. Test the Bunting client and QUARCC execution engine against NBC through public interfaces only.

### Phase 6: compatibility profile

Implement the observed registration, run, market WebSocket, order WebSocket, limit/cancel, fill/error and `DONE` profile. Add optional extensions under separate protocol/version identifiers rather than silently changing legacy behavior.

## Required tests

### Evidence and compatibility

- every external fixture records source, date/version and whether observed or documentation-derived;
- registration/start, authentication, market/order connections, limit order, cancel, fill, error and `DONE` sequences;
- quantity/open-order/run limit behavior for the selected profile;
- unknown/unsupported operations produce explicit errors.

### Determinism and recovery

- same engine/scenario/config/seed and external command stream produces identical Bunting events and state hash;
- snapshot plus replay equals uninterrupted Bunting execution;
- ordering and RNG version changes require a new engine/model version.

These prove the Rust port’s contract; they do not prove equivalence to unobserved Java internals.

### Market/scenario behavior

- checked unit conversion and strict field validation;
- clean-room matching invariants;
- market-data and participant execution consistency;
- agent/model golden and distributional tests with provenance;
- scoring and termination consistency for rules that are actually specified.

## Completion criteria

The first NBC Rust port is complete when it:

- implements a documented `nbc-v1` market-engine specification;
- supports the evidenced external compatibility profile;
- runs versioned scenarios deterministically;
- recovers through Bunting snapshots/replay;
- publishes bounded market/private data;
- records every reference-proven, Bunting-added and unresolved behavior;
- makes no unsupported claim of internal equivalence to the missing Java implementation.
