# Bunting Native Client Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract all reusable native FIX transport/session/projection logic from `apps/bunting-tui` into `packages/bunting-client` so GPUI and Ratatui presentations share one UI-independent client implementation.

**Architecture:** Create a root-workspace package owning connection profile types, transport, bounded I/O, FIX protocol/reducers, headless server validation, and authoritative projections. Keep TUI workspace/layout persistence and fixture UI concerns in `apps/bunting-tui`; update both applications to depend directly on the package, then remove the temporary `bunting_tui::client` re-export.

**Tech Stack:** Rust 1.88 package, tokio, rustls/native transport dependencies already used by TUI, simfix-wire/session/mapping, bunting-application projection types.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- `packages/bunting-client` must not depend on any `apps/*` crate.
- The package contains no Ratatui, GPUI, dock/layout, chart-rendering, or local account/fill authority.
- Secrets remain environment-loaded at connection time and are never serialized.
- FIX session/recovery/reconnect behavior remains single-source.
- The GPUI terminal no longer depends on `bunting-tui` after this plan.
- Existing TUI configuration JSON remains readable without migration.

---

### Task 1: Create the `bunting-client` package with connection-profile types

**Files:**
- Create: `packages/bunting-client/Cargo.toml`
- Create: `packages/bunting-client/src/lib.rs`
- Create: `packages/bunting-client/src/config.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `apps/bunting-tui/src/config.rs`
- Test: `packages/bunting-client/src/config.rs`
- Test: `apps/bunting-tui/src/config.rs`

**Interfaces:**
- Produces public types moved without semantic change:

```rust
pub const FIX_PROFILE_VERSION: &str = "bunting.fixlatest.competition.v1";
pub enum ActorRole { Participant, Team, Instructor, Administrator }
pub enum TransportConfig { Tcp, Tls { server_name: String, ca_file: Option<PathBuf> } }
pub struct ConnectionProfile { /* existing fields unchanged */ }
```

TUI keeps:

```rust
pub struct WorkspaceLayout { /* current UI fields */ }
pub struct TerminalConfig {
    pub selected_profile: String,
    pub profiles: BTreeMap<String, bunting_client::ConnectionProfile>,
    pub workspaces: BTreeMap<String, WorkspaceLayout>,
}
```

- [ ] **Step 1: Add a failing package membership/dependency test through Cargo**

Create the package manifest and minimal `lib.rs` exporting `config`, but leave types absent. Add `packages/bunting-client` to workspace members/default-members and `bunting-client = { path = "packages/bunting-client" }` to workspace dependencies.

Run:

```bash
cargo check -p bunting-client
```

Expected: FAIL until `config.rs` and declared exports exist.

- [ ] **Step 2: Move only connection/profile types into the package**

Move `FIX_PROFILE_VERSION`, `ActorRole`, `TransportConfig`, `ConnectionProfile`, and `default_heartbeat()` from TUI config into `packages/bunting-client/src/config.rs` with their current serde representation and validation/password methods unchanged.

Export from `lib.rs`:

```rust
pub use config::{ActorRole, ConnectionProfile, TransportConfig, FIX_PROFILE_VERSION};
```

- [ ] **Step 3: Add package-level profile tests**

Copy/adapt the existing no-secret/default validation test so it proves serialization of a `ConnectionProfile` never includes a password value and invalid endpoint/TLS server-name/heartbeat are rejected.

- [ ] **Step 4: Make TUI config consume package profile types**

In `apps/bunting-tui/src/config.rs`, import the package types and keep only `WorkspaceLayout`, `TerminalConfig`, config path/load/save/profile selection. Preserve exact JSON field names, so existing `terminal.json` files deserialize unchanged.

- [ ] **Step 5: Run config/package tests**

```bash
cargo test -p bunting-client
cargo test -p bunting-tui config
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock packages/bunting-client apps/bunting-tui/src/config.rs
git commit -m "refactor: create native bunting client package"
```

---

### Task 2: Move transport ownership into `bunting-client`

**Files:**
- Create: `packages/bunting-client/src/transport.rs`
- Modify: `packages/bunting-client/src/lib.rs`
- Modify: `packages/bunting-client/Cargo.toml`
- Modify: `apps/bunting-tui/Cargo.toml`
- Delete: `apps/bunting-tui/src/transport.rs`
- Test: `packages/bunting-client/src/transport.rs`

**Interfaces:**
- Produces the same transport abstraction currently consumed by `FixClient`/`IoTask`; exported visibility is limited to what protocol/io modules need, with only connection errors/status exposed publicly.

- [ ] **Step 1: Copy transport tests into the package before moving implementation**

Move the existing `transport.rs` test module first, update imports to `crate::config`, and run:

```bash
cargo test -p bunting-client transport
```

Expected: FAIL because transport implementation is absent.

- [ ] **Step 2: Move the production transport code without behavior changes**

Move TCP/TLS connect/read/write code into `packages/bunting-client/src/transport.rs`. Use package `ConnectionProfile`/`TransportConfig`. Preserve bounded reads, TLS server-name validation, CA handling, timeout/error strings, and unsafe-code prohibition.

- [ ] **Step 3: Update package dependencies**

Move only the transport dependencies from `apps/bunting-tui/Cargo.toml` to `packages/bunting-client/Cargo.toml`. Do not add Ratatui/crossterm/GPUI dependencies.

- [ ] **Step 4: Run transport and package checks**

```bash
cargo test -p bunting-client transport
cargo clippy -p bunting-client --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/bunting-client apps/bunting-tui/Cargo.toml Cargo.lock
git commit -m "refactor: move native transport into bunting-client"
```

---

### Task 3: Move FIX protocol, authoritative reducers, and constructors

**Files:**
- Create: `packages/bunting-client/src/protocol.rs`
- Modify: `packages/bunting-client/src/lib.rs`
- Modify: `packages/bunting-client/Cargo.toml`
- Delete: `apps/bunting-tui/src/protocol.rs`
- Test: `packages/bunting-client/src/protocol.rs`

**Interfaces:**
- Produces/export existing client surface plus authoritative history from the preceding plan:

```rust
pub use protocol::{
    Book,
    Execution,
    FixClient,
    Portfolio,
    PriceSample,
    book_request,
    cancel,
    competition_action,
    competition_requests,
    market_history_request,
    new_order,
};
```

`FixClient` remains the single reducer for connection/session state, book, executions, authoritative account, risk, discovery, news/tenders, score, and market history.

- [ ] **Step 1: Move reducer tests before implementation**

Copy the entire current `protocol.rs` test module plus market-history tests into the package file and update paths.

Run:

```bash
cargo test -p bunting-client protocol
```

Expected: FAIL until production protocol code is present.

- [ ] **Step 2: Move protocol code mechanically**

Move FIX message constructors, `FixClient`, projection types currently local to the client, session handling/reducer logic, and message parsing. Import `simfix-*` and Bunting projection crates directly from package dependencies.

Do not move TUI rendering state or quote-candle helpers.

- [ ] **Step 3: Remove duplicate profile constants**

Use `crate::FIX_PROFILE_VERSION`/the canonical API contract constant in logon/profile verification. There must be one semantic source for the competition profile version.

- [ ] **Step 4: Run package tests and Clippy**

```bash
cargo test -p bunting-client
cargo clippy -p bunting-client --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/bunting-client apps/bunting-tui/src/protocol.rs Cargo.lock
git commit -m "refactor: move FIX reducer into bunting-client"
```

---

### Task 4: Move bounded async I/O ownership into `bunting-client`

**Files:**
- Create: `packages/bunting-client/src/io_task.rs`
- Modify: `packages/bunting-client/src/lib.rs`
- Delete: `apps/bunting-tui/src/io_task.rs`
- Test: `packages/bunting-client/src/io_task.rs`

**Interfaces:**
- Produces:

```rust
pub use io_task::{IoTask, OutboundCmd, UiEvent};
```

Keep current queue capacities/backpressure semantics and reconnect ownership. Rename `UiEvent` only if it actually contains presentation-specific payloads; preferred compatible name for this migration is `ClientEvent`, with a temporary `pub type UiEvent = ClientEvent` alias if downstream churn would otherwise obscure behavior.

- [ ] **Step 1: Move I/O tests into the package and run red**

Copy existing bounded queue/reconnect tests and update imports.

```bash
cargo test -p bunting-client io_task
```

Expected: FAIL until implementation is moved.

- [ ] **Step 2: Move `IoTask`, outbound command, event, and reconnect loop**

The package task owns `FixClient` and transport; it emits immutable/snapshot events to consumers. Keep all bounded channels exactly bounded and preserve cancellation/drop behavior.

- [ ] **Step 3: Run package tests**

```bash
cargo test -p bunting-client
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add packages/bunting-client apps/bunting-tui/src/io_task.rs
git commit -m "refactor: move FIX I/O task into bunting-client"
```

---

### Task 5: Move headless server validation into the client package

**Files:**
- Modify: `packages/bunting-client/src/lib.rs`
- Modify: `apps/bunting-tui/src/lib.rs`
- Test: `packages/bunting-client/src/lib.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessValidation {
    pub verified_role: String,
    pub committed_sequence: String,
    pub observed_projections: Vec<String>,
}

