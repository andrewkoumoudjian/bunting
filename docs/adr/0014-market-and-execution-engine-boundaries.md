# ADR 0014: Market-engine and execution-engine boundaries

- Status: accepted
- Date: 2026-07-12
- Evidence baseline: `docs/reference-functionality-audit.md`
- Supersedes: documentation that describes NBC only as scenario data or describes the QUARCC trading engine only as compatibility DTOs

## Context

Bunting composes systems that use the word “engine” for different authority boundaries. Repository names and planned package names are insufficient evidence; the recorded source and contracts establish two distinct roles.

1. **NBC is a venue-side exchange/market simulator.** The direct checked-in snapshot proves a packaged Exchange Simulator, scenario/run configuration, team/JWT setup, and an observable REST/WebSocket order/market protocol with explicit step advancement. That tree does not include the Java implementation or named JAR. A separate pinned client tree contains an opaque same-named JAR whose source, license, build provenance and relationship to the direct snapshot are unresolved, so exact internal matching, scheduler and agent semantics remain unresolved.
2. **The QUARCC trading engine is a participant-side execution/OMS service.** Its C++ headers and protobuf contracts prove strategy-signal intake, submit/cancel/replace, order managers, execution gateways, market-data feeds, participant risk, ID mapping, journal/store interfaces, positions, kill switch, gRPC and Python clients. It routes to a venue and consumes execution reports; it does not own venue matching.

The current Bunting vertical slice uses released `OrderBook-rs` as the default matching kernel. That does not reduce NBC to scenarios and does not make QUARCC part of the venue kernel.

## Evidence rule

Every implementation and port document must distinguish:

- observed reference behavior;
- inferred but unproved behavior;
- Bunting-added requirements;
- unresolved behavior;
- source that cannot be copied without authorization.

No compatibility claim may exceed the recorded evidence. See `docs/reference-functionality-audit.md` and `docs/reference-adoption.md`.

## Decision

### Bunting composition

`bunting-rs` is the product/composition layer. It imports reusable first-party packages and explicitly selects a market-engine implementation for each run.

A Bunting market-engine boundary must support the common capabilities required by the product, while representing unsupported operations explicitly. Common concerns include:

- run creation and versioned configuration;
- deterministic command application;
- logical time/step advancement when applicable;
- typed acceptance, rejection, cancellation and trade output;
- public/private market views;
- Bunting-required recovery state, snapshot/restore and state hashing;
- bounded deterministic errors;
- canonical translation without discarding engine-specific metadata.

The contract is capability-based. It must not assume every engine supports replace, market orders, all order types, explicit clock advancement, scoring, or the NBC `DONE` protocol.

### Default OrderBook-rs-backed market engine

The existing Bunting market path remains the default engine. It uses released `OrderBook-rs = 0.10.3` for CLOB matching and combines it with Bunting identity, ledger, risk, persistence, recovery, protocols and Worker adapters.

`packages/orderbook` is the first-party Bunting adapter. It is not a copied upstream source tree.

### NBC market-engine package

The Rust NBC port is one coherent market-engine package, not merely a scenario parser or agent library.

#### Reference-proven external profile

The recorded evidence supports:

- an Exchange Simulator application;
- team registration/authentication and named scenario runs;
- seeded/step-configured scenario files and trader-population parameter records;
- market-data and order-entry WebSockets;
- client limit orders, cancellation, fills/errors and mandatory `DONE` step advancement;
- run/history/leaderboard result surfaces.

It does not prove the internal matching algorithm, scheduler ordering, agent formulas, persistence/restart, snapshots/replay, state hashes or scoring equations.

#### Bunting port requirements

The new `packages/nbc-market-engine` must add a documented, deterministic, transport-neutral Rust contract with exact units, bounds, capability metadata, recovery, snapshots/replay and state hashes. These additions are Bunting requirements unless separately proven equivalent to the reference.

The port may reuse shared market types, events, ledger/risk and OrderBook-rs-backed matching only when a verified or explicitly specified NBC contract is preserved. Missing internals require a versioned clean-room `nbc-v1` specification, not invented equivalence.

Direct translation remains subject to ownership/license authorization.

### QUARCC execution-engine package

The QUARCC Rust port is an optional participant-side execution package.

#### Reference-proven surface

The recorded source supports:

