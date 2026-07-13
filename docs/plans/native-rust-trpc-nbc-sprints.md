# Native Rust tRPC and NBC implementation sprints

Status: implementation-ready plan

Canonical decisions: ADR 0013, ADR 0014, ADR 0016 and ADR 0017

## Outcome

Bunting ships one public native Rust Cloudflare Worker whose only application API is the versioned tRPC contract. The Worker invokes Rust market engines in process, commits origin truth before acknowledgement, and exposes committed recovery-aware subscriptions. A native Rust FIX bridge maps FIX/TCP to the Rust tRPC client. The authorized NBC JAR is translated into a complete selectable Rust market engine with JAR-linked provenance and differential tests.

## Current implementation gaps

- `apps/edge-api` still uses `worker::Router` and REST resources.
- `schemas/trpc/bunting.v1.json` is specified but no Rust contract/wire package exists.
- Caller-supplied `x-bunting-participant-id` is still trusted by the provisional Worker.
- No tRPC query, mutation, batch, error or subscription differential suite exists.
- No native tRPC client or FIX bridge implementation exists.
- The optional Rust stream coordinator has not passed its gate and must not be added yet.
- NBC evidence and authorization are recorded, but the JAR has not been inventoried, traced or translated.
- Cargo-less legacy scaffolds under `crates/`, `clients/`, `services/` and `workers/` are not production workspace members and must not be mistaken for implemented packages.

## Target organization

Create paths only when a sprint adds real source and tests:

```text
packages/
  bunting-api-contract/       Rust procedure types, errors, schema/hash export
  trpc-wire/                  bounded tRPC HTTP/SSE parsing and encoding
  trpc-client/                native client transport for Rust adapters
  nbc-market-engine/          complete authorized NBC venue-engine port
  fix-tagvalue/               only after the selected codec spike passes
  fix-session/                only with real session/store implementation

bunting-rs/
  src/                        engine selection and curated product API

apps/
  trpc-api/                   renamed native Rust Worker, D1 migrations, Wrangler

clients/
  fix-bridge/                 native FIX/TCP to tRPC application
  typescript-sdk/             generated declarations and official tRPC client wrapper

schemas/
  trpc/bunting.v1.json        canonical public procedure/error/FIX mapping contract
  nbc/                        versioned translated config/protocol schemas when implemented

tests/
  conformance/trpc/           official tRPC versus Rust wire fixtures
  conformance/nbc/            selected JAR versus Rust engine fixtures
  deployment/                 Worker, D1, cache, subscription and optional DO tests

tools/
  nbc-evidence/               bounded extraction, inventory and provenance tooling

out/
  nbc-evidence/               ignored decompiler intermediates and traces
  releases/                   ignored release assembly
```

Dependency direction remains `packages -> bunting-rs -> apps/clients`. Protocol packages have no market semantics. `nbc-market-engine` may depend on shared market types/events, ledger/risk and the OrderBook-rs adapter only when its evidence-linked behavior remains compatible.

## Sprint 0: toolchain, reference intake and contract freeze

Branch: `feat/native-rust-trpc-s0-reference-contract`

1. Confirm Rust `1.88.0`, `rustfmt`, `clippy` and `wasm32-unknown-unknown` from the workspace toolchain.
2. Add the official tRPC source as a pinned research reference or otherwise record the exact `11.18.0` source commit, MIT license, package manifests and protocol entrypoints under the reference-adoption rules.
3. Audit the Fetch adapter, HTTP batch link, HTTP subscription link, error formatter and transformer behavior selected for `bunting.v1`.
4. Freeze the supported subset in `schemas/trpc/bunting.v1.json`, including methods, paths, envelopes, headers, content types, batching, errors, SSE lifecycle and unsupported features.
5. Add golden fixtures produced by the official tRPC implementation; do not add production TypeScript runtime code.
6. Update the audit/adoption documents before calling tRPC a conformance oracle.

Gate: every fixture names the tRPC version/source and has a deterministic normalized expected result. No server package is created yet.

## Sprint 1: mechanical Worker path move

Branch: `chore/edge-api-to-trpc-api`

Use `git mv apps/edge-api apps/trpc-api` and preserve the Cargo package name. Update root Cargo membership, CI, Wrangler commands, migration paths, docs, scoped instructions and release assembly atomically. Do not change request behavior in this sprint.

Gate: all required native/Wasm checks, Worker build output, migration discovery, stale-path search and dependency-direction checks pass.

## Sprint 2: Rust contract and tRPC wire packages

Branch: `feat/rust-trpc-contract-wire`

