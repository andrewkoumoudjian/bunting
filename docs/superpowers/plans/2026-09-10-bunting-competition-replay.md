# Bunting Competition Replay Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make competition ordering and archives represent and deterministically replay every accepted participant and simulation command in one venue-global admission order.

**Architecture:** Introduce a protocol-neutral accepted-command enum in `market-events`, expose venue-global admission metadata from `AuthoritativeWriter`, and record that metadata alongside accepted commands in archive v2. Replay uses one unified transition helper that dispatches participant and simulation commands through the same engine preparation paths used by live transactions, verifies ordering metadata, canonical events, and final state hash.

**Tech Stack:** Rust 1.88, serde, Bunting engine/market-events/command-transaction/origin-store/application/server, JSON competition archives.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- One native venue owns the authoritative commit and ordering sequence.
- The matching interval policy is versioned and archive-visible.
- Participant orders/cancels and simulation/operator commands are both present in the accepted-command archive.
- Replay cannot rely on wall-clock time or network timing.
- Reordered, duplicated-with-conflict, missing, or event-divergent archives fail verification.
- Existing archive v1 JSON is either explicitly rejected as unsupported or migrated by a deterministic compatibility decoder; it must never be silently treated as complete competition evidence.

---

### Task 1: Add a unified protocol-neutral command enum

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

- [ ] **Step 1: Write a failing serialization test**

Add a participant command fixture and assert its JSON discriminant and round trip:

```rust
#[test]
fn accepted_command_round_trips_participant_and_simulation_variants() {
    let participant = AcceptedCommand::Participant(sample_submit_command());
    let simulation = AcceptedCommand::Simulation(sample_start_request());
    for command in [participant, simulation] {
        let json = serde_json::to_string(&command).expect("serialize");
        let decoded: AcceptedCommand = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(decoded, command);
    }
}
```

Use existing test fixture constructors in this file where available; otherwise add exact constructors using run 1, command IDs 10/11, actor 1, logical time 0, and event sequence 0.

- [ ] **Step 2: Run the focused test and verify failure**

```bash
cargo test -p bunting-market-events accepted_command_round_trips_participant_and_simulation_variants
```

Expected: FAIL because `AcceptedCommand` is undefined.

- [ ] **Step 3: Implement the enum and accessors**

Implement every accessor as a match delegating to the wrapped command. Do not duplicate any command fields.

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

### Task 2: Unify transaction preparation for replay without changing live idempotency

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

The function dispatches to existing `prepare_command` or `prepare_simulation_command`. It performs no origin I/O and therefore can be used by deterministic archive replay.

- [ ] **Step 1: Write failing equivalence tests**

For a submit-order command:

```rust
let direct = prepare_command(&command, &state, None).expect("direct");
let unified = prepare_accepted_command(
    &AcceptedCommand::Participant(command.clone()),
    &state,
    None,
).expect("unified");
assert_eq!(unified.commit.events, direct.commit.events);
assert_eq!(unified.commit.candidate.state_hash().unwrap(), direct.commit.candidate.state_hash().unwrap());
```

Repeat for a `SimulationCommand::StartRun` request against a valid initial scenario.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-command-transaction prepare_accepted_command
```

Expected: FAIL because the unified helper is missing.

- [ ] **Step 3: Implement the dispatcher**

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

For participant commands, resolve any required listing snapshot exactly as the live transaction does. If callers cannot supply the correct cache entry generically, add a pure `accepted_command_listing_key()` helper and construct the cached lookup at the caller rather than skipping it.

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

### Task 3: Make venue admission metadata explicit and global

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

`arrival_sequence` is monotonic for the shared `AuthoritativeWriter`; `interval_index` is derived from the writer's monotonic start and configured interval before waiting for the commit turn.

- [ ] **Step 1: Replace the current concurrency test with metadata assertions that fail first**

Use four threads and collect `(arrival_sequence, interval_index, payload)` from the closure:

```rust
writer.execute_interval(|admission| {
    committed.lock().unwrap().push((admission.arrival_sequence, admission.interval_index, value));
    Ok(())
})
```

Assert arrival sequences are exactly `0,1,2,3` in commit order. Add a second test with a long interval proving two near-simultaneous admissions receive the same `interval_index` while preserving arrival order.

- [ ] **Step 2: Run writer tests and observe compile/test failure**

```bash
cargo test -p bunting-server writer::tests
```

Expected: FAIL because closures currently take no admission argument.

- [ ] **Step 3: Implement admission metadata**

At queue admission, assign `arrival_sequence = next_arrival.fetch_add(1, ...)`. Compute:

```rust
let interval_nanos = self.interval.as_nanos().max(1);
let interval_index = u64::try_from(self.started.elapsed().as_nanos() / interval_nanos)
    .unwrap_or(u64::MAX);
