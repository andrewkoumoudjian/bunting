# Bunting Publication-Only Worker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish ADR-0022 by reducing the Cloudflare Worker to read-only publication/query/subscription responsibilities with no dormant command authority or FIX Durable Object runtime.

**Architecture:** Preserve the current D1-backed read projection temporarily because market snapshots/subscriptions still consume it, but delete mutation construction/execution and the FIX session Durable Object. Treat D1 as publication storage only: the Worker reads committed state/event tails produced by the native venue's publication path and cannot create authoritative commands or commit origin state.

**Tech Stack:** Rust Worker, Cloudflare Workers/D1, workerd, browser-wire/api-contract, serde.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- Cloudflare cannot submit, prepare, execute, or commit market/simulation commands.
- `ORIGIN_DB` may remain only as a read-side publication store until a different projection backend is implemented.
- `FIX_SESSIONS`, `FixSessionObject`, `BUNTING_FIX_DESTINATIONS`, and command-authority secrets are removed from the supported Worker surface.
- `/fix-sessions/*` and mutation procedures remain explicit read-only rejections during the compatibility window.
- No deletion of D1 data or migrations is required to complete the runtime cutover; historical migration files may remain as immutable history if Wrangler no longer binds authority objects.

---

### Task 1: Lock the publication-only route contract with failing tests

**Files:**
- Modify: `apps/bunting-worker/src/lib.rs`
- Test: `apps/bunting-worker/src/lib.rs`
- Modify: `apps/bunting-worker/workerd/workerd.capnp`

**Interfaces:**
- Produces a route classifier where only health, read queries, and subscriptions reach dispatch; every `ParsedRequest::Mutation` produces the existing read-only error without touching D1 commit code.

- [ ] **Step 1: Add static route-policy tests**

Add tests asserting representative procedure classes:

```rust
#[test]
fn worker_policy_rejects_mutations_before_dispatch() {
    assert_eq!(route_policy("orders.submit"), RoutePolicy::RejectMutation);
    assert_eq!(route_policy("orders.cancel"), RoutePolicy::RejectMutation);
}

#[test]
fn worker_policy_allows_publication_reads() {
    assert_eq!(route_policy("system.health"), RoutePolicy::Read);
    assert_eq!(route_policy("market.snapshot"), RoutePolicy::Read);
    assert_eq!(route_policy("market.subscribe"), RoutePolicy::Subscribe);
    assert_eq!(route_policy("accounts.subscribe"), RoutePolicy::Subscribe);
}
```

- [ ] **Step 2: Run the tests and verify failure**

```bash
cargo test -p bunting-worker route_policy
```

Expected: FAIL because the explicit pure policy function is absent.

- [ ] **Step 3: Extract `RoutePolicy` without changing external behavior**

Implement:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RoutePolicy { Read, Subscribe, RejectMutation, NotFound }
```

Classify from parsed request/call path before any authentication or D1 mutation logic. Preserve current HTTP status/body for read-only rejection so clients do not see an accidental behavior change.

- [ ] **Step 4: Run Worker unit tests**

```bash
cargo test -p bunting-worker
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/bunting-worker/src/lib.rs apps/bunting-worker/workerd/workerd.capnp
git commit -m "test: lock publication-only worker routes"
```

---

### Task 2: Delete dormant command execution from the Worker

**Files:**
- Modify: `apps/bunting-worker/src/lib.rs`
- Modify: `apps/bunting-worker/Cargo.toml`
- Modify: `Cargo.lock`
- Test: `apps/bunting-worker/src/lib.rs`

**Interfaces:**
- Removes Worker-local uses of `SubmitOrderInput`, `CancelOrderInput`, `Command`, `CommandPayload`, `SimulationCommandRequest`, `prepare_authenticated`, `prepare_authenticated_simulation`, command fingerprints, origin commit requests, and command result mapping where they exist only for mutation execution.
- Keeps read projections, health, subscription planning, and authentication required for private publication reads.

- [ ] **Step 1: Add a source-level architecture guard that fails while authority symbols remain**

In the existing architecture-policy test or a new Worker unit test, inspect `include_str!("lib.rs")` and assert:

```rust
for forbidden in [
    "fn build_submit(",
    "fn build_cancel(",
    "async fn execute_command(",
    "async fn execute_command_detailed(",
    "prepare_authenticated_simulation",
] {
    assert!(!source.contains(forbidden), "forbidden Worker authority symbol: {forbidden}");
}
```

Do not include this guard inside the same source string it scans; if self-reference makes that impossible, place the guard in `apps/bunting-worker/tests/publication_boundary.rs` and inspect `src/lib.rs` from there.

- [ ] **Step 2: Run and verify the guard fails**

```bash
cargo test -p bunting-worker --test publication_boundary
```

Expected: FAIL on at least `build_submit` or `execute_command_detailed`.

- [ ] **Step 3: Delete mutation-only functions and imports**

Remove command builders, command response helpers, transaction error mapping used only by mutation execution, D1 commit helpers invoked only by mutation paths, and mutation dispatch arms. Keep `market.snapshot`, health, public/private subscription, and any private-read authentication.

- [ ] **Step 4: Remove dependencies no longer used by the read-only Worker**

Run:

```bash
cargo check -p bunting-worker
```

Then remove only dependencies made unused by deleting authority code. Re-run `cargo check --locked` after regenerating the lockfile with the repository toolchain.

- [ ] **Step 5: Run the architecture guard and Worker tests**

```bash
cargo test -p bunting-worker --test publication_boundary
cargo test -p bunting-worker
cargo clippy -p bunting-worker --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-worker/src/lib.rs apps/bunting-worker/Cargo.toml Cargo.lock apps/bunting-worker/tests/publication_boundary.rs
git commit -m "refactor: remove worker mutation authority"
```

---

### Task 3: Remove the FIX Durable Object and authority-era bindings

**Files:**
- Delete: `apps/bunting-worker/src/fix_session_object.rs`
- Modify: `apps/bunting-worker/src/lib.rs`
- Modify: `apps/bunting-worker/wrangler.toml`
- Modify: `apps/bunting-worker/workerd/workerd.capnp`
- Modify: `.github/workflows/ci.yml`
- Test: `apps/bunting-worker/tests/publication_boundary.rs`

**Interfaces:**
- Removes runtime binding `FIX_SESSIONS`, class `FixSessionObject`, and variable `BUNTING_FIX_DESTINATIONS`.
- Retains `ORIGIN_DB` as a read-only publication binding for current read/query/subscription paths.

- [ ] **Step 1: Extend the architecture guard**

Assert that `wrangler.toml` contains none of:

```text
FIX_SESSIONS
FixSessionObject
BUNTING_FIX_DESTINATIONS
```

and still contains `binding = "ORIGIN_DB"`.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-worker --test publication_boundary
```

