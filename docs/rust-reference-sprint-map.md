# Rust reference map for planned sprints

This document records the second Rust-only reference search for Bunting. It is a normative addendum to `docs/reference-adoption.md` for the repositories introduced by this pass.

A repository under `ref/` remains read-only evidence. Its presence does not approve a Cargo dependency, source copy, vendoring decision or runtime architecture.

## Selection method

Candidates were evaluated against the planned implementation PRs and retained only when they had:

- an explicit permissive license;
- maintained, reusable Rust code rather than a tutorial-sized exchange clone;
- tests, fuzzing, documented invariants or a stable format specification;
- a narrow relationship to an identified Bunting implementation gap;
- a boundary that permits borrowing behavior without importing an unsuitable runtime.

The search deliberately excluded repositories that merely claimed high throughput, lacked a license, duplicated stronger existing references or centered their design on unordered concurrency.

## Added commit-pinned references

| Local path | Upstream pin | License | Primary sprint gap | Initial disposition |
|---|---|---|---|---|
| `ref/barter-rs` | `barter-rs/barter-rs@33e56188e2095781331f85aa3d7f88e251eec65a` | MIT | OMS lifecycle, risk approval/refusal, indexed state, audit replica and adapter boundaries | architecture and test oracle; no whole-engine dependency |
| `ref/slotmap` | `orlp/slotmap@0d130ed5bbd6e51fbb64a6b6cd80d3adfbb04294` | Zlib | stable safe handles for the resting-order arena | dependency spike for `orderbook`; no vendoring planned |
| `ref/intrusive-rs` | `Amanieu/intrusive-rs@e7b27a0ea9a23084a62c5896161b8805db72e5b9` | MIT OR Apache-2.0 | intrusive FIFO/list invariants and cursor semantics | concept/source reference; not the first implementation dependency |
| `ref/rand` | `rust-random/rand@8272d49f03b4900b648df39b24d9a0343cbd45b5` | MIT OR Apache-2.0 | seeded, versioned scenario random streams | dependency candidate with explicit algorithm pinning and golden vectors |
| `ref/postcard` | `jamesmunns/postcard@de182557cff45f2ca9b2b67a6b93be5917612a44` | MIT OR Apache-2.0 | compact stable snapshot/replay payload encoding | dependency spike for snapshots; not automatically the canonical hash encoding |
| `ref/proptest` | `proptest-rs/proptest@85a3de393331b951e20c5da1cdac9d342fc36a6f` | MIT OR Apache-2.0 | generated command sequences, shrinking and state-machine tests | approved direction for dev/test dependencies after toolchain check |

## Sprint mapping

### PR 2 — deterministic market kernel

Relevant new references:

- `slotmap/src/basic.rs`
- `slotmap/src/dense.rs`
- `slotmap/examples/doubly_linked_list.rs`
- `slotmap/fuzz/fuzz_targets/target.rs`
- `intrusive-rs/src/linked_list.rs`
- `intrusive-rs/src/adapter.rs`
- `intrusive-rs/src/rbtree.rs`
- `barter-rs/barter/src/risk/mod.rs`
- `barter-rs/barter/src/risk/check/`

#### Resting-order storage decision

Bunting still begins with:

```text
BTreeMap<PriceTicks, PriceLevel>
OrderId -> OrderLocation
safe handle-backed FIFO per price level
```

`slotmap` is the preferred dependency experiment for the backing arena because it provides stable generational keys, invalidates stale keys after deletion and offers O(1) insertion/access/removal. The experiment must prove:

- deterministic traversal does not depend on SlotMap iteration order;
- canonical order and snapshot bytes are emitted through explicitly sorted Bunting projections;
- key generations survive snapshot/restore correctly when the optional serde support is enabled;
- no key value is exposed as a stable external order ID;
- Wasm size and allocation behavior are acceptable.

