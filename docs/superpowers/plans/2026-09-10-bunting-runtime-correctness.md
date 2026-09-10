# Bunting Runtime Correctness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Bunting's canonical CI trustworthy and make the native competition server/local desktop launcher resilient, diagnosable, reconnect-safe, and independent of host wall-clock authority.

**Architecture:** Keep the native server as the only venue authority. Repair the CI bootstrap without changing the root Rust 1.88 application contract, move desktop readiness to a versioned admin health handshake, isolate bad admin/FIX clients at connection boundaries, derive participant open-order limits from authoritative run state, and source order logical time from the venue run clock.

**Tech Stack:** Rust 1.88 root workspace, Rust 1.95 desktop workspace, Wasmer 7.2.1, cargo-wasix 0.1.28, std TCP, serde/serde_json, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- `apps/bunting-server` remains the sole mutation/venue authority.
- Root workspace source remains compatible with Rust 1.88; only the cargo-wasix bootstrap step may use a newer Rust toolchain.
- Local readiness must never connect to the participant FIX port.
- A random listener on port 9880 or 8080 must not be accepted as a compatible Bunting server.
- Host wall time may timestamp transport diagnostics but may not silently become simulation logical time.
- Reconnect must not reset participant venue limits.
- One malformed admin/FIX/TLS peer must not terminate unrelated venue services.

---

### Task 1: Repair canonical cargo-wasix bootstrap

**Files:**
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/release.yml`
- Modify: `.github/workflows/gpui-terminal.yml`

**Interfaces:**
- Consumes: root `rust-toolchain.toml` Rust 1.88 contract and cargo-wasix action `v0.1.28`.
- Produces: a workflow-only bootstrap environment using Rust 1.95.0 for the cargo-wasix installer while all ordinary root Cargo commands continue using the repository toolchain.

- [ ] **Step 1: Add a workflow assertion that makes the intended toolchains explicit**

In `ci.yml`, immediately before the cargo-wasix action add a shell step that verifies the repository toolchain is still active and records it:

```yaml
      - name: Verify repository Rust toolchain
        run: |
          set -euo pipefail
          rustc --version | grep -F 'rustc 1.88.'
          cargo --version
```

In `release.yml`, keep the existing `dtolnay/rust-toolchain@1.88.0` step and add the same assertion.

- [ ] **Step 2: Run the current workflow path locally or in an Actions branch and record the expected failure**

Run the equivalent installer under Rust 1.88 or dispatch CI before the fix.

Expected: cargo-wasix installation fails because the bootstrap dependency graph includes `cargo-platform 0.3.3`, which requires rustc 1.91 or newer.

- [ ] **Step 3: Scope Rust 1.95 only to the cargo-wasix action**

Change the cargo-wasix action step in both `ci.yml` and `release.yml` to:

```yaml
      - name: Install pinned WASIX toolchain
        uses: wasix-org/cargo-wasix@v0.1.28
        env:
          RUSTUP_TOOLCHAIN: '1.95.0'
        with:
          version: '0.1.28'
          toolchain-version: 'v2026-07-07.3+rust-1.96'
          locked: 'true'
```

Ensure a prior step installs the bootstrap toolchain without changing the default:

```yaml
      - name: Install cargo-wasix bootstrap Rust
        run: rustup toolchain install 1.95.0 --profile minimal
```

Use the same pattern in `gpui-terminal.yml`; remove any duplicate or contradictory cargo-wasix bootstrap workaround while preserving the terminal's own Rust 1.95 build toolchain.

- [ ] **Step 4: Verify root commands still use Rust 1.88 after cargo-wasix installation**

Add after the action in root CI:

```yaml
      - name: Verify root toolchain was not replaced
        run: rustc --version | grep -F 'rustc 1.88.'
```

Expected: PASS.

- [ ] **Step 5: Run the canonical validation commands**

Run:

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked --workspace --target wasm32-unknown-unknown
```

