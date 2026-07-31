# Implementation prompt — mechanical repository reorganization (P0)

> **Status (read first): already implemented on `main`.** By the time this prompt was archived,
> `main` had advanced ~92 commits and the mechanical reorganization described here was **complete**
> (`crates/` → `packages/`, `workers/edge-api` → `apps/`, thin `bunting-rs/` composition crate,
> `out/` ignored + release workflow). `docs/repository-reorganization.md` now records it as done, and
> newer ADRs supersede its later targets: **ADR-0018** (unified Bunting engine), **ADR-0019**
> (`bunting-engine` owns OrderBook-rs), **ADR-0022** (native competition venue + publication Worker),
> through **ADR-0027** (Wasmer WASI server runtime). Active work follows the competition-platform
> track, not this prompt.
>
> This file is retained as a **record of the reconciled reorganization plan** and as a reusable,
> from-scratch reorg prompt. For current work, target the live roadmap/ADRs instead.

You are executing the **mechanical, behavior-preserving** repository reorganization described in
`docs/repository-reorganization.md`. Change layout and paths only — do not change runtime behavior,
Cargo package names, dependency versions, or public Rust APIs (except adding one new thin crate).

## 0. Required reading (read before touching anything)
- `AGENTS.md` (root + every scoped `AGENTS.md` under a moved path)
- `docs/repository-reorganization.md`  ← the governing plan; this prompt drives its P0
- `docs/reference-functionality-audit.md`, `docs/reference-adoption.md`, `docs/architecture.md`
- `docs/adr/0013-worker-orderbook-rs-kernel.md`, `docs/adr/0014-market-and-execution-engine-boundaries.md`

## 1. Preflight
1. Start from the latest `main`. Confirm a clean worktree and clean submodule state
   (`git submodule status`, `git status --short`, `git ls-tree HEAD ref`). Do NOT init/update submodules.
2. Create and switch to branch `chore/repository-layout` (do not reorganize on `main`).
3. Capture the pre-move baseline:
   ```bash
   cargo metadata --locked --format-version 1 --no-deps > /tmp/bunting-metadata-before.json
   cargo test --locked --workspace
   cargo check --locked --workspace --target wasm32-unknown-unknown
   ```