`intrusive-collections` is a design reference for list invariants, head/tail operations, safe cursor mutation and allocation-free linking. Bunting should not begin with its pointer-backed intrusive list because the current design calls for serializable integer handles and no pointer identity in snapshots. Its tests and invariants should inform a Bunting-owned handle-based doubly linked FIFO.

#### Risk boundary

Barter's `RiskManager` and `RiskApproved`/`RiskRefused` separation is useful evidence for making approval explicit in the type flow. Bunting must not copy Barter's full trading engine or accept its default approve-all implementation. The canonical kernel requires stable rejection codes, exact fixed-point limits, reservations and event-sourced kill-switch state.

### PR 3 — replay, snapshots and differential fixtures

Relevant new references:

- `postcard/spec/`
- `postcard/src/ser/`
- `postcard/src/de/`
- `proptest/proptest/src/strategy/`
- `proptest/proptest/src/collection.rs`
- `proptest/proptest-state-machine/`
- `slotmap` serialization/fuzz tests

Postcard has a documented stable wire format from version 1.0 and is designed for constrained/no-std environments. It is a reasonable snapshot payload experiment, subject to these rules:

- snapshot schema versions remain owned by Bunting;
- maps are normalized to deterministic sorted sequences before serialization;
- skipped or flattened serde fields are prohibited in canonical snapshot structs;
- decoding is bounded and rejects trailing or incompatible data;
- golden bytes are pinned for every snapshot schema;
- state hashes continue to use a separately specified canonical preimage unless Postcard is formally adopted by ADR for that purpose.

Proptest should generate and shrink command sequences for:

- add, cross, partial fill, cancel and IOC paths;
- duplicate IDs and invalid references;
- risk rejections with no state mutation;
- ledger conservation;
- live versus replay state equality;
- snapshot plus tail equality;
- reconciliation transition sequences.

Failure-persistence files and every minimized regression must be committed. Test RNG seeds must be printed and reproducible.

### PR 4 — MarketRun Durable Object

The new references do not replace `workers-rs` or CQRS. Postcard may be evaluated for snapshot blobs, while Proptest can exercise transaction/idempotency models in pure host tests.

Barter's audit-stream/replica examples provide comparison material for non-authoritative projections:

- `barter/examples/engine_sync_with_audit_replica_engine_state.rs`
- `barter/tests/test_engine_process_engine_event_with_audit.rs`

Bunting differs by committing the canonical event batch before fan-out. An audit consumer cannot alter authoritative state.

### PR 5 — scenario schema, scheduler and random streams

Relevant new references:

- `rand/src/rngs/mod.rs`
- `rand/src/rngs/std.rs`
- Rand seedable RNG and distribution implementations
- Proptest collection/strategy generation for scheduler tests

Bunting must never use `thread_rng`, `rng()` or ambient system randomness in canonical scenario execution. It must not use `StdRng` as an unnamed long-term replay contract because implementation details may change.

The intended experiment is an explicitly named ChaCha generator with:

- algorithm name and round count in the scenario schema;
- crate major/minor version recorded in the model implementation version;
- a Bunting-owned domain-separated seed derivation from master seed, scenario version, agent ID and stream name;
- one independent stream per purpose;
- serialized stream position/state in snapshots or deterministic reconstruction from counters;
- golden vectors for initial outputs and post-restore outputs;
- rejection of feature changes that alter sampling behavior without a scenario/model version change.

Rand's own feature documentation notes that sampling options can affect reproducibility; Bunting therefore pins exact features and treats changes as model-version changes.

### PR 6 — NBC compatibility

The new references are supporting utilities only:

- Rand supplies explicit deterministic streams for reconstructed agents.
- Proptest generates malformed legacy payloads and barrier sequences.
- Postcard must not become a legacy wire format; NBC compatibility remains JSON/WebSocket translation.

### PR 7 — FIX vertical slice

No new FIX engine was added because IronFix, Fixer, FerrumFIX and QuickFIX/J already cover the space. Proptest adds value for bounded malformed-field sequences and session state-machine transitions.

