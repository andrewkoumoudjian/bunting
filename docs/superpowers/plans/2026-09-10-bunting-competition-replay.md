# Bunting Competition Replay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make competition ordering and archives represent and deterministically replay every accepted participant and simulation command in one venue-global admission order.

**Architecture:** Introduce a protocol-neutral accepted-command enum in `market-events`, expose venue-global admission metadata from `AuthoritativeWriter`, and record that metadata alongside accepted commands in archive v2. Replay uses one unified transition helper that dispatches participant and simulation commands through the same engine preparation paths used by live transactions, verifies ordering metadata, canonical events, and final state hash. The existing `tests/goldens/competition-full-run.v1.json` remains an engine simulation golden; archive v2 receives a separate `competition-archive.v2.json` golden so two distinct contracts are not conflated.

**Tech Stack:** Rust 1.88, serde, Bunting engine/market-events/command-transaction/origin-store/application/server, JSON competition archives.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- One native venue owns the authoritative commit and ordering sequence.
- The matching interval policy is versioned and archive-visible.
- Participant orders/cancels and simulation/operator commands are both present in the accepted-command archive.
- Replay cannot rely on wall-clock time or network timing.
- Reordered, duplicated-with-conflict, missing, or event-divergent archives fail verification.
- Existing archive schema v1 is explicitly unsupported as definitive competition evidence; it is not silently upgraded.
- `tests/goldens/competition-full-run.v1.json` is not the `CompetitionArchive` schema and must not be renamed or deleted by this work.

---

### Task 1: Add a unified protocol-neutral accepted command

**Files:**
- Modify: `packages/market-events/src/lib.rs`
- Test: `packages/market-events/src/lib.rs`

**Interfaces:**
- Produces:

```rust
pub const ACCEPTED_COMMAND_SCHEMA_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "command")]
pub enum AcceptedCommand {
    Participant(Command),
    Simulation(SimulationCommandRequest),
}

impl AcceptedCommand {
    pub const fn run_id(&self) -> RunId;
    pub const fn command_id(&self) -> CommandId;
    pub const fn logical_time(&self) -> LogicalTimeNs;
    pub const fn expected_sequence(&self) -> EventSequence;
    pub const fn actor(&self) -> ParticipantId;
}
```

- [ ] **Step 1: Write the failing round-trip test**

Add test fixtures for one participant `ActivateKillSwitch` command and one `SimulationCommand::StartRun` request with run 1, command IDs 10 and 11, correlation IDs 10 and 11, actor 1, logical time 0, and expected sequence 0. Then assert:

```rust
#[test]
fn accepted_command_round_trips_both_command_domains() {
    for command in [
        AcceptedCommand::Participant(sample_participant_command()),
        AcceptedCommand::Simulation(sample_simulation_command()),
    ] {
        let json = serde_json::to_string(&command).expect("serialize");
        let decoded: AcceptedCommand = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, command);
    }
}
```

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-market-events accepted_command_round_trips_both_command_domains
```

Expected: FAIL because `AcceptedCommand` is undefined.

- [ ] **Step 3: Implement enum and accessors**

Each accessor delegates to the wrapped `Command` or `SimulationCommandRequest`; no command field is duplicated in `AcceptedCommand`.

- [ ] **Step 4: Run market-events tests**

```bash
cargo test -p bunting-market-events
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/market-events/src/lib.rs
git commit -m "feat: unify accepted competition commands"
```

---

### Task 2: Add one pure preparation path for live/replay command kinds

**Files:**
- Modify: `packages/command-transaction/src/lib.rs`
- Test: `packages/command-transaction/src/lib.rs`

**Interfaces:**
- Consumes: `bunting_market_events::AcceptedCommand`.
- Produces:

```rust
pub fn prepare_accepted_command(
    command: &AcceptedCommand,
    state: &RunState,
    cached: Option<&CachedSnapshot>,
) -> Result<PreparedCommand, TransactionError>;
```

- [ ] **Step 1: Write failing equivalence tests**

For a valid participant submit order, compare `prepare_command()` with `prepare_accepted_command(AcceptedCommand::Participant(...))` and assert identical event vectors and candidate state hashes. Repeat with `prepare_simulation_command()` and `AcceptedCommand::Simulation(...)` for `StartRun`.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-command-transaction prepare_accepted_command
```

