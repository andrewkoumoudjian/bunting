# Bunting agent instructions

## Mission

Build a Rust market-simulation and exchange-testing platform composed from reusable packages, with a plain Cloudflare Worker deployment target.

## Instruction precedence

- Read this file before changing the repository.
- Read the nearest scoped `AGENTS.md` for every path touched.
- Accepted ADRs and `docs/architecture.md` are binding.
- ADR 0014 defines market-engine versus participant execution-engine authority; ADR 0018 supersedes its selectable-market-engine model with one production `bunting-engine`, and ADR 0019 makes that package the direct owner of the OrderBook-rs integration.
- Read `docs/reference-functionality-audit.md` before using, moving, porting, comparing, or describing anything under `ref/` or `vendor/`.
- Read `docs/reference-adoption.md` before adding a dependency, source adaptation, fork, vendored file, or conformance oracle.
- Before reorganizing paths, read `docs/repository-reorganization.md` and follow its execution contract.

When documents conflict, do not silently select the convenient interpretation. Reconcile the active ADR, source-backed audit, and implementation before changing code.

## Evidence discipline

For every reference claim, distinguish:

1. **observed:** proved by the recorded source, manifest, contract, test, or captured external behavior;
2. **inferred:** a reasoned interpretation that is not directly proved;
3. **Bunting-added:** a new requirement or design choice;
4. **unresolved:** missing source, license, units, formulas, ordering, or behavior;
5. **prohibited to copy:** source or specification material lacking the required authorization/license.

Never infer functionality from a repository name. Never treat `.gitmodules` branch metadata as the checked-out commit. Verify submodule pins with `git ls-tree HEAD` and `git -C ref/<name> rev-parse HEAD`.

## Engine roles

### Market engine

A single production `bunting-engine` owns venue/simulation authority: run state, time or step advancement where applicable, market configuration, order processing, trades, public market data, and the recovery contract required by Bunting.

- The engine package directly integrates released `OrderBook-rs` for CLOB matching; applications and orchestration packages must not consume the matcher as a peer authority.
- NBC is a complete compatibility input to the unified engine. Do not describe NBC as only scenario JSON, a scheduler helper, or a collection of agent models, and do not create a second selectable venue kernel.
- The direct NBC snapshot lacks the Java implementation and named JAR; the separately pinned JAR is authorized under ADR 0017 for bytecode inspection, Rust translation and redistribution. Cite bytecode or differential evidence before claiming exact internal equivalence.
- NBC-specific scenario, scheduler, agent, scoring and protocol behavior remains visibly provenance-linked inside the unified engine; any incompatibility with OrderBook-rs is an explicit unresolved gap or reviewed extension, not an implicit second matcher.

### QUARCC execution engine

The QUARCC trading engine is an optional external participant-side execution/OMS engine for users, traders, and strategies. Its recorded source includes strategy signals, submit/cancel/replace, order managers, gateway/feed boundaries, participant risk, ID mapping, journal/store abstractions, positions, kill switch, market-data streaming, gRPC and Python clients.

It must never become authoritative market state or directly mutate a market engine. Bunting must run without it. The existing `quarcc.v1` compatibility crate is the first surface of the port, not its final scope.

### Other participant-side references

RITC market making, NautilusTrader, Barter, market-maker-rs, and the NBC student client are participant strategy/execution/client systems. They are not venue matching engines.

## Repository organization

The repository root remains one Cargo workspace and owns the single `Cargo.lock` and workspace-wide `.cargo/config.toml`.

- `packages/`: first-party reusable Rust packages that compose Bunting. This includes primitives, the unified market engine, execution engines, protocol components, clients, simulators, and narrowly scoped algorithm/model libraries.
- `bunting-rs/`: integrated Bunting product/library that imports packages, configures the unified engine, and exposes the curated public API.
- `bunting-rs/crates/`: Bunting-private glue only when code has no reusable package role.
- `apps/`: deployable Workers, binaries, CLIs, and gateways that depend on `bunting-rs` or public package APIs.
- `scenarios/`: human-reviewable scenario documents, fixtures, and provenance. Runtime NBC-compatible logic belongs in `packages/bunting-engine`.
- `schemas/`: versioned protocol and file schemas.
- `tests/`: cross-package, cross-engine, protocol, and deployment tests.
- `tools/`: repository automation and release tooling.
- `ref/`: read-only source evidence and provenance; never a production path dependency.
- `vendor/`: approved copied/patched third-party source with license, exact upstream revision, notices, and patch log.
- `out/`: generated release bundles; ignored and never source of truth.