Expected: all commands PASS using the repository toolchain; the WASIX build step also completes with the pinned WASIX toolchain.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/ci.yml .github/workflows/release.yml .github/workflows/gpui-terminal.yml
git commit -m "ci: repair cargo-wasix bootstrap toolchain"
```

---

### Task 2: Version the native admin health contract

**Files:**
- Modify: `apps/bunting-server/src/admin.rs`
- Modify: `apps/bunting-server/src/lib.rs`
- Test: `apps/bunting-server/src/admin.rs`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Produces: unauthenticated `GET /health` JSON with exact fields `status`, `service`, `healthContractVersion`, and `fixCompetitionProfileVersion`.
- Consumes later: desktop `probe_bunting_health()` in Task 3.

- [ ] **Step 1: Write a failing serializer-level health test**

Extract the health body into a pure function and first add this test in `admin.rs`:

```rust
#[test]
fn health_contract_identifies_bunting_and_protocol_version() {
    let body = health_body();
    assert_eq!(body["status"], "ok");
    assert_eq!(body["service"], crate::SERVICE_NAME);
    assert_eq!(body["healthContractVersion"], 1);
    assert_eq!(
        body["fixCompetitionProfileVersion"],
        bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION
    );
}
```

- [ ] **Step 2: Run the test and verify it fails**

Run:

```bash
cargo test -p bunting-server health_contract_identifies_bunting_and_protocol_version -- --exact
```

Expected: FAIL because `health_body()` does not exist.

- [ ] **Step 3: Implement the exact health body**

Add in `admin.rs`:

```rust
const HEALTH_CONTRACT_VERSION: u16 = 1;

fn health_body() -> serde_json::Value {
    serde_json::json!({
        "status": "ok",
        "service": crate::SERVICE_NAME,
        "healthContractVersion": HEALTH_CONTRACT_VERSION,
        "fixCompetitionProfileVersion": bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION,
    })
}
```

Replace the inline `/health` JSON with `health_body()`.

- [ ] **Step 4: Run the focused and package tests**

Run:

```bash
cargo test -p bunting-server health_contract_identifies_bunting_and_protocol_version
cargo test -p bunting-server
```

Expected: PASS.

- [ ] **Step 5: Tighten the CI smoke assertion**

After the existing health curl in `.github/workflows/ci.yml`, replace the one-field grep with a Python assertion so CI proves it contacted Bunting rather than any HTTP listener:

```bash
python3 - <<'PY'
import json
with open('/tmp/bunting-health.json', encoding='utf-8') as fh:
    health = json.load(fh)
assert health['status'] == 'ok'
assert health['service'] == 'bunting-server'
assert health['healthContractVersion'] == 1
assert health['fixCompetitionProfileVersion']
PY
```

Use the actual `SERVICE_NAME` value if it differs; do not duplicate a different product name.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-server/src/admin.rs apps/bunting-server/src/lib.rs .github/workflows/ci.yml
git commit -m "feat: version native server health contract"
```

---

### Task 3: Replace FIX-port readiness with a Bunting health probe and structured launch diagnostics

**Files:**
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Modify: `apps/bunting-terminal/src/shell/layout.rs`
- Modify: `apps/bunting-terminal/src/shell/view.rs`
- Test: `apps/bunting-terminal/src/local_server.rs`

**Interfaces:**
- Consumes: `GET /health` contract from Task 2.
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalServerDiagnostic {
    pub phase: LocalServerPhase,
    pub code: &'static str,
    pub detail: String,
    pub log_path: Option<PathBuf>,
    pub exit_status: Option<String>,
}

