# Bunting Canonical Terminal Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote the GPUI RIT workstation to Bunting's canonical desktop interface and remove the Ratatui presentation from supported packaging only after an explicit workflow-parity and live-venue release gate passes.

**Architecture:** Treat this as a release/cutover plan, not another UI rewrite. First prove every supported participant/operator workflow is available through the GPUI terminal or retained headless CLI/client surfaces, then integrate the macOS GPUI artifact into canonical releases, remove the `bunting-cli` TUI feature/alias and finally delete the obsolete Ratatui app when no code depends on it.

**Tech Stack:** Rust root workspace, Rust 1.95 GPUI terminal, GitHub Actions releases, Wasmer/WASIX server, macOS ARM64 app/DMG.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- This plan executes only after runtime correctness, complete competition replay, Worker publication cleanup, authoritative market history, client extraction, and GPUI terminal plans are green.
- Ratatui is not removed until no supported workflow is lost.
- Headless CLI commands `server`, `init`, `version`, `replay`, `score`, `judge`, `doctor`, `conformance`, and `export-roster` remain supported.
- Native FIX/client logic remains in `packages/bunting-client` after TUI deletion.
- Canonical desktop release is the GPUI terminal; the cross-platform CLI remains a separate non-GUI product surface.
- Do not claim GPUI desktop support on platforms not exercised by release CI.

---

### Task 1: Create and enforce a participant/operator parity gate

**Files:**
- Create: `docs/specs/desktop-workflow-parity.md`
- Create: `tools/check_desktop_parity.py`
- Modify: `.github/workflows/ci.yml`
- Test: `tools/check_desktop_parity.py`

**Interfaces:**
- The parity document is machine-readable Markdown with one row per currently supported TUI workflow and columns `Workflow`, `Canonical Replacement`, `Evidence Test`, `Status`.
- `Status` must be `PASS` for cutover.

Required rows:

```text
connect/logon/reconnect
market book/depth
market history/chart
limit order
market order
cancel order
execution/order history
account/positions/cash
risk/score
news
tenders
run controls for authorized roles
FIX/session diagnostics
local server start/stop/reconnect
workspace selection
```

- [ ] **Step 1: Write parity checker first with all required workflow IDs**

Implement Python constants for the required IDs and parse the Markdown table. Exit nonzero if a row is missing, replacement/evidence cell is empty, or status is not `PASS`.

- [ ] **Step 2: Run checker against no matrix and observe failure**

```bash
python3 tools/check_desktop_parity.py
```

Expected: FAIL because `desktop-workflow-parity.md` is absent.

- [ ] **Step 3: Populate the parity matrix with exact GPUI/client tests**

For each required row point to an existing automated test from `apps/bunting-terminal/tests`, `packages/bunting-client`, or native-server black-box tests. If a workflow lacks evidence, implement the missing test in the relevant preceding plan before marking the row PASS.

- [ ] **Step 4: Run checker and all referenced tests**

```bash
python3 tools/check_desktop_parity.py
cargo test -p bunting-client
cargo test -p bunting-server
cd apps/bunting-terminal && cargo test --features test-support
```

Expected: PASS.

- [ ] **Step 5: Add checker to canonical CI**

Place it after architecture-boundary checks and before release-sensitive build steps.

- [ ] **Step 6: Commit**

```bash
git add docs/specs/desktop-workflow-parity.md tools/check_desktop_parity.py .github/workflows/ci.yml apps/bunting-terminal/tests packages/bunting-client apps/bunting-server/tests
git commit -m "test: gate desktop cutover on workflow parity"
```

---

