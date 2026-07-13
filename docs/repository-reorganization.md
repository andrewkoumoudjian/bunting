# Repository reorganization and Codex execution plan

Status: source-audited planning baseline for the next focused pull request

Last reviewed: 2026-07-12

Architecture note: this document records the completed mechanical reorganization. ADR 0018 supersedes its later selectable-engine target; active implementation follows [`plans/unified-bunting-engine-roadmap.md`](plans/unified-bunting-engine-roadmap.md).

This plan starts from `main` after PR #3 and the reference functionality audit. The reorganization pull request is mechanical and behavior-preserving. It does not implement NBC, expand QUARCC, select a FIX stack, fork OrderBook-rs, or change runtime semantics.

Required reading:

- `AGENTS.md`;
- `docs/reference-functionality-audit.md`;
- `docs/reference-adoption.md`;
- `docs/architecture.md`;
- ADR 0013;
- ADR 0014;
- scoped `AGENTS.md` files under every moved path.

## Audited architecture interpretation

### Market engines

- The current default Bunting market engine uses released `OrderBook-rs = 0.10.3` through a first-party adapter.
- NBC is a complete venue-side market-engine port target.
- The direct NBC snapshot proves its packaged application/config/scenarios and observable external protocol. ADR 0017 separately authorizes the pinned JAR as the source/reference runtime for the Rust market-engine translation, with provenance and differential evidence required.

### Participant execution and strategy packages

- QUARCC is an optional external participant execution/OMS engine.
- RITC market making, NautilusTrader, Barter, market-maker-rs and the NBC student client are participant-side strategy/execution/client references, not market engines.

### Protocol references

IronFix, fixer, FerrumFIX, QuickFIX/J and IronSBE have different codec, dictionary, session, store, transport, generation and runtime layers. The mechanical reorganization must not create one empty catch-all `packages/fix` or `packages/sbe` directory before concrete components are selected.

## Workspace decision

Keep one virtual Cargo workspace at the repository root with:

- one committed `Cargo.lock`;
- workspace-wide `.cargo/config.toml`;
- all first-party Rust packages and apps as workspace members;
- `ref/` and `vendor/` excluded.

Do not create a nested workspace inside `bunting-rs`.

## Target ownership levels

1. `packages/`: reusable first-party Rust packages that compose Bunting;
2. `bunting-rs/`: integrated product/composition package;
3. `bunting-rs/crates/`: justified product-private glue only;
4. `apps/`: deployable entrypoints;
5. `scenarios/`: versioned human-reviewable scenario data/provenance;
6. `schemas/`: versioned protocol/file schemas when implemented;
7. `tests/`: cross-package and black-box conformance;
8. `tools/`: repository/release automation;
9. `ref/`: read-only evidence;
10. `vendor/`: approved copied/patched third-party source;
11. `out/`: generated ignored release assembly.

## Target tree after the mechanical PR

```text
/
├── Cargo.toml
├── Cargo.lock
├── .cargo/
│   └── config.toml
├── AGENTS.md
├── README.md
├── packages/
│   ├── market-types/
│   ├── market-events/
│   ├── orderbook/                    # first-party adapter; upstream remains a dependency
│   ├── ledger/
│   ├── risk-engine/
│   ├── origin-store/
│   ├── command-transaction/
│   ├── worker-cache/
│   └── quarcc-trading-engine/        # mechanical name only; semantic rename later
├── bunting-rs/
│   ├── AGENTS.md
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── apps/
│   └── trpc-api/
├── scenarios/
│   └── nbc/
├── tests/
├── docs/
├── ref/
├── vendor/
└── out/                              # generated and ignored
```

Do not create empty future packages. In particular, the mechanical PR does not create:

- `packages/nbc-market-engine`;
- `packages/quarcc-execution-engine` under its final name;
- FIX/SBE packages;
- algorithm/model packages;
- an OrderBook-rs fork;
- empty client or simulator packages.

They are introduced only with real source, tests and an approved boundary.

## Directory ownership

| Path | Owns | Must not own |
|---|---|---|
| `packages/` | Reusable first-party Rust components | Copied third-party source, deployment configuration, product-only wiring |
| `bunting-rs/` | Public composition API and selected package re-exports | Copies of reusable package logic or a second matcher |
| `bunting-rs/crates/` | Product-private glue with no independent consumer | General primitives or future placeholders |
| `apps/` | Deployable Worker/binary/CLI/gateway entrypoints | Canonical domain definitions or matching logic |
| `scenarios/` | Scenario documents, fixtures and provenance | Executable NBC engine behavior |
| `tests/` | Cross-package/engine/protocol/deployment tests | Unit tests owned by one package |
| `ref/` | Exact external evidence and research snapshots | Production dependencies |
| `vendor/` | Approved copied/patched upstream source with notices and patches | First-party code or general references |
| `out/` | Reproducible release bundles | Source of truth or build cache |