pub async fn validate_server(
    endpoint: &str,
    password: &str,
) -> Result<HeadlessValidation, String>;
```

- [ ] **Step 1: Add a compile-time consumer test in `bunting-client`**

Add a test that references `validate_server` and `HeadlessValidation` from crate root so public API drift is caught.

- [ ] **Step 2: Move the current implementation from TUI lib**

Use package `ConnectionProfile` defaults via a private `local_validation_profile(endpoint)` constructor rather than depending on TUI `TerminalConfig`. The profile uses the current local sender/target IDs and TCP transport; password remains the explicit function argument.

- [ ] **Step 3: Keep a temporary TUI re-export only if existing callers require it**

If `apps/bunting-server` tests currently import `bunting_tui::validate_server`, change them to `bunting_client::validate_server` in the same commit. Prefer deleting the TUI forwarding function rather than maintaining two public APIs.

- [ ] **Step 4: Run dependent tests**

```bash
cargo test -p bunting-client
cargo test -p bunting-server --test tui_tcp_black_box
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/bunting-client/src/lib.rs apps/bunting-tui/src/lib.rs apps/bunting-server/tests Cargo.lock
git commit -m "refactor: move server validation into bunting-client"
```

---

### Task 6: Convert `bunting-tui` to a presentation consumer

**Files:**
- Modify: `apps/bunting-tui/Cargo.toml`
- Modify: `apps/bunting-tui/src/lib.rs`
- Modify: `apps/bunting-tui/src/tui.rs`
- Modify: files under `apps/bunting-tui/src/tui/`
- Keep: `apps/bunting-tui/src/local_market.rs`
- Keep: `apps/bunting-tui/src/chart/` until final Ratatui retirement plan
- Test: existing `apps/bunting-tui` tests

**Interfaces:**
- Consumes: `bunting_client::{ConnectionProfile, FixClient, IoTask, OutboundCmd, ClientEvent/...}`.
- Produces: TUI app behavior unchanged; no reusable client implementation remains under the app crate.

- [ ] **Step 1: Change imports to the new package**

Replace `crate::protocol`, `crate::io_task`, and transport/profile implementation imports throughout TUI rendering/controller code with `bunting_client` imports. Keep TUI-only `TerminalConfig` for workspace persistence.

- [ ] **Step 2: Delete the temporary `pub mod client` re-export**

Remove the re-export block from `apps/bunting-tui/src/lib.rs`. Any resulting downstream compile failure identifies a consumer that still needs direct `bunting-client` migration.

- [ ] **Step 3: Run the TUI and server tests**

```bash
cargo test -p bunting-tui
cargo test -p bunting-server
```

Expected: PASS.

- [ ] **Step 4: Enforce package direction**

Run:

```bash
cargo tree -p bunting-client
```

Expected: output contains no `bunting-tui`, `bunting-terminal`, Ratatui, or GPUI package.

- [ ] **Step 5: Commit**

```bash
git add apps/bunting-tui packages/bunting-client Cargo.toml Cargo.lock
git commit -m "refactor: make TUI consume shared client"
```

---

### Task 7: Point the GPUI terminal directly at `bunting-client`

**Files:**
- Modify: `apps/bunting-terminal/Cargo.toml`
- Modify: `apps/bunting-terminal/src/terminal.rs`
- Modify: `apps/bunting-terminal/src/terminal/state.rs`
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Modify: terminal source imports under `apps/bunting-terminal/src/`
- Modify: `apps/bunting-terminal/Cargo.lock`

**Interfaces:**
- Removes dependency `bunting-tui = { path = "../bunting-tui" }`.
- Adds `bunting-client = { path = "../../packages/bunting-client" }`.

- [ ] **Step 1: Change the manifest first and run check red**

Replace the TUI dependency with the client package but do not change source imports yet.

Run:

```bash
cd apps/bunting-terminal
cargo check
```

Expected: FAIL on `bunting_tui::client` imports.

- [ ] **Step 2: Replace every `bunting_tui::client` import with `bunting_client`**

Do not add compatibility aliases in the desktop app. Update types/method paths directly.

- [ ] **Step 3: Regenerate the standalone lockfile and verify source graph**

```bash
cd apps/bunting-terminal
cargo update
cargo check
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: PASS and `cargo tree` contains `bunting-client` but not `bunting-tui`.