```

Pass `VenueAdmission { interval_index, arrival_sequence }` to the closure after the turn/gate is acquired. Do not use `SystemTime`.

- [ ] **Step 4: Update every writer call site**

`session_host.rs` and `scenario.rs` must accept the closure argument even where it is temporarily unused:

```rust
writer.execute_interval(|admission| { ... })
```

The participant path carries `admission` into the archive/journal hook added in Task 5.

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

### Task 4: Introduce competition archive v2 with full accepted-command records

**Files:**
- Modify: `bunting-rs/Cargo.toml`
- Modify: `bunting-rs/src/archive.rs`
- Modify: `bunting-rs/examples/competition_archive.rs`
- Modify: `tests/goldens/competition-full-run.v1.json`
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

- [ ] **Step 1: Write a failing archive fixture containing both command kinds**

Build an initial scenario, issue `StartRun`, then a participant resting limit order at the resulting expected sequence. Construct records with arrival sequences `0` and `1`, same interval if desired, and role strings `administrator` and `participant`.

Assert after JSON round trip:

```rust
assert!(matches!(decoded.accepted_commands[0].command, AcceptedCommand::Simulation(_)));
assert!(matches!(decoded.accepted_commands[1].command, AcceptedCommand::Participant(_)));
```

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-rs archive_round_trip_replays_participant_and_simulation_commands
```

Expected: FAIL because archive v1 accepts only simulation requests.

- [ ] **Step 3: Implement v2 schema and strict ordering validation**

`validate()` requires:

```rust
for pair in self.accepted_commands.windows(2) {
    if pair[0].arrival_sequence >= pair[1].arrival_sequence {
        return Err(ArchiveError::InvalidAdmissionOrder);
    }
    if pair[0].interval_index > pair[1].interval_index {
        return Err(ArchiveError::InvalidAdmissionOrder);
    }
}
```

Require the first arrival sequence to be 0 for a full-run archive. If partial archives are later needed, introduce a separately versioned range envelope rather than weakening this rule.

Add exact error variants `InvalidAdmissionOrder` and `UnsupportedVersion`.

- [ ] **Step 4: Make v1 handling explicit**

`from_json()` must inspect `schema_version`. For version 1 return `ArchiveError::UnsupportedVersion`; do not reinterpret the simulation-only stream as complete evidence. Update golden/test fixture names to `.v2.json` where they represent definitive competition archives and update repository references atomically.

- [ ] **Step 5: Update the example to include a participant order**

The example must generate a v2 archive with at least one simulation and one participant command and compute canonical events/final hash from those commands, not by hand-editing JSON.

- [ ] **Step 6: Run archive and CLI replay checks**

```bash
cargo test -p bunting-rs
cargo run --locked -q -p bunting-rs --example competition_archive > /tmp/competition.archive.json
cargo run --locked -q -p bunting-cli -- replay /tmp/competition.archive.json
cargo run --locked -q -p bunting-cli -- score /tmp/competition.archive.json
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add bunting-rs/Cargo.toml bunting-rs/src/archive.rs bunting-rs/examples/competition_archive.rs tests/goldens
git commit -m "feat: archive complete competition command stream"
```

---

### Task 5: Replay v2 through the same pure transition preparation as live commands

**Files:**
- Modify: `bunting-rs/src/archive.rs`
- Test: `bunting-rs/src/archive.rs`

**Interfaces:**
- Consumes: `prepare_accepted_command()` from Task 2.
- Produces: `CompetitionArchive::replay()` supporting participant and simulation commands and exact canonical event verification.

- [ ] **Step 1: Add failing negative replay tests**

Add four tests:

```rust
replay_rejects_reordered_arrivals()
replay_rejects_participant_command_event_drift()
replay_rejects_sequence_gap()
replay_rejects_final_hash_drift()
```

For the sequence-gap case, mutate the second command's `expected_sequence` to a non-current sequence and require `ArchiveError::CommandRejected(1)`.

