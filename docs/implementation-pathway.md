# Implementation pathway

This pathway implements ADR 0013, ADR 0014, ADR 0017, ADR 0018, ADR 0019 and ADR 0020. ADR 0020 supersedes ADR 0016 where it made tRPC the universal application boundary. Production uses one `bunting-engine`, browser-compatible fetch/stream transport, and Worker-initiated outbound FIX/TCP with direct in-process application calls. Reference and port decisions are governed by `reference-functionality-audit.md` and `reference-adoption.md`.

## Completed foundation

1. Pin `OrderBook-rs` `0.10.3` and `PriceLevel` `0.8.4` for the production matching foundation.
2. Add a thin Bunting adapter around the upstream matcher.
3. Add checked identifiers, canonical events, participant ledger and risk boundaries.
4. Add immutable Workers Cache snapshot operations.
5. Add a plain Rust Cloudflare Worker without a Durable Object requirement.
6. Add origin-store and command-transaction boundaries with expected-version commits, idempotency and recovery.
7. Add authenticated bounded limit-GTC and cancellation procedures behind a browser-compatible JSON fetch boundary.
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
- the Worker, its Wrangler config and its migrations live under `apps/bunting-worker`;
- one root Cargo workspace and lockfile remain authoritative;
- ignored `out/` is reserved for generated release bundles;
- Cargo, CI, deployment commands, documentation and scoped instructions use the active paths.

Do not implement NBC, expand QUARCC, select a FIX/SBE stack, create model/algorithm dumping grounds, fork OrderBook-rs, upgrade dependencies or change runtime behavior in this PR.

## Completed P1A: historical tRPC conformance foundation

This sequence produced the development-only tRPC oracle and transitional Rust wire implementation. ADR 0020 retains those fixtures as browser-envelope evidence but removes tRPC as an active architecture dependency:

1. Intake and pin tRPC `11.18.0` source/license as a development-only conformance oracle.
2. Record the historical `apps/edge-api` to `apps/trpc-api` move; the active Worker is now `apps/bunting-worker`.
3. Add `packages/bunting-api-contract` with real Rust types, stable procedure names/errors and schema generation.
4. Retain the active versioned contract at `schemas/browser/bunting.v1.json`.
5. Rename the sans-I/O runtime package to `packages/browser-wire`; tRPC fixtures remain under `tests/oracles` only.
6. Differential-test every supported envelope against the pinned official tRPC Fetch adapter and client.
7. Expose the active browser boundary at `/api/<procedure>` and keep internal Worker calls transport-free.
8. Map `system.health`, `orders.submit`, `orders.cancel` and `market.snapshot` to the current in-process Rust implementation; delete the corresponding REST paths (implemented).
9. Remove caller-supplied participant identity and derive the actor from verified claims (implemented with server-configured token claims).
10. Generate future browser clients from the Rust contract without adding a production tRPC dependency.

Mutation batching remains rejected. Every mutation must preserve idempotency, expected-version and commit-before-acknowledgement semantics.

### Conditional stream coordinator

Implement origin-sequence-based HTTP subscriptions in the plain Worker first. Run the ADR 0016 load/recovery spike before adding a Durable Object. If the gate passes, add one Rust `RunStreamCoordinator` per run in the same Worker deployment for committed fan-out only; it must not own commands, matching, the event log or origin state.

## P1B: authorized NBC JAR translation

The direct `ref/nbc_engine` tree proves configuration/scenarios and the external protocol but lacks the implementation. ADR 0017 selects and authorizes the pinned JAR in `ref/nbc-hft-simulation` for execution, decompilation, Rust translation and redistribution.

1. Verify the gitlink and JAR SHA-256, then create an ignored, bounded extraction workspace.
2. Inventory every class/resource with original path, hash, role and translation disposition.
3. Capture baseline external REST/WebSocket/DONE traces from the selected JAR before translation.
4. Decompile in an isolated tool environment and record tool/version/output hashes.
5. Produce behavior specifications for run lifecycle, matching, scheduler order, agents, persistence and scoring, labeling bytecode-observed, externally observed, inferred and unresolved facts.
6. Preserve `packages/nbc-market-engine` as a transitional translation/evidence package only while its proven behavior is integrated into `packages/bunting-engine`.
7. Port one coherent vertical slice at a time with file-level provenance and JAR-versus-Rust differential tests.
8. Reuse shared Rust packages and OrderBook-rs only where the evidence matches; isolate proven NBC-specific behavior behind the engine boundary.
9. Add checked numerics, bounded state, deterministic replay, snapshots and hashes as explicit Bunting requirements where the JAR lacks equivalents.
10. Build redistribution manifests containing authorization, JAR/class provenance, translated files, divergences and required notices.

The NBC port covers complete venue behavior, but ADR 0018 integrates that behavior into the single Bunting engine instead of registering an alternate kernel. Do not reduce it to scenario scaffolding or claim equivalence without a reproducible fixture.

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

- browser-compatible subscription procedures on the public plain Worker;
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
