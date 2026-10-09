# Bunting agent instructions

## Mission

Build a Rust market-simulation and exchange-testing platform composed from reusable packages: one headless, deterministic `bunting-engine` that owns market truth, served by one native venue process to participants over FIX and the certified Bunting Native Protocol. Two products sit on that engine: a RIT-class instructor/student simulator and a QUARCC competition venue. Cloudflare is only a read-only publisher of immutable exports.

## Instruction precedence

- Read this file before changing the repository.
- Read the nearest scoped `AGENTS.md` for every path touched.
- Read [`docs/README.md`](docs/README.md) (documentation status map) before relying on any document under `docs/`. Documents marked **Historical** are evidence, never instructions, even when their own text says "binding", "active" or "non-negotiable".
- Accepted ADRs and `docs/architecture.md` are binding. Read an ADR's **status line** first; ADR 0033 records which older ADRs are superseded in whole or in part.
- Key decisions: ADR 0018 (one production engine; carries ADR 0014's market-versus-participant authority split), ADR 0028 (single ledger, live state, full replay), ADR 0029 (engine-owned order book), ADR 0030 (latency-modeled continuous admission — target), ADR 0031 (FIX + certified native protocol only — target), ADR 0032 (NBC is reference evidence only — engine surface removed in slice 11), ADR 0033 (reconciliation and host direction).
- **Target ≠ implemented.** ADRs 0030–0031 and the Target sections of `docs/architecture.md` are accepted direction, not current behavior. Check `docs/implementation-log/` before describing anything as done.
- Current execution order: [`docs/research/2026-10-09-exploration-and-next-steps.md`](docs/research/2026-10-09-exploration-and-next-steps.md) §8. Slice definitions: [`docs/plans/2026-10-07-evidence-led-core-roadmap.md`](docs/plans/2026-10-07-evidence-led-core-roadmap.md).
- Read `docs/reference-functionality-audit.md` before using, moving, porting, comparing, or describing anything under `ref/` or `vendor/`.
- Read `docs/reference-adoption.md` before adding a dependency, source adaptation, fork, vendored file, or conformance oracle.

When documents conflict, do not silently select the convenient interpretation. Reconcile the active ADR, source-backed audit, and implementation before changing code. If the conflict is not covered by an ADR, stop and record it rather than guessing.

## Evidence discipline

For every reference claim, distinguish:

1. **observed:** proved by the recorded source, manifest, contract, test, or captured external behavior;
2. **inferred:** a reasoned interpretation that is not directly proved;
3. **Bunting-added:** a new requirement or design choice;
4. **unresolved:** missing source, license, units, formulas, ordering, or behavior;
5. **prohibited to copy:** source or specification material lacking the required authorization/license.

Never infer functionality from a repository name. Never treat `.gitmodules` branch metadata as the checked-out commit. Verify submodule pins with `git ls-tree HEAD` and `git -C ref/<name> rev-parse HEAD`.

The same discipline applies to Bunting itself: a performance claim needs a recorded measurement (workload, hardware, build profile, commit); a "tests pass" claim needs the command that ran. Record both in `docs/implementation-log/`.

## Engine roles

### Market engine

A single production `bunting-engine` owns venue/simulation authority: run state, logical time, listings, market configuration, order processing, risk admission, the single economic ledger, trades, scoring, canonical events, public market-data projections, and the recovery contract required by Bunting.

- The engine package owns its private deterministic price-time book (`packages/bunting-engine/src/book.rs`, ADR 0029); applications and orchestration packages must not reach the book except through canonical engine commands and read projections.
- NBC is reference evidence only (ADR 0032). It is not a compatibility target. The engine's NBC compatibility module, the `NbcDone` command and the NBC events were removed in slice 11; do not reintroduce them. Useful ideas from NBC re-enter only as Bunting-native features with Bunting semantics and tests.
- The NBC JAR's licensing record (ADR 0017) still governs any reading or quotation of that material.

### QUARCC execution engine

The QUARCC trading engine is an optional external participant-side execution/OMS engine for users, traders, and strategies. Its recorded source includes strategy signals, submit/cancel/replace, order managers, gateway/feed boundaries, participant risk, ID mapping, journal/store abstractions, positions, kill switch, market-data streaming, gRPC and Python clients.

It must never become authoritative market state or directly mutate a market engine. Bunting must run without it. The existing `quarcc.v1` compatibility crate is the first surface of the port, not its final scope. Built-in agents compose with it (`packages/bunting-agents`).

### Other participant-side references

RITC market making, NautilusTrader, Barter, market-maker-rs, and the NBC student client are participant strategy/execution/client systems. They are not venue matching engines.

## Repository organization

The repository root remains one Cargo workspace and owns the single `Cargo.lock` and workspace-wide `.cargo/config.toml`.

- `packages/`: first-party reusable Rust packages that compose Bunting. This includes primitives, the unified market engine, execution engines, protocol components, clients, simulators, and narrowly scoped algorithm/model libraries.
- `bunting-rs/`: integrated Bunting product/library that imports packages, configures the unified engine, and exposes the curated public API.
- `bunting-rs/crates/`: Bunting-private glue only when code has no reusable package role.
- `apps/`: deployable binaries, CLIs, and gateways that depend on `bunting-rs` or public package APIs.
- `bindings/`: language bindings over the `bunting-rs` façade (ADR 0026).
- `scenarios/`: human-reviewable scenario documents, fixtures, and provenance.
- `schemas/`: versioned protocol and file schemas.
- `tests/`: cross-package, protocol, oracle and deployment tests.
- `tools/`: repository automation and release tooling.
- `ref/`: read-only source evidence and provenance; never a production path dependency.
- `vendor/`: approved copied/patched third-party source with license, exact upstream revision, notices, and patch log.
- `out/`: generated release bundles; ignored and never source of truth.

Do not create a nested Cargo workspace in `bunting-rs`. The root workspace includes `packages/*`, `bunting-rs`, justified private Bunting crates, `apps/*` and `bindings/*`.

## Package discipline

- Reusable first-party code belongs under `packages/`, not product-private directories.
- A package must have one clear responsibility, explicit dependency direction, workspace metadata/lints, tests, and scoped instructions when needed.
- Packages must not depend on `bunting-rs` or `apps/`; dependency flow is packages -> `bunting-rs` -> apps.
- Avoid generic `common`, `utils`, `algorithms`, `fix`, or `protocols` dumping grounds. Name packages after a concrete responsibility such as `fix-tagvalue`, `fix-session`, `execution-reconciliation`, or `market-making-models` when implementation justifies them.
- Keep mechanical moves separate from semantic renames and feature work.
- Use `git mv`, preserve package names during moves, and repair Cargo, CI, release tooling, docs, scripts, scoped instructions and `docs/README.md` atomically.
- Do not create empty package directories to represent future ideas.

## Binding architecture decisions

- OrderBook-rs is a dev-dependency differential oracle only; it must never return as a production dependency without a superseding ADR.
- Keep exactly one book implementation. New order types need Bunting-level semantics, book tests and oracle coverage where an oracle exists before entering the command schema.
- One native venue process is the only market authority: it accepts bounded participant sessions and calls application functions in-process (ADR 0022). Cloudflare publishes immutable leaderboards, run archives and public snapshots; it never accepts participant commands, inbound raw TCP, or owns origin truth. No Cloudflare Worker is currently built; any future publisher reads immutable post-commit exports only.
- Participant interfaces are **FIX and the Bunting Native Protocol only** (ADR 0031). Do not add REST, gRPC, WebSocket or browser command surfaces. Do not extend `browser-wire` or the browser procedures in `bunting-api-contract`; they are retired once BNP covers them.
- Admission target is continuous price-time matching with latency-modeled ordering (ADR 0030). The ADR 0024 interval writer is the current implementation; do not build new features on its sleep-to-boundary behavior.
- Hosting: the release currently packages the server for WASIX (ADR 0027), but WASIX is not a binding long-term host (ADR 0033). Keep the server buildable and testable natively and add no WASIX-only dependencies or code paths.
- Accepted commands, canonical events, idempotency, and optimistic versions remain authoritative in the origin store.
- Commit authoritative state before acknowledgement or any stream/report publication.

## Known traps (verified 2026-10-09)

Do not extend these patterns; each is scheduled for replacement (see the exploration note §5–§8):

- **Adapter-held authority** (fixed in slice 12; do not reintroduce). Per-participant limits are engine risk admission (`RiskLimits.max_live_orders`), and reports reach participants only through the committed-event distributor (`apps/bunting-server/src/distributor.rs`). Never keep limits, order sets or report routing in session state.
- **Transport-local identities.** Session-local counters (QUARCC action/order IDs, FIX ClOrdID-derived IDs) are not canonical IDs. Every adapter must namespace them per participant session before they reach the engine (`bunting-application` does this for FIX since slice 12); two sessions must never be able to produce the same `CommandId` or `OrderId`.
- **Per-command full-state copies** (fixed in slice 14; do not reintroduce). The origin owns each run's live `RunState`, applies inputs in place (`RunState::apply`, `ApplyError::{Unchanged, Poisoned}`) and journals command records (input, result, events, hash chain); full state appears only at genesis and in checkpoints. Read through `OriginStore::read_run` closures; `clone_run` costs O(state) and stays off per-command paths. Never mutate a live run outside `OriginStore::execute`, and never do I/O or call `execute` inside a read closure: it holds the store lock.
- **Inputs outside the record.** Built-in agents commit under `writer.lock()` outside admission, and their runtime state is not persisted. Every cause of a state change must be a recorded, replayable input.
- **Partial replay.** `CompetitionArchive` replays simulation commands only; do not call it a full trading replay.
- **Two clocks.** FIX admissions are stamped from wall-clock epoch milliseconds. New time-dependent features use the run's logical clock.

## Authority boundaries

The Bunting engine owns venue-side identities, matching results, canonical events, the authoritative ledger, scenario/run state, and market-data publication.

Participant-side packages own local order intent, venue reconciliation, strategy state, participant risk, and client/gateway connectivity. They submit ordinary commands and consume committed reports.

No client, strategy, execution engine, adapter, or agent may mutate a market engine through an internal reference.

## Source and license rules

- Production manifests use first-party packages or approved dependencies, never paths under `ref/`.
- Preserve exact repositories, commits, paths, and licenses for copied/adapted material.
- Prefer stable upstream APIs over copied implementation.
- Update `docs/reference-functionality-audit.md` before changing a reference’s role or adoption disposition.
- NBC JAR reading and translation are authorized by ADR 0017 with file-level provenance; NBC is otherwise reference evidence only (ADR 0032). Other unlicensed NBC material and QUARCC sources remain restricted to their documented authority/license rules.
- Specification-derived protocol files can have obligations different from the implementation code; review both.
- `bunting-engine` and the protocol packages must stay host-neutral and compile for `wasm32-unknown-unknown`.
- Keep fixed-point and checked arithmetic at market, protocol, execution, and ledger boundaries.
- Keep all request, event, snapshot, queue, subscription, and recovery buffers bounded.
- Do not commit `target/`, `out/`, database, credential, private key, certificate-authority or secret files.

## Required checks

During implementation, prefer `cargo check -p <package>` and the package's
focused tests before paying for workspace-wide gates. Developers with sccache
installed may set `RUSTC_WRAPPER=sccache`; CI and release correctness must not
depend on sccache being available.

Run from repository root before marking work complete:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
! cargo tree --locked -p bunting-engine -e normal | grep -q 'orderbook-rs'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

For reference changes, also verify gitlink pins, licenses, manifests/features, and the audit/adoption documents.

For path changes, also verify release assembly under ignored `out/`, stale-path searches, dependency direction, `docs/README.md`, and scoped instructions.

For changes that supersede a document or ADR, update `docs/README.md`, add the historical banner, and amend the old ADR's status line in the same commit.

When a slice lands, append it to `docs/implementation-log/` with commits, what changed, which checks ran, and what remains.