- [ ] **Step 2: Run and observe failure**

```bash
cargo test -p bunting-rs replay_rejects_
```

Expected: at least participant replay tests FAIL under the old simulation-only implementation.

- [ ] **Step 3: Implement unified replay**

For each archived command:

```rust
let prepared = bunting_command_transaction::prepare_accepted_command(
    &record.command,
    &state,
    cached.as_ref(),
).map_err(|_| ArchiveError::CommandRejected(index))?;
events.extend(prepared.commit.events.clone());
state = prepared.commit.candidate;
```

For participant commands requiring a listing cache, derive the same snapshot input as live `CommandTransaction::execute_detailed`; use no cache when replay state itself is canonical unless the engine contract requires a matching cached snapshot. Never synthesize events separately.

- [ ] **Step 4: Verify exact events and final hash**

Keep strict vector equality for canonical events, then compare `state_hash()`. Include accepted command count and event count in `ReplayResult` as today.

- [ ] **Step 5: Run package and CLI tests**

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

### Task 6: Record live accepted commands with venue admission metadata

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
- Produces a narrow server-owned recording boundary:

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

Provide `MemoryAcceptedCommandRecorder` for tests and a bounded file-backed recorder only when archive output is enabled in server config. Recording happens after a successful authoritative commit and uses the command/events/state returned by that commit.

- [ ] **Step 1: Write a failing memory-recorder test**

Record one simulation command and one participant command with admissions 0 and 1. Assert snapshot order and stored event sequences exactly match the arguments.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p bunting-server archive_recorder
```

Expected: FAIL because module/trait is absent.

- [ ] **Step 3: Implement the in-memory recorder first**

Use `Mutex<Vec<ArchivedAcceptedCommand>>` plus event/state metadata required to finalize an archive. Poisoned mutex returns a stable string error; never panic.

- [ ] **Step 4: Wire participant commits**

Inside `writer.execute_interval(|admission| ...)`, after `service.execute()` returns a non-duplicate accepted commit, call recorder with `AcceptedCommand::Participant(command.clone())`, role derived from `VerifiedActor`, returned events, and returned state.

Rejected commands are not added to `accepted_commands`; if rejection audit evidence is required, keep it in a separate rejection journal rather than naming it accepted.

- [ ] **Step 5: Wire simulation/operator commits**

Pass the same recorder into scenario runtime and competition mutation paths. After accepted non-duplicate `execute_simulation`, record `AcceptedCommand::Simulation(request.clone())` with the same writer admission metadata.

- [ ] **Step 6: Add one concurrent two-participant path-equivalence test**

Drive two authenticated participant sessions into the same shared writer, capture recorder output, build an archive from initial state + captured stream/events/final state, and call `replay()`. Assert final hash and canonical event vector are identical.

- [ ] **Step 7: Run server + replay tests**

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

### Task 7: Update competition docs and golden contract to v2

**Files:**
- Modify: `docs/adr/0024-discrete-matching-interval-fairness.md`
- Modify: `docs/adr/0025-run-archive-and-replay-verification.md`
- Modify: `docs/specs/competition-policies-v1.md`
- Modify: `docs/specs/bunting-product-contract.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Documents the actual v2 archive semantics and global admission order implemented above.

- [ ] **Step 1: Add a documentation contract check to CI**

Add shell assertions that `COMPETITION_ARCHIVE_VERSION` is 2 and the definitive golden filename/reference is v2. This prevents docs/test drift.

- [ ] **Step 2: Update ADR text with exact invariants**

State that `arrival_sequence` is venue-global and strictly increasing, `interval_index` is nondecreasing, accepted archives include participant and simulation commands, and replay validates canonical event vector plus final state hash.

- [ ] **Step 3: Run protocol/docs generation and replay budget**

```bash
python3 tools/generate_protocol.py
git diff --exit-code -- PROTOCOL.md
cargo run --locked -q -p bunting-rs --example competition_archive > /tmp/competition.archive.json
cargo run --locked -q -p bunting-cli -- replay /tmp/competition.archive.json
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0024-discrete-matching-interval-fairness.md docs/adr/0025-run-archive-and-replay-verification.md docs/specs/competition-policies-v1.md docs/specs/bunting-product-contract.md .github/workflows/ci.yml
git commit -m "docs: define complete competition replay contract"
```
