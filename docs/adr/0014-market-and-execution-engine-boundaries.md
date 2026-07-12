# ADR 0014: Market-engine and execution-engine boundaries

- Status: accepted
- Date: 2026-07-12
- Supersedes: any documentation that describes NBC only as scenario data or describes the QUARCC trading engine only as a compatibility DTO crate

## Context

Bunting composes several independently useful Rust packages. Two imported systems have different roles and must not be collapsed into one generic "engine" concept.

1. **NBC is a market engine.** It represents the venue/simulation side: market state, scenario execution, clock advancement, market participants/agents, order processing, market-data publication, and compatibility with the NBC run protocol.
2. **The QUARCC trading engine is a participant-side execution engine.** It is used by a user, trader, or strategy outside the market to consume data, manage orders and positions, perform risk/reconciliation, and route commands to Bunting or another venue. It may be enabled for testing, but it is not authoritative market state.

The current Bunting vertical slice uses `OrderBook-rs` as its default matching kernel. That decision does not reduce NBC to a collection of scenarios, and it does not make the QUARCC execution engine part of the venue kernel.

## Decision

### Bunting composition

`bunting-rs` is the product/composition layer. It imports reusable packages and selects a market engine for a run.

A market-engine boundary must support at least:

- run creation and configuration;
- deterministic command application;
- logical time or step advancement when the engine is simulated;
- order acknowledgement, rejection, cancellation, replacement, and trade output;
- public market-data snapshots and deltas;
- engine snapshots, restoration, and state hashing;
- bounded deterministic errors;
- canonical translation into Bunting events and ledgers.

### Default OrderBook-rs-backed market engine

The existing Bunting market path remains the default engine. It uses the released `OrderBook-rs` package for CLOB matching and combines it with Bunting identity, ledger, risk, persistence, recovery, protocols, and Worker adapters.

### NBC market engine package

The Rust NBC port is a complete market-engine package, not merely a scenario parser or an agent library. It owns NBC-compatible market behavior, including:

- NBC scenario loading and validation;
- simulation clock and step/run lifecycle;
- deterministic seeded randomness;
- NBC agent populations and market-impact behavior;
- market configuration and fundamental-value process;
- venue-side order processing and resulting market data;
- run state, snapshots, replay, and scoring where present;
- legacy NBC HTTP/WebSocket and `DONE` compatibility through adapters.

The port should reuse shared Bunting packages, including fixed-point primitives, canonical events, and the common order-book package, when those components can preserve required NBC behavior. It must not be artificially split into unrelated packages that erase the fact that NBC is one market engine. Internal modules or narrowly scoped support crates are allowed where they improve testing and dependency direction.

Exact source translation remains subject to ownership and license review. Until that is resolved, implementation must be clean-room and behavior/specification driven, with provenance and differential tests.

### QUARCC execution engine package

The QUARCC Rust port is an optional participant-side execution package. It owns:

- local/client/venue order identifiers;
- desired-versus-live order state;
- acknowledgement, reject, fill, cancel, and replace reconciliation;
- participant-side positions, exposure, and execution risk;
- market-data feed consumption;
- strategy-signal intake;
- order routing through Bunting client APIs, FIX, or other gateways;
- journaling/recovery appropriate to the participant application;
- test and simulated-feed adapters.

It does not own venue matching, authoritative market balances, canonical venue sequencing, or Bunting origin commits. Request success does not imply venue acknowledgement or execution.

The existing WASM-safe `quarcc.v1` records and transport-neutral service trait are the initial compatibility surface, not the final scope of the port. The package should grow into the execution engine while keeping a portable core and isolating native gRPC, filesystem, database, and socket adapters behind separate features or crates.

## Repository consequence

The intended package topology is:

```text
packages/
  orderbook/                    shared order-book boundary or approved fork
  nbc-market-engine/            complete NBC Rust market engine
  quarcc-execution-engine/      optional trader-side execution engine
  fix/                          FIX codec/session packages
  client/                       Bunting client and transport adapters
  market-types/
  market-events/
  ledger/
  risk-engine/
  origin-store/
  command-transaction/
  worker-cache/
  ...narrow reusable simulator and algorithm packages

bunting-rs/
  src/                          integrated Bunting product API and composition
  crates/                       Bunting-private glue only when it is not reusable

apps/
  edge-api/                     deployable Cloudflare Worker
  ...future CLIs and gateways
```

The repository root remains one Cargo workspace. `packages/` contains reusable Rust components that compose Bunting, not only non-Rust SDKs.

## Engine selection

A run configuration must identify its market-engine implementation and engine-specific version/configuration. Engine selection is explicit and durable; it must not depend on a feature flag chosen implicitly at request time.

Examples:

- `orderbook-v1`: the default OrderBook-rs-backed Bunting market engine;
- `nbc-v1`: the Rust NBC-compatible market engine.

Canonical APIs may normalize common commands and events, but engine-specific capabilities and metadata must remain typed rather than silently discarded.

## Consequences

- NBC implementation work is elevated from a late scenario task to a first-class package port.
- The QUARCC port is tested as an external participant against market-engine implementations.
- Bunting can run without the QUARCC execution engine.
- A user can enable the QUARCC engine to test OMS, execution, reconciliation, risk, and strategy behavior.
- OrderBook-rs remains the default matching dependency unless an approved fork or NBC compatibility requirement establishes a documented alternative.
- Cross-engine conformance tests are required for common command/event behavior, but exact output equality is required only where contracts specify it.

## Required validation

- NBC tests must cover deterministic run replay, scenario/agent behavior, market-data output, snapshot restoration, and compatibility fixtures.
- QUARCC tests must cover external order lifecycle, duplicate/out-of-order venue reports, reconnect reconciliation, position projection, and gateway failures.
- End-to-end tests must connect the QUARCC execution engine to both a deterministic test market and at least one Bunting market-engine implementation.
- No package may cross the venue/participant authority boundary through direct state mutation.