# Repository reorganization and Codex execution plan

Status: corrected planning baseline for the next focused pull request

Last reviewed: 2026-07-12

This plan starts from `main` after PR #3. It implements ADR 0014 and reflects the intended architecture:

- `packages/` contains the reusable Rust components that compose Bunting;
- `bunting-rs/` is the integrated product/composition project;
- NBC is a complete market-engine package;
- the QUARCC trading engine is an optional participant-side execution-engine package;
- generated Wasm and release bundles go to ignored `out/` paths and GitHub Releases.

The reorganization pull request must remain behavior-preserving. Engine ports and package renames follow in separate pull requests.

## Core interpretation

### NBC

NBC is not merely a set of scenarios. The Rust port is a market engine that owns venue/simulation behavior: run lifecycle, logical clock, scenario execution, agent populations, deterministic randomness, order processing, market data, snapshots/replay, and NBC compatibility.

The NBC engine may reuse shared Bunting primitives and the common order-book package when they preserve required behavior. It remains one coherent engine package even when implemented with internal modules or support crates.

### QUARCC trading engine

The QUARCC trading engine is not the venue or market kernel. It runs for a user, trader, or strategy outside the market. It consumes market data, manages local and venue order state, performs participant-side risk and reconciliation, tracks positions, and routes commands through Bunting client APIs, FIX, or other gateways.

Bunting must run without it. Users may enable it to test execution and trading behavior against Bunting market engines.

See [`docs/adr/0014-market-and-execution-engine-boundaries.md`](adr/0014-market-and-execution-engine-boundaries.md).

## Workspace decision

Keep the repository root as one virtual Cargo workspace with one committed `Cargo.lock` and workspace-wide `.cargo/config.toml`.

Use these ownership levels:

1. `packages/`: reusable Rust component packages that can be tested and versioned independently;
2. `bunting-rs/`: the integrated Bunting library/product that composes packages;
3. `bunting-rs/crates/`: Bunting-private implementation crates only when code is not reusable as a package;
4. `apps/`: deployable entrypoints that depend on `bunting-rs`;
5. `out/`: generated release assembly, ignored by Git and uploaded by CI.

Do not create a nested Cargo workspace inside `bunting-rs`. The root `Cargo.toml` owns every member, including `packages/*`, `bunting-rs`, `bunting-rs/crates/*`, and `apps/*`.

Cargo workspaces share one lockfile and output directory. The root workspace prevents dependency drift while allowing independently structured packages.

## Target tree

```text
/
├── Cargo.toml                         # one virtual Cargo workspace
├── Cargo.lock                         # one committed lockfile
├── .cargo/
│   └── config.toml                    # workspace-wide Wasm config
├── AGENTS.md
├── README.md
├── packages/                          # reusable components composing Bunting
│   ├── market-types/
│   ├── market-events/
│   ├── orderbook/                     # shared adapter; fork only if approved
│   ├── ledger/
│   ├── risk-engine/
│   ├── origin-store/
│   ├── command-transaction/
│   ├── worker-cache/
│   ├── nbc-market-engine/             # complete NBC Rust market engine
│   ├── quarcc-execution-engine/       # optional external trader engine
│   ├── fix/                           # future FIX codec/session family
│   ├── client/                        # future Bunting client package
│   └── ...narrow simulator/algorithm packages
├── bunting-rs/                        # integrated Bunting project
│   ├── AGENTS.md
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs
│   │   ├── engine.rs                  # engine registry/selection
│   │   ├── run.rs
│   │   └── config.rs
│   └── crates/                        # private glue only when justified
├── apps/
│   └── edge-api/                      # deployable Cloudflare Worker
├── scenarios/                         # scenario documents and fixtures
│   └── nbc/
├── schemas/                           # JSON Schema, Proto, FIX/SBE schemas
├── tests/                             # cross-package/end-to-end conformance
├── tools/                             # xtask/release/repository tooling
├── docs/
│   ├── adr/
│   └── ports/
├── ref/                               # read-only, commit-pinned evidence
├── vendor/                            # approved built third-party source only
└── out/                               # generated release bundles; ignored
```

Do not create placeholder directories except when an instruction file or initial owned source is introduced.

## Directory ownership

| Path | Owns | Must not own |
|---|---|---|
| `packages/` | Reusable Rust components composing Bunting | Product-only wiring, deployment secrets, generated release output |
| `packages/nbc-market-engine` | Complete NBC market/simulation engine | Trader-side OMS or Bunting deployment adapters |
| `packages/quarcc-execution-engine` | Optional participant-side execution/OMS engine | Venue matching or authoritative Bunting market state |
| `bunting-rs/` | Public Bunting API, package composition, engine selection, run orchestration | Copies of reusable package logic |
| `bunting-rs/crates/` | Product-private glue with no independent package use | General primitives that belong under `packages/` |
| `apps/` | Deployable binaries, Workers, CLIs, and gateways | Canonical domain definitions or reusable engine logic |
| `scenarios/` | Human-reviewable scenario data and provenance | NBC runtime implementation |
| `schemas/` | Versioned wire/file schemas | Handwritten business logic |
| `tests/` | Cross-package, cross-engine, protocol, and deployment tests | Unit tests owned by one package |
| `ref/` | Source evidence, exact revisions, examples, licenses | Production path dependencies |
| `vendor/` | Approved local third-party source with notices and patches | General reference repositories |
| `out/` | Reproducible release artifacts | Source of truth or compiler cache |