Expected: FAIL because the helper is absent.

- [ ] **Step 3: Implement dispatcher**

```rust
pub fn prepare_accepted_command(
    command: &AcceptedCommand,
    state: &RunState,
    cached: Option<&CachedSnapshot>,
) -> Result<PreparedCommand, TransactionError> {
    match command {
        AcceptedCommand::Participant(command) => prepare_command(command, state, cached),
        AcceptedCommand::Simulation(command) => prepare_simulation_command(command, state),
    }
}
```

For participant replay, the caller supplies the same cached listing snapshot semantics as `execute_detailed`; do not bypass engine cache inputs for command kinds that require them.

- [ ] **Step 4: Run command-transaction tests**

```bash
cargo test -p bunting-command-transaction
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/command-transaction/src/lib.rs
git commit -m "feat: add unified accepted-command preparation"
```

---

### Task 3: Expose venue-global admission metadata from `AuthoritativeWriter`

**Files:**
- Modify: `apps/bunting-server/src/writer.rs`
- Modify: `apps/bunting-server/src/session_host.rs`
- Modify: `apps/bunting-server/src/scenario.rs`
- Test: `apps/bunting-server/src/writer.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VenueAdmission {
    pub interval_index: u64,
    pub arrival_sequence: u64,
}

pub(crate) fn execute_interval<T>(
    &self,
    action: impl FnOnce(VenueAdmission) -> Result<T, String>,
) -> Result<T, String>;
```

- [ ] **Step 1: Write failing global-order tests**

Use four threads sharing one `AuthoritativeWriter`, collect admissions from the closure, and assert committed `arrival_sequence` values are exactly `0,1,2,3`. Add a second test with a long interval proving near-simultaneous admissions can share one `interval_index` while preserving strictly increasing arrival sequence.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-server writer::tests
```

Expected: compile/test failure because the closure currently receives no admission metadata.

- [ ] **Step 3: Implement metadata at admission**

Assign `arrival_sequence` from the existing writer-global atomic counter before waiting for the commit turn. Compute `interval_index` only from writer `Instant` elapsed time and configured interval:

```rust
let interval_nanos = self.interval.as_nanos().max(1);
let interval_index = u64::try_from(self.started.elapsed().as_nanos() / interval_nanos)
    .unwrap_or(u64::MAX);
```

Pass `VenueAdmission` to the closure after the writer obtains its turn. Do not use `SystemTime`.

- [ ] **Step 4: Update every writer call site**

Change server closures in `session_host.rs` and `scenario.rs` to accept the admission argument. Call sites not yet recording it use `_admission`; participant/simulation command paths retain it for Task 6.

- [ ] **Step 5: Run server tests**

```bash
cargo test -p bunting-server
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-server/src/writer.rs apps/bunting-server/src/session_host.rs apps/bunting-server/src/scenario.rs
git commit -m "feat: expose venue-global admission order"
```

---

### Task 4: Introduce `CompetitionArchive` schema v2 without rewriting the engine golden

**Files:**
- Modify: `bunting-rs/Cargo.toml`
- Modify: `bunting-rs/src/archive.rs`
- Modify: `bunting-rs/examples/competition_archive.rs`
- Create: `tests/goldens/competition-archive.v2.json`
- Test: `bunting-rs/src/archive.rs`

**Interfaces:**
- Produces:

```rust
pub const COMPETITION_ARCHIVE_VERSION: u16 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArchivedAcceptedCommand {
    pub interval_index: u64,
    pub arrival_sequence: u64,
    pub authenticated_role: String,
    pub command: AcceptedCommand,
}
```

`CompetitionArchive.accepted_commands` becomes `Vec<ArchivedAcceptedCommand>`.

- [ ] **Step 1: Write a failing mixed-command archive fixture**

Create an initial scenario, apply `StartRun`, then a participant resting limit order at the resulting expected sequence. Construct two archive records with arrival sequences 0 and 1 and roles `administrator` and `participant`. Assert JSON round trip preserves both enum variants.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-rs archive_round_trip_replays_participant_and_simulation_commands
```

Expected: FAIL because archive v1 stores `Vec<SimulationCommandRequest>`.

