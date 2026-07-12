# Repository reorganization and Codex execution plan

Status: approved planning baseline for the next focused pull request

Last reviewed: 2026-07-12

This plan starts from `main` after PR #3 and is intentionally separate from feature work. The reorganization pull request must be mechanical, reviewable, and behavior-preserving.

## Decision

1. Keep the repository root as the Cargo workspace root.
2. Add `bunting-rs/` as the public Rust facade package, with `src/lib.rs` re-exporting only stable Bunting APIs.
3. Keep internal reusable Rust packages under `crates/`.
4. Move deployable binaries and services under `apps/`; the current `workers/edge-api` package becomes `apps/edge-api`.
5. Reserve root `packages/` for independently versioned, user-facing packages outside the internal Rust crate graph, such as Python or JavaScript SDKs. Do not move internal Rust crates there.
6. Keep canonical scenario documents and data under `scenarios/`; scenario runtime code belongs in Rust crates.
7. Add `schemas/` only when protocol schemas exist, grouped by format or protocol.
8. Keep `ref/` as read-only provenance and research evidence. Production manifests must not depend on it.
9. Keep `vendor/` reserved for source that is actually built locally and whose license, upstream revision, and divergence are documented. It is not a second reference archive.
10. Generate release artifacts under `dist/`; do not commit compiler output. GitHub Releases should carry the deployable Worker bundle, raw Wasm module, checksums, and build metadata.

Cargo workspaces share one lockfile and one output directory at the workspace root. Keeping the root workspace avoids split configuration, duplicated lockfiles, and awkward path dependencies while still allowing `bunting-rs` to provide a conventional public crate entry point. See the Cargo workspace and configuration references:

- <https://doc.rust-lang.org/cargo/reference/workspaces.html>
- <https://doc.rust-lang.org/cargo/reference/config.html>

## Target tree

```text
/
├── Cargo.toml                  # workspace manifest
├── Cargo.lock                  # one committed workspace lockfile
├── .cargo/
│   └── config.toml             # workspace-wide Wasm configuration
├── AGENTS.md
├── README.md
├── bunting-rs/                 # public facade package; package name `bunting`
│   ├── AGENTS.md
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── crates/                     # internal reusable Rust libraries
│   ├── market-types/
│   ├── market-events/
│   ├── orderbook/
│   ├── ledger/
│   ├── risk-engine/
│   ├── origin-store/
│   ├── command-transaction/
│   ├── worker-cache/
│   └── quarcc-trading-engine/  # rename separately; see below
├── apps/                       # deployable programs and services
│   └── edge-api/
├── packages/                   # independently distributed non-core SDKs
├── scenarios/                  # canonical scenario data and provenance
├── schemas/                    # JSON Schema, Protobuf, FIX/SBE schemas when added
├── tests/                      # cross-package conformance and black-box tests
├── tools/                      # repository automation or a future `xtask`
├── docs/
│   ├── adr/
│   └── ports/
├── ref/                        # read-only, commit-pinned references
├── vendor/                     # built local source only, with notices and patches
└── dist/                       # generated, ignored, published through Releases
```

Do not create empty directories merely to match this diagram. Add a directory when the first owned file is introduced.

## Ownership rules

| Path | Owns | Must not own |
|---|---|---|
| `bunting-rs/` | Stable public facade and curated re-exports | Matching logic, Worker routes, persistence adapters, scenario data |
| `crates/` | Reusable Rust libraries with explicit boundaries | Deployable runtime configuration, generated release output |
| `apps/` | Deployable entrypoints and runtime adapters | A second matching engine or canonical domain definitions |
| `packages/` | Independently released SDKs or ecosystem packages | Internal Rust implementation crates |
| `scenarios/` | Canonical scenario documents, fixtures, and provenance | Legacy transport behavior or hidden executable logic |
| `schemas/` | Versioned wire and file schemas | Generated code unless the generator requires it to be committed |
| `tests/` | Cross-crate, protocol, replay, and deployment conformance | Unit tests that belong next to one crate |
| `ref/` | Source evidence, exact revisions, examples, licenses | Production path dependencies |
| `vendor/` | Approved locally built third-party source | Unreviewed copies or general research repositories |
| `dist/` | Reproducible release bundles | Source of truth or committed build cache |