## Package rules

A top-level package must have:

- one clear responsibility;
- a stable package name and documented public surface;
- explicit dependency direction;
- workspace-inherited metadata and lints;
- native and, when relevant, Wasm tests;
- an `AGENTS.md` when special rules apply;
- no dependency on `apps/` or `bunting-rs`.

Avoid generic dumping grounds such as `common`, `utils`, or one broad `algorithms` crate. Use narrowly named packages such as `market-making-models`, `arrival-processes`, or `execution-reconciliation` when the first implementation justifies them.

## Market-engine composition

`bunting-rs` must select a market engine explicitly per run. The run record stores an engine identifier and versioned engine configuration.

Initial engine families:

- `orderbook-v1`: current Bunting engine using the OrderBook-rs-backed market path;
- `nbc-v1`: Rust NBC market engine.

The shared market-engine boundary should cover deterministic commands, time/step advancement, market data, snapshots, restore, state hashes, and canonical event translation. Engine-specific configuration must remain typed.

## OrderBook-rs placement

The current production path uses released `orderbook-rs = 0.10.3` through Bunting's adapter.

During the mechanical reorganization:

- move the existing adapter to `packages/orderbook`;
- keep using the released upstream dependency;
- do not create a fork merely because the package directory exists.

If a fork becomes necessary for NBC compatibility or a release-blocking Wasm issue, approve it separately. The fork may live under `packages/orderbook-rs` when it is an intentional maintained component of the Bunting package set, or under `vendor/` when treated as patched third-party source. The ADR approving the fork must choose one model and document upstream synchronization, licensing, patches, and compatibility tests.

## Current-to-target move map

The first reorganization PR is mechanical and preserves Cargo package names.

| Current path | Target path | Action |
|---|---|---|
| `Cargo.toml` | `Cargo.toml` | Keep root workspace; change member paths |
| `Cargo.lock` | `Cargo.lock` | Keep one lockfile |
| `.cargo/config.toml` | `.cargo/config.toml` | Keep at workspace root |
| `crates/market-types` | `packages/market-types` | `git mv`; preserve package name |
| `crates/market-events` | `packages/market-events` | `git mv`; preserve package name |
| `crates/orderbook` | `packages/orderbook` | `git mv`; preserve package name and upstream dependency |
| `crates/ledger` | `packages/ledger` | `git mv`; preserve package name |
| `crates/risk-engine` | `packages/risk-engine` | `git mv`; preserve package name |
| `crates/origin-store` | `packages/origin-store` | `git mv`; preserve package name |
| `crates/command-transaction` | `packages/command-transaction` | `git mv`; preserve package name |
| `crates/worker-cache` | `packages/worker-cache` | `git mv`; preserve package name |
| `crates/quarcc-trading-engine` | `packages/quarcc-trading-engine` | Move mechanically; rename/expand later |
| `workers/edge-api` | `apps/edge-api` | `git mv`; update Cargo, Wrangler, migrations, docs, CI |
| none | `bunting-rs` | Add integrated composition crate, initially thin |
| `scenarios/nbc` | `scenarios/nbc` | Keep scenario documents separate from engine runtime |
| generated Worker output | `out/edge-api/<version>/` | Generate and ignore; upload to Releases |

The initial move does not yet create `packages/nbc-market-engine`. Add that package in the NBC port PR with real source and tests. Do not represent NBC only through scenario packages.

## Follow-up semantic package changes

After the mechanical move:

1. expand and rename `packages/quarcc-trading-engine` to `packages/quarcc-execution-engine`, preserving a migration path for the current `quarcc.v1` compatibility API;
2. create `packages/nbc-market-engine` as the coherent Rust port of NBC;
3. add a common market-engine contract or registry only after comparing the actual needs of the default and NBC engines;
4. add FIX and client package families when implementation begins;
5. introduce a maintained OrderBook-rs fork only through a dedicated ADR.

## Non-goals of the mechanical PR

- no engine behavior changes;
- no Cargo package-name changes;
- no NBC runtime implementation;
- no QUARCC OMS/execution implementation beyond existing code;
- no dependency upgrades;
- no D1 schema changes;
- no order-type or streaming changes;
- no source copied from `ref/`;
- no tracked `target/`, Worker `build/`, `out/`, or raw Wasm artifacts.

## Codex execution contract

### 1. Preflight