- [ ] **Step 3: Implement schema v2 and strict admission validation**

`validate()` requires first arrival sequence `0`, then for each adjacent record:

```rust
if next.arrival_sequence != current.arrival_sequence.saturating_add(1) {
    return Err(ArchiveError::InvalidAdmissionOrder);
}
if next.interval_index < current.interval_index {
    return Err(ArchiveError::InvalidAdmissionOrder);
}
```

Also preserve current positive policy checks and initial snapshot validation.

- [ ] **Step 4: Make archive-v1 handling explicit**

`from_json()` inspects `schema_version`; version 1 returns `ArchiveError::UnsupportedVersion`. Do not reinterpret a simulation-only archive as complete evidence.

- [ ] **Step 5: Generate a new archive-specific golden**

Update `bunting-rs/examples/competition_archive.rs` to produce a v2 archive containing at least one simulation and one participant command. Capture its deterministic pretty JSON in `tests/goldens/competition-archive.v2.json` and add a test that the example-equivalent archive serializes to the same JSON value.

Do not alter `tests/goldens/competition-full-run.v1.json`; it remains the simulation-domain golden consumed by `packages/bunting-engine/tests/simulation_domain.rs`.

- [ ] **Step 6: Run archive and CLI checks**

```bash
cargo test -p bunting-rs
cargo run --locked -q -p bunting-rs --example competition_archive > /tmp/competition.archive.json
cargo run --locked -q -p bunting-cli -- replay /tmp/competition.archive.json
cargo run --locked -q -p bunting-cli -- score /tmp/competition.archive.json
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add bunting-rs/Cargo.toml bunting-rs/src/archive.rs bunting-rs/examples/competition_archive.rs tests/goldens/competition-archive.v2.json
git commit -m "feat: archive complete competition command stream"
```

---

### Task 5: Replay mixed command kinds through the unified preparation path

**Files:**
- Modify: `bunting-rs/src/archive.rs`
- Test: `bunting-rs/src/archive.rs`

**Interfaces:**
- Consumes: `prepare_accepted_command()` from Task 2.
- Produces: strict mixed-domain `CompetitionArchive::replay()`.

- [ ] **Step 1: Add failing negative replay tests**

Add exact tests:

```text
replay_rejects_reordered_arrivals
replay_rejects_missing_arrival_sequence
replay_rejects_participant_event_drift
replay_rejects_expected_sequence_gap
replay_rejects_final_hash_drift
```

For expected-sequence gap, mutate the second wrapped command's `expected_sequence` to a non-current sequence and require `ArchiveError::CommandRejected(1)`.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-rs replay_rejects_
```

Expected: mixed participant replay tests fail under the old simulation-only implementation.

- [ ] **Step 3: Implement unified replay**

For each record, validate its command's `run_id` equals the archive initial run. Resolve the participant listing snapshot/cache input exactly as live preparation requires, then call:

```rust
let prepared = bunting_command_transaction::prepare_accepted_command(
    &record.command,
    &state,
    cached.as_ref(),
)
.map_err(|_| ArchiveError::CommandRejected(index))?;