## Current-to-target move map

| Current path | Target path | Action in the reorganization PR |
|---|---|---|
| `Cargo.toml` | `Cargo.toml` | Keep at root; add `bunting-rs` and `apps/edge-api` members |
| `Cargo.lock` | `Cargo.lock` | Keep one root lockfile and regenerate only through Cargo |
| `.cargo/config.toml` | `.cargo/config.toml` | Keep at root so workspace commands always discover it |
| `crates/*` | `crates/*` | Keep paths and Cargo package names unchanged |
| `workers/edge-api` | `apps/edge-api` | Move with `git mv`; update every manifest, workflow, script, doc, migration command, and Wrangler path |
| none | `bunting-rs` | Add a small facade crate; no business logic |
| `scenarios/*` | `scenarios/*` | Keep; update scoped instructions only if path references change |
| `docs/*` | `docs/*` | Keep; repair active links and commands |
| `ref/*` | `ref/*` | Keep read-only and excluded from the workspace |
| `vendor/*` | `vendor/*` | Keep reserved and excluded unless an approved fork is introduced |
| generated Worker output | `dist/edge-api/<version>/` | Generate through release automation; keep ignored |

## Naming decisions deferred to separate pull requests

The mechanical reorganization must preserve Cargo package names and public Rust paths. Renames make review and rollback harder and should follow separately.

- `crates/quarcc-trading-engine` currently provides a compatibility contract, not a matching engine. A later API-focused PR should consider renaming the directory and package to `quarcc-compat`, with any required compatibility transition.
- New FIX work should begin as a codec/protocol crate such as `crates/protocol-fix`; a deployable bridge belongs under `apps/fix-gateway` only when it exists.
- NBC Rust runtime behavior should be separated into explicit scenario, agent-model, and legacy-protocol crates instead of one catch-all port.
- Algorithms shared by the engine belong in narrowly named crates. Example strategies or competition agents belong under scenarios or examples, not in a generic `algorithms` dumping ground.
- Continue using the released `orderbook-rs` dependency. Do not create an in-repository fork unless the documented fork gate is satisfied. An approved local fork belongs under `vendor/orderbook-rs`, not `packages/`.

## Non-goals for the mechanical reorganization

- no matching, risk, ledger, persistence, route, or protocol behavior changes;
- no Cargo package renames;
- no dependency upgrades;
- no D1 schema changes;
- no new order types;
- no streaming implementation;
- no formatting churn outside files whose paths or content must change;
- no copied source from `ref/`;
- no committed `target/`, `build/`, `dist/`, or raw Wasm output.

## Codex execution contract

### 1. Preflight