### Task 2: Integrate the GPUI macOS artifact into canonical releases

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/gpui-terminal.yml`
- Modify: `apps/bunting-terminal/scripts/package-macos-arm64.sh`
- Modify: `README.md`

**Interfaces:**
- Canonical tag release publishes:
  - portable Wasmer/WASIX server bundle;
  - cross-platform headless/native CLI artifacts and language bindings;
  - `Bunting-Market-Terminal-<tag>-macos-arm64.dmg` and checksum.

- [ ] **Step 1: Add a release `gpui-macos-arm64` job**

Use `macos-15`, install Rust 1.95, build/package the terminal with its standalone manifest, build/copy the exact server WASM artifact required by the app, run `cargo test --features test-support`, Clippy, local-server live smoke, codesign verification, and create the DMG.

- [ ] **Step 2: Keep cargo-wasix bootstrap scoped correctly**

Use the runtime-correctness plan's Rust 1.95 bootstrap environment for cargo-wasix; do not change the root workspace Rust 1.88 contract.

- [ ] **Step 3: Upload terminal artifact under a stable name**

```yaml
      - uses: actions/upload-artifact@v4
        with:
          name: gpui-macos-arm64
          path: |
            apps/bunting-terminal/dist/*.dmg
            apps/bunting-terminal/dist/*.sha256
          if-no-files-found: error
```

Adjust `dist` path to the packaging script's actual existing output and update script/workflow together so this is exact.

- [ ] **Step 4: Add `gpui-macos-arm64` to publish job dependencies**

`publish.needs` includes `native`, `wasi-server`, and `gpui-macos-arm64`; artifact download remains merged into release staging.

- [ ] **Step 5: Update release notes**

Describe GPUI Market Terminal as the canonical macOS desktop application and the `bunting` archive as CLI/server tooling. Do not call the CLI archive a desktop terminal.

- [ ] **Step 6: Validate workflow with a non-production prerelease tag/dispatch path**

Build all artifacts without publishing a stable production release. Verify checksum generation includes DMG and server/CLI artifacts.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/release.yml .github/workflows/gpui-terminal.yml apps/bunting-terminal/scripts/package-macos-arm64.sh README.md
git commit -m "release: ship GPUI terminal in canonical releases"
```

---

### Task 3: Remove `bunting-tui` from CLI/release compatibility surface

**Files:**
- Modify: `apps/bunting-cli/Cargo.toml`
- Modify: `apps/bunting-cli/src/native.rs`
- Modify: `.github/workflows/release.yml`
- Modify: `install.sh`
- Modify: `README.md`
- Modify: `RUNBOOK.md`
- Test: `apps/bunting-cli/src/native.rs`

**Interfaces:**
- Removes Cargo feature `tui`, `Command::Tui`, `TuiOptions`, `bunting_tui::run`, legacy executable-name routing from `bunting-tui` to `bunting tui`, and release/install `bunting-tui` alias.
- Preserves all non-TUI CLI subcommands.

- [ ] **Step 1: Rewrite CLI tests to state the post-cutover contract**

Change `legacy_names_route_to_unified_subcommands` so `bunting-server` compatibility remains, but `bunting-tui` is not normalized to a supported subcommand. Add:

```rust
#[test]
fn tui_subcommand_is_not_part_of_canonical_cli() {
    assert!(Cli::try_parse_from(["bunting", "tui"]).is_err());
}
```

- [ ] **Step 2: Run with current feature and observe failure**

```bash
cargo test -p bunting-cli --features tui tui_subcommand_is_not_part_of_canonical_cli
```

Expected: FAIL because TUI command still parses.

- [ ] **Step 3: Remove the TUI feature/dependency and command**

Delete `[features] tui`, optional `bunting-tui`, Tui import/variant/match arm, cfg attribute related only to that feature, and legacy `bunting-tui` compatibility mapping.

- [ ] **Step 4: Remove TUI release alias/install behavior**

In release packaging, build `bunting-cli` without `--features tui` and do not create `bunting-tui` symlink/copy. Update `install.sh` so new installs do not install or mention that alias.

- [ ] **Step 5: Run CLI tests**

```bash
cargo test -p bunting-cli
cargo clippy -p bunting-cli --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-cli .github/workflows/release.yml install.sh README.md RUNBOOK.md Cargo.lock
git commit -m "refactor: retire Ratatui CLI surface"
```

---

### Task 4: Delete the obsolete Ratatui application after dependency proof

**Files:**
- Delete: `apps/bunting-tui/**`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: server/dev manifests that still reference `bunting-tui`
- Modify: `.github/workflows/ci.yml`
- Modify: documentation containing canonical TUI instructions
- Test: root workspace

**Interfaces:**
- `packages/bunting-client` is the only retained reusable native FIX client implementation.
- Root workspace no longer has member/default-member `apps/bunting-tui`.

- [ ] **Step 1: Prove no production/dev consumer depends on `bunting-tui`**

Run:

```bash
grep -R --line-number --exclude-dir=.git --exclude='*.md' 'bunting[-_]tui' Cargo.toml apps packages bindings tests tools
cargo tree --workspace | grep -F 'bunting-tui'
```

Expected before deletion: only the TUI package itself or intentionally scheduled removal references remain. Migrate any server dev-test import to `bunting-client` before continuing.

- [ ] **Step 2: Remove workspace membership and package references**

Delete `apps/bunting-tui` from root workspace members/default-members and remove dependencies from all manifests.

- [ ] **Step 3: Delete the application directory**

```bash
git rm -r apps/bunting-tui
```

No client/protocol source is lost because it was extracted and parity-tested in the client-extraction plan.

- [ ] **Step 4: Regenerate lockfile and run root checks**

```bash
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Expected: PASS with no `bunting-tui` package in `cargo metadata`.

- [ ] **Step 5: Run GPUI standalone checks**

```bash
cd apps/bunting-terminal
cargo check
cargo test --features test-support
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add -A apps/bunting-tui Cargo.toml Cargo.lock apps packages tests .github/workflows/ci.yml
git commit -m "refactor: remove retired Ratatui application"
```

---

### Task 5: Reconcile product, architecture, and RIT parity documentation

**Files:**
- Modify: `AGENTS.md`
- Modify: `README.md`
- Modify: `RUNBOOK.md`
- Modify: `docs/architecture/README.md`
- Modify: `docs/specs/bunting-product-contract.md`
- Modify: `docs/specs/rit-tui-parity-matrix.md`
- Modify: `docs/research/rit-binary-audit/market-feature-ledger.md`
- Modify: `docs/status.md`
- Modify: `docs/plan.md`

**Interfaces:**
- All current docs name GPUI Market Terminal as canonical desktop UI, `bunting-client` as shared native client, and CLI as headless/operational surface. No doc instructs a user to launch `bunting tui` or `bunting-tui`.

- [ ] **Step 1: Add a stale-documentation guard**

Extend `tools/check_desktop_parity.py` to scan canonical docs/install/release files and reject phrases/commands:

```text
bunting tui
bunting-tui
native Ratatui terminal
```

Exclude historical design/ADR/spec files that intentionally record the migration history.

- [ ] **Step 2: Run and observe failures**

```bash
python3 tools/check_desktop_parity.py
```

Expected: FAIL on stale current documentation before edits.

- [ ] **Step 3: Update current docs to exact shipped state**

Document GPUI Kit 0.6.1, RIT-style default layout, authoritative history, local-server diagnostics, supported macOS architecture, headless CLI, collaboration status, and remaining RIT gaps. Do not mark unsupported multi-listing/facilities/history analytics as complete.

- [ ] **Step 4: Run doc/protocol/parity guards**

```bash
python3 tools/check_desktop_parity.py
python3 tools/generate_protocol.py
git diff --exit-code -- PROTOCOL.md
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add AGENTS.md README.md RUNBOOK.md docs tools/check_desktop_parity.py
git commit -m "docs: complete canonical desktop cutover"
```

---

### Task 6: Execute the final live release gate

**Files:**
- No production code changes expected; failures return to the owning earlier task/plan.
- Update: `docs/status.md` only after all evidence passes.
- Update: PR #19 description/status.

**Interfaces:**
- Release gate evidence covers native venue resilience, replay, market history, Worker publication boundary, client, GPUI UI, package, and live app/server integration.

- [ ] **Step 1: Run full root verification**

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked --workspace --target wasm32-unknown-unknown
python3 tools/check_client_boundary.py
python3 tools/check_collaboration_boundary.py
python3 tools/check_desktop_parity.py
```

Expected: PASS.

- [ ] **Step 2: Run Worker raw-runtime verification**

Build `apps/bunting-worker` with `worker-build --release --no-panic-recovery`, start pinned workerd, assert health, read-only mutation rejection, snapshot/subscription behavior, and absence of FIX Durable Object requirement.

- [ ] **Step 3: Run authoritative replay verification**

Generate a competition archive containing simulation plus participant order/cancel traffic from a live native fixture, replay with `bunting replay`, score it, and assert canonical event vector/final hash match the live run.

- [ ] **Step 4: Run GPUI terminal verification**

```bash
cd apps/bunting-terminal
cargo fmt --check
cargo test --features test-support
cargo clippy --all-targets -- -D warnings
cargo build --release
```

Expected: PASS.

- [ ] **Step 5: Exercise packaged macOS application against bundled Wasmer venue**

From a fresh app-support directory: start server from app, verify Wasmer version/health, establish FIX, receive book/account/risk/history, submit and cancel a participant order, switch RIT workspace panels, stop/restart server, reconnect, verify open-order accounting survives, and confirm a malformed separate client does not kill the venue.

- [ ] **Step 6: Verify release artifact contents/checksums/signing**

Confirm DMG opens and app is arm64, ad-hoc signature verifies for preview distribution, server WASM/config templates are present, and checksums match. Stable distribution notarization is a separate release-hardening requirement unless already added and verified.

- [ ] **Step 7: Update status and PR #19 only from evidence**

Mark GPUI desktop canonical and Ratatui retired in `docs/status.md`. Update PR #19 body to list actual commits/tests/workflow run IDs and remaining non-goals. Keep claims bounded to verified platforms/features.

- [ ] **Step 8: Commit status evidence**

```bash
git add docs/status.md
git commit -m "docs: record canonical terminal release evidence"
```
