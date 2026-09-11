# Bunting Local Venue State Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make desktop upgrades recover safely from stale or incompatible local Bunting state without silently overwriting user configuration or reducing server integrity checks.

**Architecture:** Keep server scenario-hash validation fail-closed. The desktop launcher records the bundled server/scenario identity separately from mutable Application Support files, classifies known restore incompatibilities from the bounded server log, and offers an explicit quarantine/reset operation that moves only generated venue state/session snapshots aside. User-owned `local.json` and `scenario.json` are never silently overwritten.

**Tech Stack:** Rust 1.95 desktop workspace, std filesystem/process APIs, serde/serde_json, SHA-256 via `sha2`, GPUI Kit diagnostics/actions.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- Server-side scenario/state integrity checks remain unchanged and fail closed.
- Existing `local.json` and `scenario.json` are never silently overwritten after first install.
- Reset/quarantine affects only generated state and generated FIX session snapshots.
- The current local storage file is `bunting-local-state.json`; generated participant session files match `bunting-local-state.fix-session-*.json`.
- A reset is explicit user action; startup never deletes incompatible state automatically.
- Quarantined files are retained under a timestamped recovery directory until the user removes them manually.

---

### Task 1: Record bundled local-server identity without changing user config

**Files:**
- Modify: `apps/bunting-terminal/Cargo.toml`
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Modify: `apps/bunting-terminal/scripts/package-macos-arm64.sh`
- Test: `apps/bunting-terminal/src/local_server.rs`

**Interfaces:**
- Produces:

```rust
const LOCAL_BUNDLE_MANIFEST_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct LocalBundleManifest {
    version: u16,
    server_sha256: String,
    scenario_sha256: String,
    config_sha256: String,
}

fn sha256_file(path: &Path) -> io::Result<String>;
fn bundled_manifest(plan: &LaunchPlan) -> io::Result<LocalBundleManifest>;
```

- [ ] **Step 1: Add `sha2 = "0.11"` to the desktop manifest and write a failing digest test**

Create a temp file containing `abc` and assert:

```rust
assert_eq!(
    sha256_file(&path).unwrap(),
    "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
);
```

- [ ] **Step 2: Run red**

```bash
cd apps/bunting-terminal
cargo test sha256_file
```

Expected: FAIL because the helper does not exist.

- [ ] **Step 3: Implement bounded streaming SHA-256**

Read through a fixed 64 KiB buffer and update `sha2::Sha256`; do not load the WASM module into one unbounded `Vec` solely to hash it.

- [ ] **Step 4: Compute a bundle manifest from the actual launch inputs**

`bundled_manifest()` hashes the resolved `bunting-server.wasm`, template `local.json`, and template `scenario.json` that are packaged with the app. Serialize the manifest to `Contents/Resources/server/bundle-manifest.json` in `package-macos-arm64.sh` after the four required inputs have been validated and before codesigning.

The packaging script computes hashes with `shasum -a 256` and emits exactly:

```json
{
  "version": 1,
  "server_sha256": "<64 lowercase hex characters>",
  "scenario_sha256": "<64 lowercase hex characters>",
  "config_sha256": "<64 lowercase hex characters>"
}
```

At runtime, recompute/verify the bundled files against this manifest when the manifest exists. Development builds without the packaged manifest compute an in-memory identity and continue.

- [ ] **Step 5: Run tests and package-script shell validation**

```bash
cd apps/bunting-terminal
cargo test local_bundle_manifest
cargo clippy --all-targets -- -D warnings
bash -n scripts/package-macos-arm64.sh
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/Cargo.toml apps/bunting-terminal/Cargo.lock apps/bunting-terminal/src/local_server.rs apps/bunting-terminal/scripts/package-macos-arm64.sh
git commit -m "feat: identify bundled local venue inputs"
```

---

### Task 2: Persist installed bundle identity separately from mutable templates

**Files:**
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Test: `apps/bunting-terminal/src/local_server.rs`

**Interfaces:**
- Produces Application Support metadata file `bundle-installation.json`:

```rust
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct InstalledBundleIdentity {
    version: u16,
    server_sha256: String,
    scenario_template_sha256: String,
    config_template_sha256: String,
}
```

This file describes what the app bundle supplied. It does not assert that mutable `local.json`/`scenario.json` still equal the templates.

- [ ] **Step 1: Write a failing upgrade-preservation test**

Create an application-server temp directory containing user-edited `local.json` and `scenario.json`, plus an old `bundle-installation.json`. Run the installation/template preparation logic with a different new bundle identity and assert the two user files are byte-for-byte unchanged while `bundle-installation.json` is updated only after successful preparation.

- [ ] **Step 2: Run red**

```bash
cargo test preserves_user_local_templates_on_bundle_upgrade
```

Expected: FAIL because installed bundle identity is not tracked.

- [ ] **Step 3: Implement installation identity persistence**

Keep existing `copy_if_missing()` for `local.json`/`scenario.json`. Write `bundle-installation.json` atomically using a temp file in the same directory followed by rename. Never use its hash mismatch as permission to overwrite mutable files.

- [ ] **Step 4: Add first-install test**

With an empty Application Support directory, assert templates are copied exactly once and identity metadata is created.

- [ ] **Step 5: Run tests**

```bash
cargo test bundle_installation
cargo test preserves_user_local_templates_on_bundle_upgrade
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src/local_server.rs
git commit -m "feat: track local venue bundle upgrades safely"
```

---

### Task 3: Classify incompatible persisted-state startup failures

**Files:**
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Test: `apps/bunting-terminal/src/local_server.rs`