1. Start from the latest `main` containing this document.
2. Confirm the worktree is clean.
3. Read root `AGENTS.md`, this document, `docs/architecture.md`, `docs/reference-adoption.md`, and all scoped `AGENTS.md` files under paths being moved.
4. Create `chore/repository-layout`; do not implement directly on `main`.
5. Capture the pre-move output of:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo test --locked --workspace
cargo check --locked --workspace --target wasm32-unknown-unknown
```

### 2. Perform only history-preserving moves

1. Create `apps/` and move `workers/edge-api` to `apps/edge-api` with `git mv`.
2. Remove `workers/` only if it is empty.
3. Add `bunting-rs/Cargo.toml`, `bunting-rs/src/lib.rs`, and a concise scoped `AGENTS.md`.
4. The facade may re-export stable identifiers, commands/events, and client-facing transaction types. It must not expose Worker-only adapters by default or duplicate implementation.
5. Do not move `crates/*` into `packages/` or under `bunting-rs/`.

### 3. Repair all path consumers atomically

Update at least:

- root workspace members and workspace path dependencies;
- `apps/edge-api/Cargo.toml` relative paths;
- `.github/workflows/*.yml` architecture-policy checks and working directories;
- Wrangler configuration, migration commands, and deployment documentation;
- README and architecture/path references;
- scoped `AGENTS.md` files;
- scripts, fixtures, or tests containing `workers/edge-api`;
- any release or artifact paths.

Search the entire tracked tree, including documentation:

```bash
git grep -n 'workers/edge-api'
git grep -n 'workers/'
git grep -n 'crates/quarcc-trading-engine'
```

The first two searches must return only intentional historical discussion in this plan or ADRs. Do not rewrite historical ADR decisions merely to remove an old path; add a clarification when history must remain intact.

### 4. Add release-boundary scaffolding without committing output

The reorganization PR may add a repository script or `xtask` command that:

1. runs the existing release Worker build from `apps/edge-api`;
2. collects the complete deployable Worker bundle, including the JavaScript shim and Wasm module;
3. writes it under `dist/edge-api/<version>/`;
4. emits SHA-256 checksums and a small build manifest containing the Git commit, Rust toolchain, target, and package version;
5. leaves `dist/` ignored.

A separate release-workflow PR should upload that directory as a GitHub Actions artifact and attach it to version-tagged GitHub Releases. A raw Wasm file alone is not the complete deployable Worker because Wrangler currently enters through the generated JavaScript shim.

### 5. Validation gates

Run from the repository root:

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

- the member set before and after differs only by the new facade package and the edge-api path;
- no production manifest references `ref/`;
- the D1 migration is still discoverable from the moved Wrangler configuration;
- the Worker build still produces its shim and Wasm module;
- no generated artifact is tracked;
- active documentation contains no stale deployment command;
- CI passes on the pull request.

### 6. Commit and pull request shape

Prefer three reviewable commits:

1. `chore: move edge API under apps`
2. `feat: add bunting Rust facade crate`
3. `docs: align repository paths and release instructions`

The pull request must contain a before/after tree, a move map, the exact validation output, and an explicit statement that runtime semantics did not change.

## Product and engineering work after reorganization

### P0: staging deployment and run provisioning

- create the real D1 database and replace the placeholder database ID through environment-specific configuration;
- apply migrations and install `BUNTING_API_TOKEN`;
- add an explicit administrative run-provisioning boundary for runs, instruments, participants, opening balances, and limits;
- add staging smoke tests for create/provision, submit, cancel, duplicate command, stale expected version, restart recovery, and cache miss;
- document rollback, migration, and secret-rotation procedures.

### P1: committed market-data streaming

- add a plain Worker WebSocket endpoint;
- publish snapshot plus absolute L1/L2 updates only after origin commit;
- use committed event-sequence cursors, reset recovery, bounded subscriptions, frames, and backlog;
- avoid isolate-local resume guarantees.

### P2: broader upstream capabilities

Expose upstream behavior incrementally: IOC, FOK, post-only, replace, mass cancel, STP, fees, expiry, lifecycle history, typed rejects, upstream risk configuration, depth, metrics, impact, and snapshot upgrade verification.

### P3: scenarios and deterministic orchestration

Implement run clocks, named random streams, scenario schemas, NBC provenance, agent models, scoring, and replay tests. Scenario agents must submit ordinary commands rather than mutate the book.

### P4: protocols, SDKs, and distribution

Add the FIX codec/bridge, Nautilus and RITC adapters, public SDK packages, semantic versioning, release bundles, checksums, and compatibility tests.

## Branch disposition

This section records the branches identified from merged pull requests and the earlier Codex implementation branch.

| Branch | Observed relationship to `main` | Recommendation |
|---|---|---|
| `feat/command-transaction-origin-store` | PR #3 merged; remote ref is already absent | No action |
| `feat/orderbook-rs-worker-kernel` | zero commits ahead; fully contained in `main` | Safe to delete now |
| `feat/bootstrap-architecture` | zero commits ahead; fully contained in `main` | Safe to delete now |
| `feat/deterministic-kernel-vertical-slice` | one unique commit and substantially behind; contains the superseded custom matcher path audited in PR #2 | Preserve with an annotated tag or `archive/` ref, then delete the feature branch |

The unique deterministic-kernel commit is not a candidate for merging wholesale. PR #2 already preserved the Bunting-owned identifiers, canonical events, ledger, and participant risk while deliberately rejecting the custom matching engine and Durable Object assumptions. Archive it only as implementation history or a source of focused test ideas.

For any branch not listed here, do not infer safety from age or naming. Compare it with `main`; delete only when it is zero commits ahead or after every unique commit has been merged, cherry-picked, or intentionally preserved by tag with a written disposition.