1. Start from latest `main` containing ADR 0014 and this document.
2. Confirm the worktree is clean.
3. Read root `AGENTS.md`, this plan, `docs/architecture.md`, `docs/reference-adoption.md`, ADR 0013, ADR 0014, and scoped instructions under every moved path.
4. Create `chore/repository-layout`; do not reorganize directly on `main`.
5. Capture:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo test --locked --workspace
cargo check --locked --workspace --target wasm32-unknown-unknown
```

### 2. Move reusable crates to packages

Use `git mv` for each current `crates/*` member listed in the move map. Preserve package names, public Rust paths, source contents, and tests. Do not combine or split packages in this PR.

Move `crates/quarcc-trading-engine` to `packages/quarcc-trading-engine` without renaming it yet. Update its instructions to state that it is the seed of an optional external execution engine.

### 3. Add the Bunting composition project

Create:

```text
bunting-rs/
  AGENTS.md
  Cargo.toml
  src/lib.rs
```

The initial package should depend on and re-export curated stable package APIs. It may define engine identifiers/configuration scaffolding, but it must not duplicate package implementation or introduce a premature generic engine abstraction.

### 4. Move the deployable Worker

Move `workers/edge-api` to `apps/edge-api` with history preserved. Repair relative dependencies, `wrangler.toml`, migration discovery, CI checks, and deployment commands.

### 5. Repair all paths atomically

Update:

- root workspace members and workspace dependencies;
- every package path dependency;
- `.github/workflows/*.yml` checks;
- Worker/Wrangler paths and migration commands;
- README, architecture, port notes, and active ADR clarifications;
- scoped `AGENTS.md` files;
- scripts, fixtures, and release tooling.

Search the full tracked tree:

```bash
git grep -n 'crates/'
git grep -n 'workers/edge-api'
git grep -n 'workers/'
git grep -n 'dist/'
git grep -n 'quarcc-trading-engine'
```

Historical ADR text may retain old paths when clearly marked historical. Active commands and ownership descriptions must use new paths.

### 6. Release output

Add or update a script/`xtask` that:

1. builds `apps/edge-api` in release mode;
2. collects the generated JavaScript shim, Wasm module, and required metadata;
3. writes `out/edge-api/<version>/`;
4. emits SHA-256 checksums and a manifest containing commit, toolchain, target, and package versions;
5. leaves `out/` ignored.

A later workflow uploads the directory as an Actions artifact and attaches it to version-tagged GitHub Releases.

### 7. Validation

Run from root:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

Also verify:

- workspace members changed only by path plus the new `bunting-rs` package;
- package names did not change;
- no production manifest references `ref/`;
- no package depends on `bunting-rs` or `apps/`;
- `bunting-rs` depends inward on packages;
- migration discovery and Worker build still work;
- no generated artifact is tracked;
- active docs no longer describe `packages/` as non-Rust-only;
- active docs identify NBC as a market engine and QUARCC as an execution engine;
- CI passes.

### 8. Commit shape

Prefer reviewable commits:

1. `chore: move reusable Rust crates under packages`
2. `chore: move edge API under apps`
3. `feat: add bunting composition crate`
4. `docs: align package and engine boundaries`

The PR must include before/after trees, a move map, exact validation output, and an explicit statement that runtime semantics did not change.

## Next engineering work

### P0: mechanical repository reorganization

Execute this plan only.

### P1: NBC market-engine port foundation

- inventory the complete NBC engine surface, not only scenarios;
- resolve ownership/license or define a clean-room behavioral specification;
- define package modules for run lifecycle, market configuration, clock, agents, order handling, market data, snapshots, scoring, and compatibility;
- implement deterministic scenario parsing and the first end-to-end market run;
- compare behavior against captured NBC fixtures;
- expose NBC as an explicit Bunting run engine.

### P2: QUARCC execution-engine port

- preserve the existing `quarcc.v1` compatibility surface;
- implement portable order lifecycle, ID mapping, desired/live reconciliation, positions, participant risk, feed handling, and journaling;
- isolate native gRPC, FIX, SQLite, and socket adapters;
- connect it through the Bunting client as an optional user engine;
- test it against both a deterministic test market and Bunting engines.

### P3: staging and run provisioning

Provision D1/secrets, add authenticated engine-aware run creation, and test default and NBC engine selection.

### P4: streaming and broader market capabilities

Add committed market-data streaming, upstream order capabilities, replay/upgrade tests, and engine-specific market-data compatibility.

### P5: FIX, clients, algorithms, and release distribution

Add focused packages as implementations become real; publish complete Worker bundles and selected reusable packages.

## Branch disposition

| Branch | Relationship to `main` | Recommendation |
|---|---|---|
| `feat/command-transaction-origin-store` | PR #3 merged; remote ref absent | No action |
| `feat/orderbook-rs-worker-kernel` | zero commits ahead; contained in `main` | Safe to delete |
| `feat/bootstrap-architecture` | zero commits ahead; contained in `main` | Safe to delete |
| `feat/deterministic-kernel-vertical-slice` | one unique commit, substantially behind, superseded custom matcher | Tag/archive for history, then delete |

Do not infer safety for unlisted branches. Compare each branch with `main`; delete only when it is zero commits ahead or its unique work has been intentionally preserved.