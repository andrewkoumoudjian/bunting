# Bunting Delta-Ready Collaboration Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Bunting-owned, UI-independent collaboration contract plus an in-memory reference backend so shared workspace state can be tested now and DeltaDB can be adopted later without touching exchange authority.

**Architecture:** Create `packages/bunting-collaboration` with versioned workspace document/operation/reference types and a `CollaborationBackend` trait. The first backend is deterministic in-memory state with revisioned operation history and ephemeral presence. GPUI consumes it through a small controller entity; collaboration state can reference immutable Bunting run/event/command IDs but cannot contain or submit market commands, account balances, positions, fills, or risk authority.

**Tech Stack:** Rust 1.88 core package, serde, std synchronization/collections, GPUI Kit consumer integration. No CRDT/network dependency in this plan.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- DeltaDB is not a dependency until its public source, license, protocol, and stability are audited.
- Zed GPL collaboration source is reference-only and is not copied into Bunting.
- Exchange commands/fills/account/risk/run state never flow through the collaboration backend.
- Collaboration failure never blocks FIX trading.
- Durable shared document state and ephemeral presence are separate.
- Collaboration operations are versioned Bunting-owned types so a future `DeltaDbBackend` is an adapter, not a domain rewrite.

---

### Task 1: Create the collaboration domain package