**Interfaces:**
- Extends structured launch diagnostics with recoverability:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    None,
    QuarantineGeneratedState,
}

pub struct LocalServerDiagnostic {
    // fields from runtime-correctness plan
    pub recovery_action: RecoveryAction,
}

fn classify_server_exit(log_tail: &str, status: &str) -> LocalServerDiagnostic;
```

Known recoverable generated-state incompatibility includes the server message `configured immutable scenario does not match the restored run hash`.

- [ ] **Step 1: Write failing classification tests**

```rust
#[test]
fn scenario_hash_restore_failure_offers_generated_state_quarantine() {
    let diagnostic = classify_server_exit(
        "bunting-server: configured immutable scenario does not match the restored run hash",
        "exit status: 2",
    );
    assert_eq!(diagnostic.recovery_action, RecoveryAction::QuarantineGeneratedState);
}

#[test]
fn unknown_startup_failure_does_not_offer_destructive_recovery() {
    let diagnostic = classify_server_exit("bunting-server: cannot bind FIX listener", "exit status: 2");
    assert_eq!(diagnostic.recovery_action, RecoveryAction::None);
}
```

- [ ] **Step 2: Run red**

```bash
cargo test classify_server_exit
```

Expected: FAIL until recoverability exists.

- [ ] **Step 3: Implement narrow message classification**

Map only known state/schema/scenario restore incompatibility messages to `QuarantineGeneratedState`. Port binding, Wasmer errors, malformed config, missing files, authentication, and unknown failures remain `None`.

- [ ] **Step 4: Run tests**

```bash
cargo test classify_server_exit
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/bunting-terminal/src/local_server.rs
git commit -m "feat: classify recoverable local venue state failures"
```

---

### Task 4: Quarantine only generated venue state and FIX session snapshots

**Files:**
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Test: `apps/bunting-terminal/src/local_server.rs`

**Interfaces:**
- Produces:

```rust
pub fn quarantine_generated_state(&mut self) -> Result<PathBuf, String>;

fn generated_state_files(server_dir: &Path) -> io::Result<Vec<PathBuf>>;
```

Files eligible for quarantine are exactly:

```text
bunting-local-state.json
bunting-local-state.fix-session-*.json
```

`local.json`, `scenario.json`, `bunting-server.log`, and bundle metadata are never moved by this function.

- [ ] **Step 1: Write a failing bounded-file-selection test**

Create temp files named:

```text
bunting-local-state.json
bunting-local-state.fix-session-1.json
bunting-local-state.fix-session-2.json
local.json
scenario.json
bunting-server.log
other.json
```

Assert `generated_state_files()` returns only the first three.

- [ ] **Step 2: Run red**

```bash
cargo test generated_state_files
```

Expected: FAIL until helper exists.

- [ ] **Step 3: Implement exact file selection and quarantine destination**

Create a child directory named `recovery/<unix-seconds>-<process-id>/` under the server Application Support directory. Move selected files with `fs::rename`; if cross-device rename occurs, copy + fsync + remove source only after successful copy. Never follow symlinks outside the server directory: inspect `symlink_metadata` and reject symlink candidates.

- [ ] **Step 4: Make the operation transactional enough for user recovery**

Before moving, create `manifest.json` in the recovery directory listing original file names. If any move fails, return an error identifying files already moved; do not delete the recovery directory. A second invocation is safe and quarantines whatever eligible files remain.

- [ ] **Step 5: Add preservation test**

Call quarantine and assert state/session files moved, `local.json` and `scenario.json` unchanged, and returned directory contains the recovery manifest.

- [ ] **Step 6: Run tests**

```bash
cargo test generated_state_files
cargo test quarantine_generated_state
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/bunting-terminal/src/local_server.rs
git commit -m "feat: quarantine incompatible local venue state"
```

---

### Task 5: Add explicit recovery action to the GPUI terminal

**Files:**
- Modify: `apps/bunting-terminal/src/shell/layout.rs`
- Modify: `apps/bunting-terminal/src/shell/view.rs`
- Test: `apps/bunting-terminal/tests/workstation_ui.rs`

**Interfaces:**
- When and only when `server_snapshot.diagnostic.recovery_action == RecoveryAction::QuarantineGeneratedState`, the diagnostic surface shows `Reset Generated Local State`.
- Clicking it quarantines generated state, then starts the app-owned local server and reconnects FIX only after versioned health becomes ready.

- [ ] **Step 1: Write a failing UI recovery test**

Inject a recoverable scenario-hash diagnostic and assert the action is visible. Inject a bind failure and assert the action is absent.

- [ ] **Step 2: Run red**

```bash
cd apps/bunting-terminal
cargo test --features test-support --test workstation_ui local_state_recovery
```

Expected: FAIL before the recovery action is rendered.

- [ ] **Step 3: Wire the button to the controller operation**

The handler calls `quarantine_generated_state()`. On success, display the returned recovery directory and invoke the same existing `start_local_server` flow. On failure, keep the venue stopped and surface the error; do not attempt partial cleanup.

- [ ] **Step 4: Require explicit confirmation in the dialog/sheet**

Use GPUI Kit's current confirmation dialog/sheet component. Copy must state that orders/positions in the local generated venue state will be reset, while `local.json` and `scenario.json` are preserved and old state is moved to the shown recovery directory.

- [ ] **Step 5: Run UI and local-server tests**

```bash
cargo test --features test-support --test workstation_ui local_state_recovery
cargo test local_server
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src apps/bunting-terminal/tests/workstation_ui.rs
git commit -m "feat: add safe local venue recovery action"
```