## 2. Move the implemented crates (history-preserving `git mv`, names preserved)
```
git mv crates/market-types          packages/market-types
git mv crates/market-events         packages/market-events
git mv crates/orderbook             packages/orderbook
git mv crates/ledger                packages/ledger
git mv crates/risk-engine           packages/risk-engine
git mv crates/origin-store          packages/origin-store
git mv crates/command-transaction   packages/command-transaction
git mv crates/worker-cache          packages/worker-cache
git mv crates/quarcc-trading-engine packages/quarcc-trading-engine   # mechanical move only; NO rename
git mv workers/edge-api             apps/edge-api
```
All nine `packages/*` crates remain mutual siblings, so their intra-crate `path = "../<name>"`
dependencies are **unchanged** (including `packages/worker-cache`'s `../market-types`).

Do **not** move the ~15 `Cargo.toml`-less stub scaffolds under `crates/` (`agent-models`,
`market-making`, `matching-engine`, `order-reconciliation`, `protocol-*`, `replay-format`,
`scenario-*`, `scoring`, `simfix-*`, `simulation-clock`, `test-fixtures`) into the active package
set. Leave them where they are; they are reviewed separately per the roadmap. Likewise leave
`clients/`, `services/`, `web/`, `scenarios/`, `tests/`, and the consumer-worker stubs under
`workers/` in place.

## 3. Add the thin composition crate `bunting-rs/`
Create `bunting-rs/{Cargo.toml, AGENTS.md, src/lib.rs}`. It MAY re-export a deliberately small,
stable set of first-party types and expose product/version metadata. It MUST NOT duplicate command
-transaction or matching logic, expose Worker-only adapters by default, claim NBC/QUARCC is
implemented, or introduce a nested workspace. It depends **inward** on `packages/*` only
(e.g. `bunting-market-types = { path = "../packages/market-types" }`), never on `apps/`.

## 4. Repair manifests (the only path edits needed)
- **Root `Cargo.toml`**: set `members` (keep explicit — do not glob):
  ```toml
  members = [
    "packages/market-types", "packages/market-events", "packages/orderbook",
    "packages/ledger", "packages/risk-engine", "packages/origin-store",
    "packages/command-transaction", "packages/worker-cache",
    "packages/quarcc-trading-engine",
    "bunting-rs", "apps/edge-api",
  ]
  exclude = ["ref", "vendor", "out"]
  ```
  Update the two path-pinned `workspace.dependencies`:
  `bunting-market-events = { path = "packages/market-events" }`,
  `bunting-market-types  = { path = "packages/market-types" }`.
- **`apps/edge-api/Cargo.toml`** (was `workers/edge-api`): its five `../../crates/*` path deps
  become `../../packages/*` (command-transaction, market-types, orderbook, origin-store,
  worker-cache). `bunting-market-events` and `worker` are workspace-inherited → untouched.
- **`.cargo/config.toml`**: no change (target-global rustflag).
- **`Cargo.lock`**: expect it byte-unchanged (path members are keyed by name, not path). If Cargo
  rewrites it, that is acceptable as long as no dependency *version* changes.

## 5. Move the Worker deploy config with the crate
Under `apps/edge-api/`, keep `wrangler.toml`, `migrations/`, and sources together. worker-build
emits to `apps/edge-api/build/` and `wrangler.toml`'s `main = "build/worker/shim.mjs"` resolves
relative to `apps/edge-api/` — so no wrangler path edits are needed beyond confirming the build
runs from the new directory. Update any deploy/migration/secret commands in docs to the new path.

## 6. Update CI and ignores
- `.github/workflows/ci.yml` "Enforce architecture dependency policy" greps:
  `crates/orderbook/Cargo.toml` → `packages/orderbook/Cargo.toml` (both getrandom + uuid lines);
  `crates/worker-cache/src/lib.rs` → `packages/worker-cache/src/lib.rs`. Leave the root
  `Cargo.toml`/`.cargo/config.toml` greps, the `cargo tree -p bunting-orderbook` check, and the
  `workers/market-run-do/AGENTS.md` assertion as-is. (Optionally add `chore/**` to push triggers.)
- `.gitignore`: add `/out/` and `apps/edge-api/build/` (keep `dist/`, `/target/`, `.wrangler/`).

## 7. Fix active path consumers (not history)
Run and triage each hit — change active paths/commands, leave historical ADR text and `ref/` source
untouched:
```bash
git grep -n 'crates/'            ; git grep -n 'workers/edge-api'
git grep -n 'workers/'           ; git grep -n 'quarcc-trading-engine'
git grep -n 'dist/'              ; git grep -n 'out/'
```
Update `README.md`, `docs/architecture.md`, `docs/implementation-pathway.md`,
`docs/codex-implementation-prompt.md`, and scoped `AGENTS.md` prose to the new paths. Do NOT global
-replace inside `ref/`, historical diffs, or archived planning docs under `docs/claude/`.

## 8. Release tooling for `out/` (optional in this PR but preferred)
Add a `tools/` script (or `xtask`) that builds `apps/edge-api` in release mode, collects the JS
shim + Wasm module + metadata into `out/edge-api/<version>/`, emits SHA-256 checksums and a manifest
(commit, toolchain, target, package versions), and leaves `out/` ignored. A tag-driven
`.github/workflows/release.yml` uploads that bundle to a GitHub Release. A raw `.wasm` alone is not
the deployable entrypoint — ship the shim + module + metadata together.

## 9. Non-goals (reject if tempted)
No package renames; no behavior/public-API changes beyond the thin `bunting-rs` crate; no NBC or
QUARCC implementation; no protocol-stack selection; no dependency upgrades; no D1/schema changes; no
new order types/streaming; no source copied from `ref/`; no OrderBook-rs fork; no committed
`target/`/`build/`/`out/`/Wasm; no branch deletion inside this PR.

## 10. Validate (must all pass before committing)
```bash
cargo metadata --locked --format-version 1 --no-deps > /tmp/bunting-metadata-after.json
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
cd apps/edge-api && worker-build --release   # expect apps/edge-api/build/worker/shim.mjs
```
Also verify: workspace members changed only by path plus the new `bunting-rs`; package names and
dependency versions unchanged; no production manifest references `ref/`; nothing depends on
`bunting-rs`/`apps/`; `bunting-rs` depends inward only; Worker build + D1 migration discovery work
from `apps/edge-api`; no generated artifact tracked; no upstream source copied into `packages/`.

## 11. Commit shape and PR
Reviewable commits:
1. `chore: move reusable Rust crates under packages`
2. `chore: move edge API under apps`
3. `feat: add thin bunting composition crate`
4. `docs: align active paths and commands`

Open the PR only if the user asks. PR body must include: before/after trees, the exact move map,
before/after Cargo metadata member lists, the validation output, the stale-path search results, and
an explicit statement that runtime semantics, package names, and dependency versions did not change.

## After this PR (do NOT do here)
P1 NBC market-engine foundation → P2 QUARCC execution-engine port → P3 staging/run provisioning →
P4 streaming + broader default-engine order capabilities → P5 concrete protocol/client/model
packages + release distribution. See `docs/repository-reorganization.md` "Work after reorganization".
</content>