- strategy signal and explicit submit/cancel/replace service operations;
- per-strategy/account order managers;
- local/broker ID mapping;
- execution gateway and market-data feed boundaries;
- participant-side risk, position projection and kill switch;
- journal/order-store abstractions and SQLite implementations;
- sequential event dispatch and deferred fill handling;
- gRPC service and Python client surfaces.

#### Bunting port requirements

The portable Rust package should add exact fixed-point conversions, normalized venue reports, explicit duplicate/out-of-order outcomes, safe lifecycle transitions, desired/live reconciliation, deterministic portable snapshots/replay and a public Bunting client adapter. These are new design requirements where the C++ source/tests do not prove an equivalent behavior.

Native gRPC, sockets, broker SDKs, SQLite/filesystem stores and protocol adapters remain isolated from the portable core.

The execution engine does not own venue matching, authoritative balances, Bunting canonical sequencing or origin commits. Transport delivery does not imply venue acknowledgement or execution.

## Repository consequence

The intended topology is:

```text
packages/
  market-types/
  market-events/
  orderbook/                    # first-party adapter around released OrderBook-rs
  ledger/
  risk-engine/
  origin-store/
  command-transaction/
  worker-cache/
  nbc-market-engine/            # complete NBC Rust market-engine port
  quarcc-execution-engine/      # optional participant execution/OMS port
  client/                       # future public client boundary when implemented
  ...focused protocol/model packages selected from real implementations

bunting-rs/
  src/                          # composition, engine selection, public API
  crates/                       # private glue only when genuinely product-private

apps/
  edge-api/
  ...future deployable gateways/CLIs

vendor/
  ...approved copied/patched third-party source only
```

The repository root remains one Cargo workspace. Do not create generic `fix`, `protocols` or `algorithms` dumping grounds before selecting concrete responsibilities.

A copied or patched OrderBook-rs source tree does not belong under `packages/orderbook`. Prefer upstream fixes or a dedicated pinned fork repository; use `vendor/orderbook-rs` only through a separate approval if in-repository source is required.

## Engine selection

Every run record identifies:

- market-engine ID;
- engine version;
- typed engine configuration/schema version;
- capability set;
- recovery/snapshot format version.

Selection is explicit and durable, never an implicit request-time feature choice.

Initial families:

- `orderbook-v1`: current OrderBook-rs-backed Bunting engine;
- `nbc-v1`: documented Rust NBC-compatible/clean-room engine profile.

Common APIs normalize only genuinely common commands/events. Engine-specific metadata and unsupported capabilities remain typed.

## Authority boundary

Market engines may:

- accept participant commands;
- own matching/venue state and venue sequences;
- emit canonical/engine-specific events;
- publish authoritative market/private reports;
- persist their Bunting recovery state.

Participant execution engines may:

- consume market/private reports;
- maintain local intent, lifecycle, risk and positions;
- submit, cancel, replace, query and stop routing through public interfaces;
- persist their own application state.

No participant/client/strategy package receives a mutable internal reference to a market engine or writes venue state directly.

## Consequences

- NBC is elevated to a first-class market-engine port, with compatibility claims limited by available evidence.
- QUARCC is tested as an external participant engine and remains optional.
- Bunting can run without QUARCC.
- The same QUARCC core can be tested against deterministic fake venues, the default Bunting engine and NBC through adapters.
- OrderBook-rs remains the default matching dependency unless an approved engine-specific compatibility requirement establishes an alternative.
- Cross-engine conformance tests target only common capabilities; exact equality is required only by an explicit contract.
- Reference and port audits become prerequisites to package design.

## Required validation

### NBC

- external compatibility fixtures distinguish observed traces from documentation-derived examples;
- unsupported capabilities reject explicitly;
- deterministic Rust replay/state hashes prove the Rust contract without claiming hidden Java equivalence;
- scenario/model provenance and all Bunting-added behavior are recorded.

### QUARCC

- public contract/discriminant fixtures;
- lifecycle and report-ordering tests linked to evidence/specification;
- reconciliation/recovery tests identify Bunting-added behavior;
- gateway failures do not corrupt portable state.

### End to end

- QUARCC connects to market engines only through public client/protocol adapters;
- no authority-boundary state mutation;
- engine ID/version/capabilities survive persistence and recovery;
- reference pins, licenses and evidence classifications remain current.