fn probe_bunting_health(address: SocketAddr, timeout: Duration) -> Result<HealthProbe, String>;
fn verify_wasmer(path: &Path) -> Result<(), String>;
```

`HealthProbe` contains the parsed service/contract/profile identity; readiness is true only when all expected identity fields match.

- [ ] **Step 1: Write a failing health-probe test using a one-shot local TCP listener**

Add a test helper that binds `127.0.0.1:0`, accepts one request, writes a minimal HTTP response, and assert:

```rust
#[test]
fn health_probe_rejects_non_bunting_http_listener() {
    let address = spawn_http_once(r#"{"status":"ok","service":"other","healthContractVersion":1,"fixCompetitionProfileVersion":"x"}"#);
    let error = probe_bunting_health(address, Duration::from_millis(250)).unwrap_err();
    assert!(error.contains("service"));
}
```

Add a second test with `service:"bunting-server"` and the expected contract/profile asserting success.

- [ ] **Step 2: Run terminal tests and verify the new tests fail**

Run from `apps/bunting-terminal`:

```bash
cargo test health_probe -- --nocapture
```

Expected: FAIL because `probe_bunting_health` is undefined.

- [ ] **Step 3: Implement a bounded HTTP health probe without adding a heavyweight client dependency**

Use `TcpStream::connect_timeout`, set read/write timeouts, write:

```text
GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n
```

Bound the response read to 16 KiB, split headers/body at `\r\n\r\n`, require HTTP 200, parse body with `serde_json`, then require:

```rust
probe.status == "ok"
    && probe.service == "bunting-server"
    && probe.health_contract_version == 1
```

Also require the exact expected FIX competition profile string from the shared client/package after Task 6 migration; until then import the existing constant transitively available to the app rather than hard-coding a second profile version.

- [ ] **Step 4: Delete `endpoint_is_ready()` and all FIX-port readiness calls**

Replace `FIX_ENDPOINT` probing with `ADMIN_ENDPOINT: "127.0.0.1:8080"`. `LocalServerState::External` is entered only after a compatible health response. A non-Bunting listener produces `Failed` with code `HEALTH_IDENTITY_MISMATCH`; connection refusal remains `Stopped` when no child is owned.

- [ ] **Step 5: Write and run a failing Wasmer-version test**

Refactor command execution behind:

```rust
fn parse_wasmer_version(stdout: &str) -> Option<(u64, u64, u64)>;
```

Add:

```rust
#[test]
fn wasmer_version_requires_7_2_1() {
    assert_eq!(parse_wasmer_version("wasmer 7.2.1"), Some((7, 2, 1)));
    assert_ne!(parse_wasmer_version("wasmer 6.1.0"), Some((7, 2, 1)));
}
```

Run `cargo test wasmer_version` and observe failure before implementation.

- [ ] **Step 6: Verify the discovered Wasmer binary before launch**

`verify_wasmer(path)` executes `path --version`, requires success, parses the semantic version, and requires exactly `(7,2,1)` for this release. Return a diagnostic with code `WASMER_VERSION_MISMATCH` containing discovered and required versions.

- [ ] **Step 7: Surface the bounded root server error instead of exit code alone**

Add:

```rust
fn tail_log(path: &Path, max_bytes: usize) -> io::Result<String>;
```

Read only the final 8 KiB, normalize control characters, and include the last non-empty `bunting-server:` line in `LocalServerDiagnostic.detail`. Never include credentials or full config contents.

When the child exits, populate `exit_status` and `log_path` separately; `snapshot()` formats the diagnostic but preserves typed fields internally.

- [ ] **Step 8: Run terminal tests and Clippy**

Run:

```bash
cd apps/bunting-terminal
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add apps/bunting-terminal/src/local_server.rs apps/bunting-terminal/src/shell/layout.rs apps/bunting-terminal/src/shell/view.rs
git commit -m "fix: make local server readiness authoritative"
```

---

### Task 4: Isolate malformed admin and FIX peers from listener lifetime

**Files:**
- Modify: `apps/bunting-server/src/admin.rs`
- Modify: `apps/bunting-server/src/acceptor.rs`
- Test: `apps/bunting-server/src/admin.rs`
- Test: `apps/bunting-server/src/acceptor.rs`

**Interfaces:**
- Produces: listener loops only return `Err` for bind/listener-level failures; per-connection validation/handling errors are logged and the loop continues.

- [ ] **Step 1: Write a failing pure admin request test for invalid run IDs**

Extract request dispatch into a function returning a response tuple rather than letting parsing errors escape:

```rust
#[test]
fn malformed_admin_run_id_is_a_client_error() {
    let response = classify_request("GET /admin/runs/not-a-number HTTP/1.1\r\nAuthorization: Bearer x\r\n\r\n", "x");
    assert_eq!(response.status, 400);
    assert_eq!(response.body["error"], "invalid_run_id");
}
```

- [ ] **Step 2: Run and observe failure**

```bash
cargo test -p bunting-server malformed_admin_run_id_is_a_client_error
```

Expected: FAIL because the current parser returns `Err`.

- [ ] **Step 3: Make admin client failures non-fatal**

Introduce status 400 support in `write_http`. In `run()`, change the loop to:

```rust
for accepted in listener.incoming() {
    let stream = match accepted {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("bunting-server: admin accept failed: {error}");
            continue;
        }
    };
    if let Err(error) = handle_owned(stream, config, origin) {
        eprintln!("bunting-server: admin connection closed: {error}");
    }
}
```

Treat request parse/authorization/path errors as HTTP 4xx responses. Reserve returned `Err` for socket write/read failures for that connection; the outer loop logs them and continues.

- [ ] **Step 4: Write a failing FIX terminator-peer classification test**

Extract peer comparison to a pure function:

```rust
#[test]
fn untrusted_terminated_peer_is_rejected_without_listener_failure() {
    let result = terminated_peer_allowed(
        "127.0.0.2".parse().unwrap(),
        "127.0.0.1".parse().unwrap(),
    );
    assert!(!result);
}
```

The listener-level regression test should connect an untrusted peer, then a trusted/normal peer, and prove the accept loop remains available.

- [ ] **Step 5: Move `verify_terminated_peer` inside the per-connection rejection path**

In `acceptor::run`, do not use `?` on peer verification. Write a short rejection when possible, log it, and `continue`. Keep `TcpListener::bind` as fatal. Accept errors should be logged and retried unless an unrecoverable listener condition is explicitly classified.

- [ ] **Step 6: Run package tests**

```bash
cargo test -p bunting-server
```

Expected: PASS, including a regression proving a malformed client does not terminate the listener loop.

- [ ] **Step 7: Commit**

```bash
git add apps/bunting-server/src/admin.rs apps/bunting-server/src/acceptor.rs
git commit -m "fix: isolate native server client failures"
```

---

### Task 5: Derive open-order admission from authoritative state

**Files:**
- Modify: `apps/bunting-server/src/session_host.rs`
- Modify: `packages/bunting-application/src/lib.rs`
- Test: `packages/bunting-application/src/lib.rs`
- Test: `apps/bunting-server/tests/tui_tcp_black_box.rs`

**Interfaces:**
- Produces:

```rust
pub fn participant_open_order_count(
    state: &RunState,
    participant_id: ParticipantId,
) -> usize;
```

The FIX host checks this value immediately before submitting a new order under the authoritative writer gate.

- [ ] **Step 1: Write a failing application projection test**

Create a scenario state with one participant and a resting GTC order, then assert:

```rust
assert_eq!(participant_open_order_count(&state, participant_id), 1);
```

After applying its cancel event/transition, assert zero.

- [ ] **Step 2: Run the test and verify failure**

```bash
cargo test -p bunting-application participant_open_order_count
```

Expected: FAIL because the projection does not exist.

- [ ] **Step 3: Implement the projection using engine-owned ownership/listing state**

Iterate the authoritative resting order/ownership representation already exposed by `RunState`; count only orders owned by `participant_id` that are still live/resting. Do not introduce a second mutable counter.

- [ ] **Step 4: Delete connection-local `open_orders: BTreeSet<_>` from `session_host.rs`**

Immediately before a `SubmitOrder`, while inside `writer.execute_interval`, recover current state and reject when:

```rust
participant_open_order_count(&state, ParticipantId::new(credential.participant_id))
    >= config.max_open_orders