Postcard is not used for FIX wire encoding. FIX session persistence may use a versioned internal snapshot format only after conformance state is expressed in Bunting-owned structs.

### PR 8 — native clients, RITC and Nautilus adapters

Relevant new Barter paths include:

- `barter/src/engine/mod.rs`
- `barter/src/engine/state/`
- `barter/src/engine/action/generate_algo_orders.rs`
- `barter/src/risk/mod.rs`
- `barter-execution/`
- `barter-integration/`
- audit and risk-manager examples under `barter/examples/`

Borrowable principles:

- typed separation of strategy, risk and execution requests;
- centralized indexed state rather than scattered strategy-owned truth;
- explicit trading enabled/disabled state;
- an audit stream that can rebuild a read-only replica;
- mock/live execution boundaries with comparable normalized reports.

Rejected Barter architecture for Bunting's authoritative path:

- multithreaded engine orchestration;
- Tokio tasks and channels inside the pure kernel;
- exchange connector implementations as kernel dependencies;
- any implicit wall-clock or runtime scheduling authority;
- whole-engine adoption.

Barter may become a native client-side interoperability/reference harness. It does not replace Bunting's order-reconciliation crate, ledger, risk engine or Durable Object sequencer.

## Per-reference dependency and borrowing decisions

### Barter

- **Direct production dependency:** rejected for the market kernel and Worker.
- **Native dev/reference use:** allowed after dependency isolation.
- **Code borrowing:** narrowly scoped MIT-licensed traits/tests only, with path and commit attribution.
- **Primary value:** order/risk type flow, audit replicas, indexed state and connector boundaries.

### SlotMap

- **Direct dependency:** approved for an isolated order-arena spike, not yet production approval.
- **Vendoring:** rejected; use the released crate if adopted.
- **Primary risk:** accidentally treating key/iteration representation as canonical external state.

### Intrusive collections

- **Direct dependency:** deferred for the first kernel slice.
- **Code borrowing:** normally unnecessary; study invariants and tests.
- **Primary risk:** pointer ownership and serialization complexity conflicting with deterministic snapshot requirements.

### Rand

- **Direct dependency:** expected for scenario/test code after an explicit RNG ADR and golden-vector test.
- **Default/ambient RNG:** prohibited in canonical execution.
- **Vendoring:** rejected.
- **Primary risk:** reproducibility changes caused by algorithm, crate, distribution or feature upgrades.

### Postcard

- **Direct dependency:** snapshot/replay-format spike only.
- **Canonical event/hash encoding:** not approved by this document.
- **Vendoring:** rejected.
- **Primary risk:** serde schema evolution or unordered input structures changing bytes.

### Proptest

- **Direct dependency:** dev/test only.
- **Production dependency:** prohibited.
- **Primary use:** generated command/state-machine sequences, shrinking and persistent regressions.
- **Primary risk:** non-reproducible failures when seeds/configuration are not captured.

## Candidate rejected during this search

`cmd05/lob-engine-rs` had a highly relevant design on inspection: integer ticks, deterministic FIFO matching, replay validation, latency scheduling and bounded analytics. It was not added because no repository `LICENSE` file was present at the audited revision. Bunting must not copy or vendor it unless a clear license is added and the exact revision is re-audited.

Several smaller Rust matching-engine repositories were also rejected because they were tutorial-sized, lacked meaningful tests or documentation, or did not improve on the already pinned OrderBook-rs/Liquibook/exchange-core oracle set.

## Required follow-up evidence

Before any of the new references becomes a dependency:

1. pin a released crate version and exact features;
2. run native and `wasm32-unknown-unknown` builds where applicable;
3. record transitive dependency and binary-size changes;
4. add deterministic golden vectors;
5. add property/regression tests covering the proposed integration;
6. document upgrade and compatibility policy in an ADR or port note;
7. keep production Cargo manifests independent of paths under `ref/`.