- [ ] **Step 4: Add a workflow source-graph assertion**

In `.github/workflows/gpui-terminal.yml`, assert:

```bash
! cargo tree --manifest-path apps/bunting-terminal/Cargo.toml | grep -q '^bunting-tui '
cargo tree --manifest-path apps/bunting-terminal/Cargo.toml | grep -F 'bunting-client v'
```

- [ ] **Step 5: Commit**

```bash
git add apps/bunting-terminal packages/bunting-client .github/workflows/gpui-terminal.yml
git commit -m "refactor: decouple GPUI terminal from Ratatui app"
```

---

### Task 8: Add an architecture guard for the client/UI boundary

**Files:**
- Create: `tools/check_client_boundary.py`
- Modify: `.github/workflows/ci.yml`
- Modify: `AGENTS.md`

**Interfaces:**
- CI fails if `packages/bunting-client` imports/depends on `apps`, Ratatui, crossterm, GPUI, `gpui-kit`, or UI chart modules; and fails if `apps/bunting-terminal/Cargo.toml` depends on `bunting-tui`.

- [ ] **Step 1: Write the checker with exact forbidden patterns**

Use Python `pathlib` to inspect `packages/bunting-client/Cargo.toml` and Rust files. Exit nonzero with path/pattern for any violation. The forbidden dependency names are:

```python
FORBIDDEN = ("bunting-tui", "bunting-terminal", "ratatui", "crossterm", "gpui", "gpui-kit")
```

Permit the substring `gpui` in prose comments only by parsing manifest dependency keys separately and Rust `use`/`extern crate` lines rather than blanket text matching.

- [ ] **Step 2: Run checker locally**

```bash
python3 tools/check_client_boundary.py
```

Expected: PASS.

- [ ] **Step 3: Add checker to canonical CI before tests**

Run it in the architecture dependency policy step.

- [ ] **Step 4: Run full validation**

```bash
python3 tools/check_client_boundary.py
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add tools/check_client_boundary.py .github/workflows/ci.yml AGENTS.md
git commit -m "ci: enforce native client UI boundary"
```