events.extend(prepared.commit.events.clone());
state = prepared.commit.candidate;
```

Never synthesize a participant trade/order event separately in replay.

- [ ] **Step 4: Verify canonical events and final hash**

Keep exact vector equality against `canonical_events`, then compute and compare `state.state_hash()`. Return command/event counts and final score projection as today.

- [ ] **Step 5: Run tests**

```bash
cargo test -p bunting-rs
cargo test -p bunting-cli
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add bunting-rs/src/archive.rs
git commit -m "fix: replay complete competition archives"
```

---

### Task 6: Record accepted live commands with the writer admission metadata

**Files:**
- Create: `apps/bunting-server/src/archive_recorder.rs`
- Modify: `apps/bunting-server/src/lib.rs`
- Modify: `apps/bunting-server/src/runtime.rs`
- Modify: `apps/bunting-server/src/session_host.rs`
- Modify: `apps/bunting-server/src/scenario.rs`
- Modify: `apps/bunting-server/src/config.rs`
- Test: `apps/bunting-server/src/archive_recorder.rs`
- Test: `apps/bunting-server/tests/path_equivalence.rs`

**Interfaces:**
- Produces:

```rust
pub(crate) trait AcceptedCommandRecorder: Send + Sync {
    fn record(
        &self,
        admission: VenueAdmission,
        authenticated_role: &str,
        command: &AcceptedCommand,
        events: &[EventEnvelope],
        state: &RunState,
    ) -> Result<(), String>;
}
```

`MemoryAcceptedCommandRecorder` is used in tests. A file-backed recorder is enabled only by an explicit archive-output server config field and writes after an authoritative accepted non-duplicate commit.

- [ ] **Step 1: Write failing memory-recorder test**

Record one simulation and one participant command with admissions 0 and 1, then assert stored command order, roles, event sequences, and final state sequence match the arguments exactly.

- [ ] **Step 2: Run red**

```bash
cargo test -p bunting-server archive_recorder
```

Expected: FAIL because recorder does not exist.

- [ ] **Step 3: Implement memory recorder**

Store the ordered records and event/state metadata behind a `Mutex`. Poisoned synchronization returns a stable error; no panic/unwrap is permitted in production code.

- [ ] **Step 4: Wire participant commits**

Inside `writer.execute_interval(|admission| ...)`, after `ApplicationService::execute` returns an accepted, non-duplicate commit, call recorder with `AcceptedCommand::Participant(command.clone())`, the verified actor role, committed events, and committed state. Rejected commands do not enter `accepted_commands`.

- [ ] **Step 5: Wire simulation/operator commits**

Pass the same recorder to scenario/operator mutation paths. Accepted non-duplicate simulation commits record `AcceptedCommand::Simulation(request.clone())` using the same writer admission source.

- [ ] **Step 6: Add two-participant live-to-replay test**

Drive two authenticated participant sessions through one shared writer, include at least one simulation command, capture the recorder stream, build a `CompetitionArchive`, and call `replay()`. Assert canonical event vector and final hash equal the live run.

- [ ] **Step 7: Run server/replay/workspace tests**

```bash
cargo test -p bunting-server
cargo test -p bunting-rs
cargo test --locked --workspace
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/bunting-server/src/archive_recorder.rs apps/bunting-server/src/lib.rs apps/bunting-server/src/runtime.rs apps/bunting-server/src/session_host.rs apps/bunting-server/src/scenario.rs apps/bunting-server/src/config.rs apps/bunting-server/tests/path_equivalence.rs
git commit -m "feat: record authoritative competition command order"
```

---

### Task 7: Reconcile competition documentation without rewriting historical golden semantics

**Files:**
- Modify: `docs/adr/0024-discrete-matching-interval-fairness.md`
- Modify: `docs/adr/0025-run-archive-and-replay-verification.md`
- Modify: `docs/specs/competition-policies-v1.md`
- Modify: `docs/specs/bunting-product-contract.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Documents archive schema v2 separately from the existing engine simulation-domain golden v1.

- [ ] **Step 1: Add a documentation/contract guard**

In CI, assert `COMPETITION_ARCHIVE_VERSION` is 2, `tests/goldens/competition-archive.v2.json` exists, and `packages/bunting-engine/tests/simulation_domain.rs` still references `competition-full-run.v1.json`.

- [ ] **Step 2: Update ADR semantics**

State that `arrival_sequence` is venue-global and contiguous, `interval_index` is nondecreasing, archive v2 contains participant and simulation accepted commands, and replay verifies exact canonical events plus final state hash.

- [ ] **Step 3: Clarify the two golden contracts**

ADR 0025 may continue to reference `competition-full-run.v1.json` only as the deterministic engine/simulation golden. Add a separate reference to `competition-archive.v2.json` for the definitive mixed-command archive contract.

- [ ] **Step 4: Run generators and replay**

```bash
python3 tools/generate_protocol.py
git diff --exit-code -- PROTOCOL.md
cargo run --locked -q -p bunting-rs --example competition_archive > /tmp/competition.archive.json
cargo run --locked -q -p bunting-cli -- replay /tmp/competition.archive.json
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add docs/adr/0024-discrete-matching-interval-fairness.md docs/adr/0025-run-archive-and-replay-verification.md docs/specs/competition-policies-v1.md docs/specs/bunting-product-contract.md .github/workflows/ci.yml
git commit -m "docs: define complete competition replay contract"
```