Do not create a nested Cargo workspace in `bunting-rs`. The root workspace includes `packages/*`, `bunting-rs`, justified private Bunting crates, and `apps/*`.

## Package discipline

- Reusable first-party code belongs under `packages/`, not product-private directories.
- A package must have one clear responsibility, explicit dependency direction, workspace metadata/lints, tests, and scoped instructions when needed.
- Packages must not depend on `bunting-rs` or `apps/`; dependency flow is packages -> `bunting-rs` -> apps.
- Avoid generic `common`, `utils`, `algorithms`, `fix`, or `protocols` dumping grounds. Name packages after a concrete responsibility such as `fix-tagvalue`, `fix-session`, `execution-reconciliation`, or `market-making-models` when implementation justifies them.
- Keep mechanical moves separate from semantic renames and feature work.
- Use `git mv`, preserve package names during repository reorganization, and repair Cargo, CI, Wrangler, migrations, docs, scripts, and scoped instructions atomically.
- Do not create empty package directories to represent future ideas.

## Binding architecture decisions

- `orderbook-rs = 0.10.3` remains the production matching dependency internal to the unified `bunting-engine` package.
- Do not create another generic Bunting-owned CLOB when the upstream API provides the required behavior.
- NBC may require compatibility behavior around the shared matcher, but a separate production matching implementation is prohibited unless a later ADR changes ADR 0018 with documented evidence and differential tests.
- `packages/orderbook` is the current transitional first-party adapter. Move its behavior and tests into a private `bunting-engine` module during the foundation migration, then remove the crate; it is never a location for copied upstream source.
- Handle an OrderBook-rs issue through features/configuration, upstream contribution, released fix, then a dedicated pinned fork repository. Use `vendor/orderbook-rs` only when an in-repository patched source copy is explicitly approved. Do not hide third-party source under `packages/`.
- The deployment target is one native Rust Cloudflare Worker with direct tRPC dispatch and no REST router. ADR 0016 authorizes an optional Rust stream-coordination Durable Object only after its gate; it never owns market commands or origin truth.
- Workers Cache stores immutable checksum-addressed public book snapshots; it is not a transaction coordinator.
- Accepted commands, canonical events, idempotency, and optimistic versions remain authoritative in the origin store.
- Commit authoritative state before acknowledgement, cache publication, or stream publication.

## Authority boundaries

The Bunting engine owns venue-side identities, matching results, canonical events, authoritative ledger projections, scenario/run state, and market-data publication.

Participant-side packages own local order intent, venue reconciliation, strategy state, participant risk, and client/gateway connectivity. They submit ordinary commands and consume committed reports.

No client, strategy, execution engine, adapter, or agent may mutate a market engine through an internal reference.

## Source and license rules

- Production manifests use first-party packages or approved dependencies, never paths under `ref/`.
- Preserve exact repositories, commits, paths, and licenses for copied/adapted material.
- Prefer stable upstream APIs over copied implementation.
- Update `docs/reference-functionality-audit.md` before changing a reference’s role or adoption disposition.
- NBC JAR translation and redistribution are authorized by ADR 0017 with file-level provenance and divergence records. Other unlicensed NBC material and QUARCC sources remain restricted to their documented authority/license rules.
- Specification-derived protocol files can have obligations different from the implementation code; review both.
- Worker-bound packages must compile for `wasm32-unknown-unknown` unless explicitly native-only and excluded from the Worker dependency graph.
- Keep fixed-point and checked arithmetic at market, protocol, execution, and ledger boundaries.
- Keep all request, event, snapshot, queue, subscription, and recovery buffers bounded.
- Do not commit `target/`, Worker `build/`, `out/`, database, credential, or secret files.

## Required checks

Run from repository root before marking work complete:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

For reference changes, also verify gitlink pins, licenses, manifests/features, and the audit/adoption documents.

For path changes, also verify Worker build output, migration discovery, release assembly under ignored `out/`, stale-path searches, dependency direction, and active documentation of NBC/QUARCC roles.