## First-party package rules

A package needs:

- one implemented responsibility;
- documented public surface and dependency direction;
- workspace-inherited metadata/lints;
- native tests and Wasm tests where relevant;
- scoped instructions for special constraints;
- no dependency on `bunting-rs` or `apps/`.

Avoid generic `common`, `utils`, `algorithms`, `protocols`, `fix` or `sbe` dumping grounds. Select concrete names only after implementation proves a boundary.

## OrderBook-rs rule

The existing `crates/orderbook` becomes `packages/orderbook`. It remains the Bunting adapter around the released dependency.

A source fork is not part of this reorganization. Handle a future upstream issue through:

1. configuration/features;
2. upstream contribution;
3. released upstream fix;
4. a dedicated pinned fork repository;
5. `vendor/orderbook-rs` only when an approved in-repository patched copy is necessary.

Do not copy upstream source under `packages/orderbook`.

## Current-to-target move map

| Current path | Target path | Mechanical action |
|---|---|---|
| `Cargo.toml` | `Cargo.toml` | Keep workspace root; update member/dependency paths |
| `Cargo.lock` | `Cargo.lock` | Keep one lockfile; regenerate only through Cargo if path metadata changes |
| `.cargo/config.toml` | `.cargo/config.toml` | Keep at root |
| `crates/market-types` | `packages/market-types` | `git mv`; preserve Cargo package name |
| `crates/market-events` | `packages/market-events` | `git mv`; preserve package name |
| `crates/orderbook` | `packages/orderbook` | `git mv`; preserve package name and released dependency |
| `crates/ledger` | `packages/ledger` | `git mv`; preserve package name |
| `crates/risk-engine` | `packages/risk-engine` | `git mv`; preserve package name |
| `crates/origin-store` | `packages/origin-store` | `git mv`; preserve package name |
| `crates/command-transaction` | `packages/command-transaction` | `git mv`; preserve package name |
| `crates/worker-cache` | `packages/worker-cache` | `git mv`; preserve package name |
| `crates/quarcc-trading-engine` | `packages/quarcc-trading-engine` | Move as-is; preserve package/API name |
| `workers/edge-api` | `apps/edge-api` | `git mv`; update Cargo, Wrangler, migrations, docs and CI |
| none | `bunting-rs` | Add one thin composition crate with no duplicated logic |
| `scenarios/nbc` | `scenarios/nbc` | Keep |
| generated Worker output | `out/trpc-api/<version>/` | Generate, ignore and upload through release tooling |

Do not move stub-only directories into the active package set. Review each scaffold separately after the mechanical PR; delete or retain instructions according to the audited roadmap.

## Non-goals

- no Cargo package renames;
- no behavior or public Rust API changes beyond a new thin composition package;
- no NBC implementation;
- no QUARCC execution implementation beyond the existing compatibility crate;
- no protocol-stack selection;
- no dependency upgrades;
- no D1 migration/schema changes;
- no new order types or streaming;
- no source copied from `ref/`;
- no generated `target/`, Worker `build/`, `out/` or raw Wasm committed;
- no branch deletion inside the reorganization PR.

# Codex execution contract

## 1. Preflight and evidence

1. Start from the latest `main` containing the source-backed reference audit.
2. Confirm the worktree and submodule state are clean.
3. Read all required documents and scoped instructions.
4. Create `chore/repository-layout`; do not reorganize directly on `main`.
5. Record the exact reference state without initializing/updating it:

```bash
git submodule status
git ls-tree HEAD ref
git status --short
```

6. Capture the pre-move workspace:

```bash
cargo metadata --locked --format-version 1 --no-deps > /tmp/bunting-metadata-before.json
cargo test --locked --workspace
cargo check --locked --workspace --target wasm32-unknown-unknown
```

## 2. Move existing reusable crates only

Use `git mv` for each implemented workspace crate in the move map. Preserve:

- Cargo package names;
- public Rust paths;
- source/tests;
- scoped instructions;
- lockfile dependency versions;
- runtime behavior.

Do not combine, split or semantically rename packages.

## 3. Add the thin Bunting composition package

Create:

```text
bunting-rs/
  AGENTS.md
  Cargo.toml
  src/lib.rs
```

The initial crate may:

- re-export a deliberately small stable set of first-party types;
- expose product/version metadata;
- provide nonfunctional engine-ID/config scaffolding only if it does not invent an abstraction.

It may not:

- duplicate command transaction or matching logic;
- expose Worker-only adapters by default;
- claim NBC or QUARCC is implemented;
- introduce a nested workspace.

## 4. Move the deployable Worker

Move `workers/edge-api` to `apps/edge-api` with history preserved. Update:

- relative path dependencies;
- root workspace members;
- `wrangler.toml` build/migration context;
- deployment/migration/secret commands;
- CI architecture-policy checks;
- scoped instructions;
- documentation and scripts.

## 5. Repair all path consumers atomically

Search the full tracked tree:

```bash
git grep -n 'crates/'
git grep -n 'workers/edge-api'
git grep -n 'workers/'
git grep -n 'quarcc-trading-engine'
git grep -n 'dist/'
git grep -n 'out/'
```

Classify each hit as:

- active path that must change;
- historical ADR text that remains intentionally historical;
- reference-source path that must not change;
- stale text to remove.

Do not use global blind replacement inside `ref/`, historical diffs or archived planning evidence.

## 6. Release boundary

A repository tool may be added to:

1. run the existing Worker release build from `apps/trpc-api`;
2. collect the generated JavaScript shim, Wasm module and required metadata;
3. write `out/trpc-api/<version>/`;
4. emit SHA-256 checksums and a manifest with commit, toolchain, target and package versions;
5. leave `out/` ignored.

A later workflow uploads that complete bundle. A raw Wasm module alone is not the deployable Worker entrypoint.

## 7. Validation

Run from root:

```bash
cargo metadata --locked --format-version 1 --no-deps > /tmp/bunting-metadata-after.json
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

Verify additionally:

- workspace members changed only by paths plus the new `bunting-rs` package;
- Cargo package names and dependency versions are unchanged;
- no production manifest references `ref/`;
- no package depends on `bunting-rs` or `apps/`;
- `bunting-rs` depends inward only on first-party packages;
- Worker build and D1 migration discovery work from `apps/trpc-api`;
- no generated artifact is tracked;
- no copied upstream source entered `packages/`;
- `docs/reference-functionality-audit.md` and reference pins are unchanged unless the PR explicitly explains an evidence-only correction;
- CI passes.

## 8. Commit and PR shape

Prefer:

1. `chore: move reusable Rust crates under packages`
2. `chore: move edge API under apps`
3. `feat: add thin bunting composition crate`
4. `docs: align active paths and commands`

The PR includes:

- before/after trees;
- exact move map;
- before/after Cargo metadata member lists;
- validation output;
- stale-path search results;
- explicit statement that runtime semantics, package names and dependency versions did not change.

# Work after reorganization

## P1: NBC evidence and market-engine foundation

- finish the external-contract fixture manifest;
- apply ADR 0017 authorization and preserve translation/redistribution provenance;
- create `packages/nbc-market-engine` with strict config/provenance only when real source/tests are added;
- specify `nbc-v1` matching/order/step capabilities without claiming hidden Java equivalence;
- implement deterministic Bunting-added recovery and state hashing;
- add agent/model behavior incrementally with provenance.

## P2: QUARCC execution-engine port

- retain the `quarcc.v1` contract;
- finish the evidence-linked transition table;
- implement portable exact-unit lifecycle/report handling;
- add Bunting-defined reconciliation/recovery explicitly;
- isolate native gRPC/store/gateway packages;
- test through public adapters against fake/default/NBC markets.

## P3: staging and run provisioning

Provision D1/secrets and add authenticated engine-aware run creation without conflating market-engine selection with participant execution-engine enablement.

## P4: streaming and broader default-engine capabilities

Add committed market/private streams, reset recovery and upstream order features under typed capabilities.

## P5: concrete protocol/client/model packages and releases

Select protocol layers through focused spikes, then create narrowly named packages with real implementation. Publish complete Worker bundles and approved reusable packages.

# Branch disposition

| Branch | Relationship to `main` | Recommendation |
|---|---|---|
| `feat/command-transaction-origin-store` | PR #3 merged; remote ref absent | No action |
| `feat/orderbook-rs-worker-kernel` | zero commits ahead; contained in `main` | Safe to delete |
| `feat/bootstrap-architecture` | zero commits ahead; contained in `main` | Safe to delete |
| `feat/deterministic-kernel-vertical-slice` | one unique commit, substantially behind, superseded custom matcher | Tag/archive for history, then delete |

For unlisted branches, compare against current `main`. Delete only when zero commits ahead or after every unique commit is merged, cherry-picked, or intentionally preserved.