1. Add `packages/bunting-api-contract` with `system.health`, `orders.submit`, `orders.cancel` and `market.snapshot` types first.
2. Generate and verify the canonical schema/hash from Rust without hand-editing a second contract.
3. Add `packages/trpc-wire` as sans-I/O parsing/encoding code with fixed request, batch, error and response bounds.
4. Implement single query/mutation, bounded query batch and structured error envelopes.
5. Differential-test status, headers and normalized bodies against the official tRPC fixtures.

Gate: native and Wasm tests pass; mutation batching, unknown procedures, unsafe transformers and unsupported extensions reject explicitly.

## Sprint 3: native Worker tRPC cutover

Branch: `feat/native-rust-trpc-worker`

1. Replace `worker::Router` with one direct fetch handler.
2. Expose only `GET|POST /trpc/<procedure-or-batch>`.
3. Map the four initial procedures to existing in-process Rust behavior.
4. Delete `/health`, `/v1/cache/...` and `/v1/runs/...` resources.
5. Replace caller-selected participant headers with verified authentication claims.
6. Preserve request bounds, expected versions, duplicate outcomes, typed errors and commit-before-acknowledgement.

Gate: route inventory proves no application REST surface; official clients pass end-to-end against the Rust Worker; D1/cache recovery tests remain exact.

## Sprint 4: clients and FIX compatibility

Branch: `feat/trpc-clients-fix-bridge`

1. Add `packages/trpc-client` with the exact supported wire subset and bounded reconnect/retry behavior.
2. Generate `clients/typescript-sdk` declarations/wrapper from the Rust contract and verify its hash.
3. Select FIX components through the audited IronFix/Fixer/FerrumFIX/QuickFIX/J spike; review FIX dictionary licensing separately.
4. Add real `fix-tagvalue` and `fix-session` packages only with implementation/tests.
5. Implement `clients/fix-bridge` with loopback FIX/TCP, durable local session state and application mappings from the canonical schema.

Gate: two independent FIX implementations pass logon, heartbeat, resend/gap-fill, restart, submit, cancel, partial/full fill, reject and market-data recovery tests.

## Sprint 5: committed subscriptions and Durable Object decision

Branch: `feat/trpc-committed-subscriptions`

1. Implement the pinned tRPC HTTP-subscription subset against origin event sequences.
2. Test snapshot/reset, reconnect, public coalescing, private no-drop behavior, slow consumers and bounded backlog in the plain Worker.
3. Run the documented staging load profile and record latency, polling, subrequest, CPU and cost evidence.
4. Add no Durable Object if the plain Worker passes.
5. If it fails an ADR 0016 criterion, implement the Rust `RunStreamCoordinator` in a separate PR with Wrangler migration, eviction recovery and non-authority tests.

Gate: the decision report is committed whether the Durable Object is accepted or rejected.

## Sprint 6: NBC extraction and executable evidence

Branch: `feat/nbc-jar-evidence-extraction`

1. Verify the pinned gitlink and JAR hash.
2. Add bounded tooling under `tools/nbc-evidence`; write intermediates only to ignored `out/nbc-evidence`.
3. Inventory archive resources/classes and decompiler/tool versions.
4. Run the JAR without credentials in an isolated environment and capture baseline external traces.
5. Produce class/resource disposition, translation provenance and behavior-spec documents.
6. Update the evidence manifest with extracted hashes and resolved/unresolved semantics.

Gate: no generated decompiler output is production source; every planned translated module has an evidence pointer and review disposition.

## Sprint 7: NBC market-engine vertical slices

Use one branch/PR per coherent slice:

1. strict configuration, units and provenance;
2. run lifecycle, logical clock and scheduler ordering;
3. limit/cancel/matching/fill behavior;
4. market data and `DONE` barrier;
5. agent families and deterministic random streams;
6. scoring, limits and termination;
7. persistence observations plus Bunting snapshot/replay/state hashing;
8. full scenario and external-contract conformance.

Each PR updates the translation ledger, cites JAR class/resource paths, adds JAR-versus-Rust fixtures and labels intentional Bunting additions. An NBC-specific matcher is permitted only when the evidence disproves shared OrderBook-rs compatibility.

## Sprint 8: engine selection, staging and release

Persist engine ID/version/capabilities/configuration with every run. Exercise the default `orderbook-v1` and translated `nbc-v1` engines through the same tRPC and FIX client boundaries. Validate recovery, deployment, migrations, artifact checksums, redistribution manifests, scenario provenance and rollback before release.

## Program rules

- One semantic concern per PR; mechanical path moves stay separate.
- No implementation claim without file/test/runtime evidence.
- No empty future package directories.
- No public REST fallback.
- No Durable Object market authority under ADR 0016.
- No JAR-derived production module without file-level translation provenance.
- Every sprint runs the repository-required checks plus its focused gates.