Expected: FAIL on current Wrangler configuration.

- [ ] **Step 3: Remove the Durable Object module and Wrangler binding**

Delete `mod fix_session_object;`, the source file, the `[[durable_objects.bindings]]` stanza, the corresponding new-sqlite-class migration stanza used to deploy that class, and `BUNTING_FIX_DESTINATIONS` from active config. Preserve historical SQL migration files unless a migration file is provably unused and has never been applied; immutable deployment history is safer than rewriting it.

- [ ] **Step 4: Update raw workerd configuration and smoke checks**

Remove FIX Durable Object bindings from `workerd.capnp`. Keep CI's explicit `/fix-sessions/...` 405 check if the HTTP compatibility rejection still exists; otherwise replace it with a route-level assertion that the path is not implemented and cannot reach a Durable Object.

- [ ] **Step 5: Run Worker build and raw workerd smoke**

```bash
cargo test -p bunting-worker
cargo install worker-build --version 0.8.5 --locked
cd apps/bunting-worker
worker-build --release --no-panic-recovery
```

Then run the same `npx workerd@1.20260716.1 serve workerd/workerd.capnp` health/snapshot smoke used by CI.

Expected: health is contract-compatible; publication read without seeded D1 still reports `ORIGIN_UNAVAILABLE`; no FIX Durable Object binding is required.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-worker/src apps/bunting-worker/wrangler.toml apps/bunting-worker/workerd/workerd.capnp .github/workflows/ci.yml apps/bunting-worker/tests/publication_boundary.rs
git commit -m "refactor: remove worker FIX authority runtime"
```

---

### Task 4: Make publication storage semantics explicit in code and docs

**Files:**
- Modify: `apps/bunting-worker/src/d1_origin.rs`
- Modify: `apps/bunting-worker/README.md`
- Modify: `docs/adr/0022-native-competition-venue-and-publication-worker.md`
- Modify: `docs/specs/bunting-product-contract.md`
- Test: `apps/bunting-worker/tests/publication_boundary.rs`

**Interfaces:**
- `d1_origin.rs` exposes read functions only: loading published run state and bounded event tails. No public/private function in the module commits a command or candidate run.

- [ ] **Step 1: Add a D1 source guard**

Assert `d1_origin.rs` contains the read entry points used by `market.snapshot`/subscriptions and does not contain SQL verbs associated with authoritative writes (`INSERT INTO command`, `UPDATE run`, or equivalent actual table mutations found during implementation).

- [ ] **Step 2: Run guard and remove write helpers if any remain**

```bash
cargo test -p bunting-worker --test publication_boundary
```

Expected: PASS only when the D1 module used by Worker runtime is read-side.

- [ ] **Step 3: Update ADR/product docs**

State explicitly: native server owns mutation and origin commit; Cloudflare D1 is a publication replica/store in this deployment shape; Worker serves bounded snapshots/subscriptions and private projections; publication lag is represented by committed sequence/cursor and cannot be mistaken for command authority.

- [ ] **Step 4: Run full root validation**

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo check --locked --workspace --target wasm32-unknown-unknown
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/bunting-worker/src/d1_origin.rs apps/bunting-worker/README.md docs/adr/0022-native-competition-venue-and-publication-worker.md docs/specs/bunting-product-contract.md apps/bunting-worker/tests/publication_boundary.rs
git commit -m "docs: make worker publication storage explicit"
```