**Files:**
- Create: `packages/bunting-collaboration/Cargo.toml`
- Create: `packages/bunting-collaboration/src/lib.rs`
- Create: `packages/bunting-collaboration/src/model.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Test: `packages/bunting-collaboration/src/model.rs`

**Interfaces:**
- Produces:

```rust
pub const COLLABORATION_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct WorkspaceId(pub String);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct PeerId(pub String);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MarketReference {
    pub run_id: RunId,
    pub event_id: Option<EventId>,
    pub command_id: Option<CommandId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub id: String,
    pub author: PeerId,
    pub body: String,
    pub reference: Option<MarketReference>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub author: PeerId,
    pub body: String,
    pub reference: Option<MarketReference>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SharedWorkspaceDocument {
    pub schema_version: u16,
    pub watchlists: BTreeMap<String, Vec<InstrumentId>>,
    pub annotations: BTreeMap<String, Annotation>,
    pub chat: Vec<ChatMessage>,
    pub shared_layout: Option<String>,
}
```

No type in this package stores authoritative `Command`, fill, balance, position, or risk state.

- [ ] **Step 1: Add package manifest and a failing model round-trip test**

Create the package with dependencies on `bunting-market-types` and serde only. Test:

```rust
let document = SharedWorkspaceDocument::empty();
let json = serde_json::to_string(&document).unwrap();
let decoded: SharedWorkspaceDocument = serde_json::from_str(&json).unwrap();
assert_eq!(decoded, document);
assert_eq!(decoded.schema_version, COLLABORATION_SCHEMA_VERSION);
```

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-collaboration
```

Expected: FAIL until types/constructor exist.

- [ ] **Step 3: Implement exact v1 model**

`SharedWorkspaceDocument::empty()` creates version 1, empty maps/vectors, and no shared layout. Validate IDs/bodies at operation admission rather than silently normalizing serialized documents.

- [ ] **Step 4: Add an authority-boundary source test**

The package test reads its own `Cargo.toml` and asserts it has no dependency on `bunting-engine`, `bunting-command-transaction`, `bunting-origin-store`, `bunting-ledger`, or `bunting-risk-engine`.

- [ ] **Step 5: Run package tests**

```bash
cargo test -p bunting-collaboration
cargo clippy -p bunting-collaboration --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock packages/bunting-collaboration
git commit -m "feat: define collaboration workspace model"
```

---

### Task 2: Define versioned collaboration operations and revision semantics

**Files:**
- Create: `packages/bunting-collaboration/src/operation.rs`
- Modify: `packages/bunting-collaboration/src/lib.rs`
- Test: `packages/bunting-collaboration/src/operation.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum WorkspaceOperation {
    SetWatchlist { name: String, instruments: Vec<InstrumentId> },
    RemoveWatchlist { name: String },
    UpsertAnnotation { annotation: Annotation },
    RemoveAnnotation { id: String },
    AppendChat { message: ChatMessage },
    SetSharedLayout { layout: Option<String> },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceUpdate {
    pub revision: u64,
    pub operation_id: String,
    pub author: PeerId,
    pub operation: WorkspaceOperation,
}

pub fn apply_operation(
    document: &mut SharedWorkspaceDocument,
    update: &WorkspaceUpdate,
) -> Result<(), CollaborationError>;
```

- [ ] **Step 1: Write failing deterministic operation tests**

Apply watchlist, annotation, chat, and removal operations to two empty documents in the same revision order and assert final documents are equal.

Add duplicate chat/annotation IDs and duplicate `operation_id` coverage at backend layer; the pure function applies one already-admitted update.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-collaboration operation
```

Expected: FAIL because operation types are missing.

- [ ] **Step 3: Implement strict validation**

Reject empty operation/annotation/chat IDs, empty watchlist names, chat/annotation bodies above 16 KiB, shared layout strings above 64 KiB, and invalid schema versions. Preserve instrument/reference IDs exactly.

- [ ] **Step 4: Run tests**

```bash
cargo test -p bunting-collaboration operation
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/bunting-collaboration/src
git commit -m "feat: add collaboration operations"
```

---

### Task 3: Add the backend contract and deterministic in-memory implementation

**Files:**
- Create: `packages/bunting-collaboration/src/backend.rs`
- Create: `packages/bunting-collaboration/src/memory.rs`
- Modify: `packages/bunting-collaboration/src/lib.rs`
- Test: `packages/bunting-collaboration/src/memory.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceSnapshot {
    pub revision: u64,
    pub document: SharedWorkspaceDocument,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Presence {
    pub peer: PeerId,
    pub selected_instrument: Option<InstrumentId>,
    pub selected_panel: Option<String>,
}

pub trait CollaborationBackend: Send + Sync {
    fn join(&self, workspace: &WorkspaceId, peer: &PeerId)
        -> Result<WorkspaceSnapshot, CollaborationError>;
    fn apply(
        &self,
        workspace: &WorkspaceId,
        expected_revision: u64,
        update: WorkspaceUpdate,
    ) -> Result<WorkspaceSnapshot, CollaborationError>;
    fn changes_since(
        &self,
        workspace: &WorkspaceId,
        after_revision: u64,
        limit: usize,
    ) -> Result<Vec<WorkspaceUpdate>, CollaborationError>;
    fn set_presence(
        &self,
        workspace: &WorkspaceId,
        presence: Presence,
    ) -> Result<(), CollaborationError>;
    fn presence(&self, workspace: &WorkspaceId)
        -> Result<Vec<Presence>, CollaborationError>;
}
```

Hard maximum `changes_since` limit is 4096.

- [ ] **Step 1: Write failing two-peer convergence test**

Create one `InMemoryCollaborationBackend`, join peers A/B, apply an annotation from A at revision 0, read changes from B after 0, then join/read snapshot and assert both observe revision 1 and identical document.

- [ ] **Step 2: Add failing revision-conflict/idempotency tests**

Applying at stale `expected_revision` returns `CollaborationError::RevisionConflict { current }`. Reusing the same `operation_id` with identical contents returns the existing snapshot without incrementing revision; same ID with different contents returns `OperationConflict`.

- [ ] **Step 3: Run red**

```bash
cargo test -p bunting-collaboration memory
```

Expected: FAIL until backend exists.

- [ ] **Step 4: Implement memory backend under one mutex**

Store per-workspace document, revision, operation fingerprint/map, ordered updates, and presence map. Apply validation before mutation, require expected revision, assign the next revision with checked add, apply operation, then append update atomically.

Presence is not appended to durable update history and does not increment document revision.

- [ ] **Step 5: Add reconnect/replay test**

Apply 10 operations, request `changes_since(3, 4096)`, replay returned updates into a snapshot at revision 3, and assert final document equals backend snapshot at revision 10.

- [ ] **Step 6: Run tests**

```bash
cargo test -p bunting-collaboration
cargo clippy -p bunting-collaboration --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add packages/bunting-collaboration/src
git commit -m "feat: add in-memory collaboration backend"
```

---

### Task 4: Add an explicit DeltaDB adapter seam without a Delta dependency

**Files:**
- Create: `packages/bunting-collaboration/src/delta.rs`
- Modify: `packages/bunting-collaboration/src/lib.rs`
- Test: `packages/bunting-collaboration/src/delta.rs`
- Modify: `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

**Interfaces:**
- Produces adapter-neutral mapping structures only:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplicatedOperationEnvelope {
    pub schema_version: u16,
    pub workspace_id: WorkspaceId,
    pub update: WorkspaceUpdate,
}

pub trait ReplicatedTransport: Send + Sync {
    fn publish(&self, envelope: &ReplicatedOperationEnvelope) -> Result<(), CollaborationError>;
    fn fetch_after(
        &self,
        workspace: &WorkspaceId,
        revision: u64,
        limit: usize,
    ) -> Result<Vec<ReplicatedOperationEnvelope>, CollaborationError>;
}
```

This is intentionally named generically. No `DeltaDbBackend` concrete type is compiled until DeltaDB source/license review passes.

- [ ] **Step 1: Write a round-trip test for replicated envelopes**

Serialize one annotation update containing an immutable `MarketReference`, deserialize, and assert exact equality.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-collaboration replicated_operation
```

Expected: FAIL until envelope exists.

- [ ] **Step 3: Implement the transport seam only**

Add no HTTP/WebSocket/CRDT dependency. Document in Rustdoc that future DeltaDB integration maps Delta operations/documents to Bunting's versioned envelope and may not add market mutation methods to this package.

- [ ] **Step 4: Update the design spec's Delta feasibility date only if new public source evidence exists**

If no public DeltaDB source is available at implementation time, keep the current finding unchanged. If source is now public, stop this task before importing anything and perform a separate license/source audit plan.

- [ ] **Step 5: Run tests and dependency tree**

```bash
cargo test -p bunting-collaboration
cargo tree -p bunting-collaboration
```

Expected: no Zed/Delta/CRDT/network dependency.

- [ ] **Step 6: Commit**

```bash
git add packages/bunting-collaboration/src/delta.rs packages/bunting-collaboration/src/lib.rs docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md
git commit -m "feat: add Delta-ready replication seam"
```

---

### Task 5: Integrate collaboration as a non-blocking GPUI controller

**Files:**
- Create: `apps/bunting-terminal/src/collaboration.rs`
- Modify: `apps/bunting-terminal/src/main.rs`
- Modify: `apps/bunting-terminal/Cargo.toml`
- Modify: `apps/bunting-terminal/src/terminal/panel.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_research.rs`
- Test: `apps/bunting-terminal/tests/collaboration_ui.rs`

**Interfaces:**
- Adds `bunting-collaboration = { path = "../../packages/bunting-collaboration" }`.
- Produces `CollaborationController` that owns an `Arc<dyn CollaborationBackend>`, active workspace/peer, last snapshot, presence, and a separate collaboration status enum. It has no reference to FIX outbound command queues.

- [ ] **Step 1: Add failing UI/controller isolation test**

Construct the controller with an in-memory backend and a fake trading client. Make the backend return `CollaborationError::Unavailable`, then assert the terminal's FIX connected state and order-submit availability are unchanged while collaboration status becomes disconnected.

- [ ] **Step 2: Run red**

```bash
cd apps/bunting-terminal
cargo test --features test-support --test collaboration_ui
```

Expected: FAIL because controller does not exist.

- [ ] **Step 3: Implement controller join/apply/presence methods**

Keep methods narrow: `join_workspace`, `upsert_annotation`, `append_chat`, `set_presence`, `refresh`. Errors update collaboration status only; they never call `Terminal::set_status` in a way that marks FIX/venue offline.

- [ ] **Step 4: Add Research workspace collaboration panel**

Add a `Collaboration` panel adjacent to Research/News rather than to the primary trading order path. Render peer presence, annotations, and chat with GPUI Kit lists/inputs. References to market events are links/labels only.

- [ ] **Step 5: Prove no market command API is reachable from collaboration module**

Add a source-boundary test asserting `apps/bunting-terminal/src/collaboration.rs` does not import `OutboundCmd`, `new_order`, `cancel`, `CommandPayload`, or `SimulationCommand`.

- [ ] **Step 6: Run terminal + collaboration tests**

```bash
cargo test -p bunting-collaboration
cd apps/bunting-terminal
cargo test --features test-support
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/bunting-terminal packages/bunting-collaboration Cargo.toml Cargo.lock
git commit -m "feat: add non-authoritative workspace collaboration"
```

---

### Task 6: Add collaboration architecture guards and documentation

**Files:**
- Create: `tools/check_collaboration_boundary.py`
- Modify: `.github/workflows/ci.yml`
- Modify: `AGENTS.md`
- Modify: `README.md`
- Modify: `docs/specs/bunting-product-contract.md`

**Interfaces:**
- CI enforces that `bunting-collaboration` has no engine/origin/command/risk/ledger authority dependency and the terminal collaboration controller cannot import FIX command constructors.

- [ ] **Step 1: Implement the source/dependency checker**

Parse `packages/bunting-collaboration/Cargo.toml` and reject dependency keys:

```python
FORBIDDEN_DEPS = {
    "bunting-engine",
    "bunting-command-transaction",
    "bunting-origin-store",
    "bunting-risk-engine",
    "bunting-ledger",
}
```

Scan `apps/bunting-terminal/src/collaboration.rs` import lines for `OutboundCmd`, `new_order`, `cancel`, `CommandPayload`, and `SimulationCommand`.

- [ ] **Step 2: Run checker**

```bash
python3 tools/check_collaboration_boundary.py
```

Expected: PASS.

- [ ] **Step 3: Add checker to CI architecture policy**

Run before compilation/tests so a boundary regression fails quickly.

- [ ] **Step 4: Document product semantics**

README/product contract must distinguish exchange multiplayer (server/FIX authority) from workspace collaboration (annotations/chat/presence/watchlists). State that current shared-workspace backend is local/in-memory reference infrastructure, not remote DeltaDB multiplayer.

- [ ] **Step 5: Run root validation**

```bash
python3 tools/check_collaboration_boundary.py
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add tools/check_collaboration_boundary.py .github/workflows/ci.yml AGENTS.md README.md docs/specs/bunting-product-contract.md
git commit -m "ci: enforce collaboration authority boundary"
```