```

Do not increment/decrement any connection-local set after commits.

- [ ] **Step 5: Add reconnect regression coverage**

In `tui_tcp_black_box.rs`, connect participant A, submit resting orders up to the configured limit, disconnect, reconnect with the same participant identity, and submit one more. Assert the new order receives the max-open-orders business reject. Cancel one authoritative order, submit again, and assert acceptance.

- [ ] **Step 6: Run focused and server tests**

```bash
cargo test -p bunting-application participant_open_order_count
cargo test -p bunting-server --test tui_tcp_black_box
cargo test -p bunting-server
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/bunting-application/src/lib.rs apps/bunting-server/src/session_host.rs apps/bunting-server/tests/tui_tcp_black_box.rs
git commit -m "fix: enforce open-order limits from venue state"
```

---

### Task 6: Remove host wall clock from participant market-command logical time

**Files:**
- Modify: `apps/bunting-server/src/session_host.rs`
- Test: `apps/bunting-server/src/session_host.rs`
- Test: `apps/bunting-server/tests/tui_tcp_black_box.rs`

**Interfaces:**
- Consumes: recovered `RunState::simulation().clock.now`.
- Produces: participant `FixCommandContext.logical_time` equal to the authoritative run clock at admission; transport heartbeat timestamps continue using wall time independently.

- [ ] **Step 1: Extract and test logical-time selection**

Add:

```rust
fn command_logical_time(state: &RunState) -> LogicalTimeNs {
    state.simulation().clock.now
}
```

Test with a scenario advanced to a known logical time:

```rust
#[test]
fn participant_command_time_comes_from_run_clock() {
    let state = state_at(LogicalTimeNs::new(42_000_000));
    assert_eq!(command_logical_time(&state), LogicalTimeNs::new(42_000_000));
}
```

Use the package's existing scenario helper rather than constructing an invalid `RunState` by hand.

- [ ] **Step 2: Run and verify the test fails before extraction**

```bash
cargo test -p bunting-server participant_command_time_comes_from_run_clock
```

Expected: FAIL because helper is missing/current path uses `epoch_millis()`.

- [ ] **Step 3: Change `FixCommandContext` construction**

Replace:

```rust
logical_time: LogicalTimeNs::new(epoch_millis().saturating_mul(1_000_000)),
```

with:

```rust
logical_time: command_logical_time(&state),
```

Keep `epoch_millis()` only for FIX session heartbeat/session timers.

- [ ] **Step 4: Add a black-box assertion that wall-clock delay does not advance market logical time**

Start a run at a fixed logical clock, wait at least 20 ms wall time, submit an order, and assert the resulting durable event/request logical timestamp equals the run clock rather than elapsed host time. Advance the simulation explicitly and assert a subsequent order uses the advanced logical time.

- [ ] **Step 5: Run server and workspace validation**

```bash
cargo test -p bunting-server
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-server/src/session_host.rs apps/bunting-server/tests/tui_tcp_black_box.rs
git commit -m "fix: source FIX command time from venue clock"
```

---

### Task 7: Validate the local macOS launch path end-to-end

**Files:**
- Modify: `apps/bunting-terminal/scripts/package-macos-arm64.sh`
- Modify: `.github/workflows/gpui-terminal.yml`
- Modify: `apps/bunting-terminal/README.md`

**Interfaces:**
- Consumes: health probe, Wasmer version validation, native server package.
- Produces: release validation that starts the bundled server from a fresh application-support directory and proves `/health` plus a real FIX reconnect through the same client path used by the terminal.

- [ ] **Step 1: Add an isolated application-support override for tests**

In `local_server.rs`, make `application_server_dir()` honor only a test/release-validation variable named `BUNTING_TERMINAL_SERVER_DIR` before falling back to macOS Application Support. Validate/create the directory normally.

Add a test setting the variable to a temp directory and asserting resolution stays inside it.

- [ ] **Step 2: Run the test and verify the current implementation fails**

```bash
cd apps/bunting-terminal
cargo test application_server_dir_override
```

Expected: FAIL before the override exists.

- [ ] **Step 3: Add workflow smoke launch using fresh state**

In `gpui-terminal.yml`, after packaging/building the server artifact, create a temp directory, set `BUNTING_TERMINAL_SERVER_DIR`, launch the same bundled Wasmer command, poll `http://127.0.0.1:8080/health`, then invoke the existing headless native FIX validation path (`bunting_tui::validate_server` until Task 6 extraction moves it to `bunting-client`).

- [ ] **Step 4: Assert failure logs are retained as workflow artifacts**

On smoke failure, upload the bounded server log and health response artifact. Never upload credentials or a full user config.

- [ ] **Step 5: Run terminal tests and inspect workflow YAML**

```bash
cd apps/bunting-terminal
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

Expected: PASS locally; GPUI workflow PASS on macOS ARM64.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src/local_server.rs apps/bunting-terminal/scripts/package-macos-arm64.sh .github/workflows/gpui-terminal.yml apps/bunting-terminal/README.md
git commit -m "test: exercise bundled local venue launch"
```
