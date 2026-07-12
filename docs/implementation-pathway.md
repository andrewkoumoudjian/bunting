# Implementation pathway

This pathway implements ADR 0013 and ADR 0014. Reference and port decisions are governed by `reference-functionality-audit.md` and `reference-adoption.md`.

## Completed foundation

1. Pin `OrderBook-rs` `0.10.3` and `PriceLevel` `0.8.4` for the current default market engine.
2. Add a thin Bunting adapter around the upstream matcher.
3. Add checked identifiers, canonical events, participant ledger and risk boundaries.
4. Add immutable Workers Cache snapshot operations.
5. Add a plain Rust Cloudflare Worker without a Durable Object requirement.
6. Add origin-store and command-transaction boundaries with expected-version commits, idempotency and recovery.
7. Add authenticated bounded limit-GTC and cancellation routes.
8. Add the initial WASM-safe `quarcc.v1` compatibility contract.
9. Audit the reference inventory and distinguish market, execution, protocol, simulation and utility roles.

The implemented command path is:

```text
submit/cancel -> expected-version load -> cache/origin recovery -> participant risk
              -> OrderBook-rs -> canonical events -> ledger -> origin commit
              -> immutable cache write -> response
```

## Completed P0: mechanical repository reorganization

The repository now follows [`repository-reorganization.md`](repository-reorganization.md) without semantic changes:

- implemented reusable crates live under `packages/` with Cargo package names preserved;
- the thin `bunting-rs` composition crate depends inward on reusable packages;
- the Worker, its Wrangler config and its migrations live under `apps/edge-api`;
- one root Cargo workspace and lockfile remain authoritative;
- ignored `out/` is reserved for generated release bundles;
- Cargo, CI, deployment commands, documentation and scoped instructions use the active paths.

Do not implement NBC, expand QUARCC, select a FIX/SBE stack, create model/algorithm dumping grounds, fork OrderBook-rs, upgrade dependencies or change runtime behavior in this PR.

## P1: NBC evidence and market-engine foundation

The checked-in NBC snapshot proves a packaged exchange simulator, configuration/scenarios and an observable REST/WebSocket/DONE protocol, but it does not contain the Java implementation or named JAR.

### Evidence work

1. Record exact reference commit/file hashes and license/authority status.
2. Build a fixture manifest separating observed traces from documentation-derived examples.
3. Specify the externally evidenced profile: scenario/run registration, authentication, market/order streams, limit orders, cancel, fills/errors, limits, result fields and `DONE` advancement.
4. Mark internal matching, scheduling, agent formulas, persistence, recovery and scoring equations unresolved unless stronger evidence is obtained.

### Initial package work

1. Create `packages/nbc-market-engine` only with real source and tests.
2. Implement strict configuration/provenance and exact units.
3. Define typed engine capabilities; do not assume replace or unsupported order types.
4. Specify a clean-room/versioned `nbc-v1` run/matching/step contract where the reference is silent.
5. Add Bunting-required deterministic recovery, snapshots/replay and state hashing as explicitly new behavior.
6. Implement the observed limit/cancel/fill/error/market-data/DONE profile.
7. Add agent families and scenarios incrementally with formula, unit, RNG and provenance records.

Do not claim internal Java equivalence from scenario field names.

## P2: QUARCC execution-engine port

The C++/protobuf reference proves a participant OMS/execution service with submit/cancel/replace, per-strategy order managers, gateways/feeds, participant risk, ID mapping, journal/store interfaces, positions, kill switch and gRPC/Python clients.

### Evidence work

1. Record exact source/test evidence for lifecycle, report ordering, risk, positions, stores and gateways.
2. Produce an evidence-linked language-neutral transition table.
3. Resolve ownership/license or document clean-room authority.
4. Distinguish reference-proven behavior from new portable Rust behavior.

### Portable package work

1. Retain the current `quarcc.v1` compatibility contract.
2. Rename/expand to `packages/quarcc-execution-engine` in a semantic PR.
3. Add checked units, typed client/local/venue IDs and normalized venue reports.
4. Implement source-proven lifecycle transitions and explicit invalid/quarantine outcomes.
5. Add desired/live reconciliation, portable snapshots/replay and deterministic cancel planning as Bunting-added capabilities.
6. Integrate through a public Bunting client adapter.
7. Isolate gRPC, SQLite/filesystem, sockets, FIX and external gateways from the portable core.
8. Test through public interfaces against a fake venue, the default Bunting engine and NBC.

## P3: staging and engine-aware run provisioning

The current order routes reject unknown runs. Add an authenticated, idempotent and bounded administrative boundary for:

- market-engine ID/version/capabilities;
- run and instrument creation;
- participant identities, balances and limits;
- engine-specific configuration;
- environment-specific D1 setup and migrations;
- secret installation/rotation;
- staging smoke tests for provisioning, submit, cancel, duplicate command, stale version, cache miss and restart.

Selecting a market engine is separate from enabling an external participant execution engine.

## P4: committed streaming and broader default-engine capabilities

### Streaming

- plain Worker WebSocket endpoint;
- publish only after origin commit;
- bounded public/private subscriptions, frames and backlog;
- committed event-sequence cursors;
- snapshot/reset and event-tail recovery;
- no isolate-local resume guarantee.

### Default OrderBook-rs-backed engine

Expose upstream capabilities incrementally under typed engine capabilities:

- IOC, FOK and post-only;
- replace and mass cancel;
- STP and fees;
- host-driven expiry;
- upstream risk configuration;
- lifecycle history and typed rejects;
- depth, metrics, impact and enriched snapshots;
- snapshot/journal upgrade verification.

Do not imply that NBC supports the same operations unless its profile specifies them.

## P5: focused protocol, client and model packages

The reference audit shows that FIX and SBE repositories are layered workspaces. Select components through focused spikes:

- FIX core/dictionary/tag-value parsing;
- FIX session sequencing, stores and transport as separate concerns;
- SBE core/schema/codegen separately from native channels/transports/client/server;
- public Bunting client and stream recovery;
- pure market-making/arrival/agent models with exact units and provenance.

Create narrowly named packages only with real implementation and tests. Do not create empty `fix`, `protocols`, `algorithms` or `common` packages.

## Release and SDK packaging

- expose curated Rust APIs through `bunting-rs`;
- keep independently versioned language SDKs in clearly named packages when implemented;
- produce the complete Worker shim/Wasm bundle under ignored `out/` paths;
- attach checksums and build metadata to versioned releases;
- keep native gRPC/Python QUARCC packaging blocked until license/provenance requirements are resolved.

## OrderBook-rs upgrade gate

Every upgrade must run:

- native and Wasm compilation with the exact selected feature set;
- limit/market/cancel/partial-fill tests;
- snapshot checksum/restore tests;
- risk, kill-switch, expiry and deterministic mass-cancel tests;
- cache round-trip and command-transaction tests;
- size and cold-start comparison;
- review of public API, snapshot/wire format and PriceLevel version changes;
- update of the reference functionality audit and recorded pin.
