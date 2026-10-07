# ADR 0028 (PROPOSED): Headless run ownership, economic truth, and verifiable execution

- **Status: Proposed — NOT accepted, NOT an active supersession**
- Date: 2026-10-07
- Evidence baseline: [main@0fdbd130a59b212ac6cae1d3b78000acd94fb9b2](https://github.com/andrewkoumoudjian/bunting/commit/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2)
- Research: [independent code/alternative audit](../research/2026-10-07-independent-core-architecture-audit.md)
- Execution proposal: [revised core-first roadmap](../plans/2026-10-07-evidence-led-core-roadmap.md)
- Existing binding decisions: ADR 0018, 0019, 0022, 0023, 0024, 0025 and 0027 remain operative while this is proposed.

## Context

The October 7 independent review directly verified that:

- The engine owns listing-keyed books but SubmitOrder carries only InstrumentId, so cross-listed orders cannot specify an exchange; market projections are still keyed by instrument.
- The normal trade settlement path changes basic Ledger cash/positions, while the separately journaled PortfolioLedger used for scoring receives other simulation-domain actions without corresponding fill postings.
- RunState transitions clone the complete state and reconstruct/serialize upstream matcher snapshots; file origin commits clone/rewrite the entire persisted state. This is a structure-level overhead observation, not a numerical benchmark.
- The native FIX host returns committed reports to the requester without a demonstrated committed-event distributor for resting makers, and uses per-connection open-order counts.
- CompetitionArchive::replay replays SimulationCommandRequest only, not ordinary matching commands.
- The current writer serializes callers after an interval wait, unlike the shared closed-interval batch described in ADR 0024.
- Wasmer/WASIX is an accepted deployment choice, but no source-backed native-versus-WASIX throughput and crash-durability comparison establishes it as optimal for this product.

The goal is a independently verifiable, multi-venue, multi-day Rust simulator with explicit contracts and no hosting dependency in its economic core. The existing architecture contains useful boundaries; this proposal only changes those for which code exposes a limitation or a specific experiment is defined.

## Decision (proposed, conditional on acceptance)

1. **Retain one first-party, headless authoritative market engine.** Market state includes scenario/version, listings, logical clock, matching queues, participant ownership, risk and economic ledger. The matcher's implementation remains private to the engine. FIX, JSON, sockets, browser, Wasmer/Cloudflare, UI and agent strategy execution are adapters, not market authorities. This preserves the central principle of ADR 0018/0019.

2. **Listing identity is explicit at venue-sensitive boundaries.** Order submission, book mutation, venue halt, raw L2, fills and execution reports address a ListingKey (VenueId, InstrumentId). InstrumentId continues to identify fungible economic holdings. A temporary compatibility adapter may resolve legacy instrument-only submissions only when exactly one listing exists; ambiguity is rejected. Version the command and wire formats; do not silently change existing schema v1 meaning.

3. **One authoritative posting operation governs all economic facts.** Every matched fill atomically updates reserved buying power/inventory, cash, signed positions, fees/counterparty clearing and cost-basis lots or equivalent exact book values; valuation and realized/unrealized P&L are versioned projections. Fines, tenders, OTC, financing, settlement, options delivery and scoring must draw from the same authority once implemented. No second ledger may silently carry a contradictory balance. Exact journal structure and posting schemas are subject to an early implementation spike, but reconciliation and conservation invariants are nonoptional.

4. **Live mutable books are owned by the run, snapshots are recovery tools.** An accepted input yields a single deterministic transition and recoverable staged change set. Persist the input and resulting canonical economic facts atomically before acknowledging or publishing; rollback or enter a recovery-required state on durable commit failure. Use immutable checksummed checkpoints for bounded restart, not per-command matching. Index high-frequency order/maker lookups without breaking canonical serialization order.

5. **Record every cause of authoritative state change.** A complete archive contains normal order/cancel commands, control/agent actions, logical schedule and exogenous news, admission ordering/interval metadata, seeds, engine/compiler/policy/calendar contract versions, event stream, ledger effects, checkpoints and final hash. The verifier rebuilds the same ordinary trading outcomes from genesis and checkpoints, verifies full run and participant scores and rejects missing/changed inputs. Do not call the current control-only archive a full trading replay.

6. **Commit once and dispatch many.** A transport-independent committed-event feed fans out public listing-specific market data and private participant-specific order/fill/position changes. Slow-consumer bounded buffers use explicit gap/catch-up. FIX session recovery is separate from engine sequence; disconnect cannot become implicit cancellation.

7. **Fairness is a versioned policy, not a sleep primitive.** Admission sequence/time and the policy for group release, ordering and clearing must be recorded. Define what the competition promises: continuous price/time FIFO on recorded arrival, delayed interval-based FIFO, seeded permutation, or a batch auction. If interval-based, enforce an actual shared sealed interval queue. No policy can make physical remote latency identical; operator communications must be precise about the guarantee. Any departure from ADR 0024's particular shared interval mechanism requires its explicit supersession.

8. **Mutable economic authority is initially single-owner per independent RunId.** Independent runs may execute concurrently. Finer venue sharding, lock-free book ownership and specialized ring buffers are optional experiments requiring evidence of a throughput bottleneck; no blanket adoption. Agent RNG, wake ordering and calendar clock must be included in atomic recovery.

9. **The host remains an interchangeable adapter in the target.** Native Linux Rust provides a comparison baseline; WASIX remains the accepted production choice until a separate runtime decision is adopted after measurement. Cloudflare must not own exchange state. This item does not authorize violating ADR 0027.

10. **Matching provider is explicitly an experiment, not a religion.** Keep released OrderBook-rs 0.10.3 under ADR 0018/0019 for initial economic fixes. Differential conformance versus viable Rust deterministic matchers, upstream patches or a minimal owned matching implementation must precede any proposed replacement. An actual provider change needs a new explicit superseding ADR with unit/latency/priority/licensing data; this ADR alone does not approve a second production CLOB.

## Consequences

**Positive:** venue-specific behavior becomes representable; every fill leads to one financial truth; cold recovery can verify orders, money and final ranking; live operations no longer require full state cloning/snapshot deserialization on each input; server and UI features can consume the same engine. Interface boundaries become simpler to understand and test.

**Costs/migration:** schema and client compatibility changes; a one-time migration/reconstruction of old financial projections may be required; in-memory mutation demands correct failed-commit recovery; durable record/version design and calendar/agent checkpoint require explicit serialization contracts; independent replay will reject incompletely archived historical runs. These costs must not be hidden behind optimistic migration claims.

**Unresolved:** exact posting schema and cost-basis policy, whether the existing upstream matcher can supply a zero-copy/cloneable undo-friendly transaction path, durable backend choice, fairness policy, runtime host and empirically realistic agent calibration. The linked roadmap defines the experiments.

## Rejected or postponed alternatives

- **Keep dual unrelated ledgers:** fails trade-to-score reconciliation.
- **Route cross-listings implicitly by instrument:** impossible to select exchange unambiguously.
- **Keep file-per-command full JSON state rewrite as permanent architecture:** substantial predictable write amplification; it can stay a small reference implementation while alternatives are benchmarked.
- **Adopt a custom CLOB immediately:** fixes none of the upstream-neutral economic or persistence defects and requires recreating sophisticated matching semantics without evidence of benefit.
- **Shard each venue immediately:** complicates atomic portfolio reservations and cross-venue execution; justify only after a measured bottleneck.
- **Make every transport/session an authority:** fragments market sequence and makes passive-maker delivery and reconnect inconsistent.
- **Port all of ABIDES or a full financial platform:** adds complexity before calibrating a minimal credible model.
- **Require native-only or WASIX-only before measurement:** assumes outcome of host experiment; current accepted WASIX deployment remains unchanged for now.
- **Treat interval sleeping as equal-fairness execution:** does not deliver a shared batch or erase network arrival advantage.
- **Testing-only phase before known missing features:** avoids the actual economic contract repair; instead attach focused tests and benchmarks to each implementation slice.

## Validation required for acceptance and implementation

1. A two-venue same-instrument fixture submits explicitly to either book and produces separate listing events/depth with a reconciled consolidated economic portfolio.
2. Every normal fill and fee, adjustment, tender and score (where enabled) is posted once, conserves exact amounts and deterministically recomputes NLV. Rejected/duplicate commands cannot change balances.
3. Live mutable state with failed commit, process kill, abrupt reboot simulation and checkpoint restore never acknowledges an irrecoverable transition and never double-applies idempotent retries.
4. Both maker and taker recover their own exact execution reports regardless of socket connection identity; unauthorized clients never receive private reports.
5. Complete archive replays ordinary and simulation commands and verifies final economic and book state from cold start and checkpoint.
6. Concurrent admissions, late arrivals, queue overflow, reconnect and intervals obey documented ordering without hidden wall-clock dependencies in matching.
7. Five trading days preserve orders and accounts and deterministically apply the published schedule: opens/closes, expiration, carry, marks, settlements and fees.
8. Native and WASIX produce equivalent hashes for identical frozen inputs before a hosting selection; measure CPU, latency, memory, snapshot/restart, syscalls and operational constraints.
9. Upstream matching semantic and benchmark comparison records trace correctness, priority policy and license before changing dependency. Source-based results do not constitute a throughput gate.

## Operational impact

Publish a complete run manifest with versioned policies, calendar, instrument/listing registry, match provider, admission order algorithm, fees and scoring, seed schedule and resource limits. Persist sequence/cursors; expose high-water, queue latency, commit latency, failed sync, checkpoint age, event gap, and recovery progress. An origin in a failed or uncertain commit state is not allowed to continue serving as if healthy.

Do not require a particular frontend, hosting provider or worker deployment to verify a run locally. Cloudflare publication artifacts are read-only derivatives. Retire redundant stores only after migration and replay parity proof.

## Security impact

The engine only accepts previously authorized participant identity as command input; gateway authentication binds actor to immutable run and allowed account. Client-supplied venue cannot grant privileges on a restricted market. Private execution events are filtered before fan-out; durable origin and checkpoints contain access-controlled information. Ingress, per-participant open orders, event publication and replay buffers are bounded. Archived results are checksum-verified and may be signed by organizers; signatures do not replace hash-complete replay.

## Relationship to accepted ADRs

- **0018/0019:** preserve one engine owner and private OrderBook-rs integration for now; **future** provider replacement would require a dedicated explicit supersession of the pinned matcher decision.
- **0022/0023:** retain separation of native venue and Cloudflare publication, and the requirement for concurrent isolated FIX participants; only revise implementation seams if acceptance demands.
- **0024:** propose clarification/replacement of its discrete batching mechanics if the measured implementation or competition fairness model differs; until then the accepted interval-batch policy is binding.
- **0025:** propose expanding its archived execution proof from simulation-only controls to *every* economic action.
- **0027:** **no immediate supersession**. Perform native/WASIX parity and operational benchmark and author a separate host-selection ADR if evidence favors a change.
- **0009/0010/0014:** preserve exact arithmetic, deterministic simulation and participant-versus-market authority.

Acceptance of this ADR would require review of precise contract/migration shapes and reconcile each affected binding ADR. Until accepted, this is a design recommendation and does not override AGENTS.md or production source.

## References

- [Source audit and external comparisons](../research/2026-10-07-independent-core-architecture-audit.md)
- [Implementation and evidence gates](../plans/2026-10-07-evidence-led-core-roadmap.md)
- [Existing ADR 0018](0018-unified-bunting-engine.md), [0019](0019-bunting-engine-package-owns-orderbook-rs.md), [0024](0024-discrete-matching-interval-fairness.md), [0025](0025-run-archive-and-replay-verification.md), [0027](0027-wasmer-wasi-server-runtime.md)
- [Market domain](https://github.com/andrewkoumoudjian/bunting/blob/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2/packages/market-events/src/lib.rs#L103-L119), [engine transition](https://github.com/andrewkoumoudjian/bunting/blob/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2/packages/bunting-engine/src/lib.rs#L784-L951), [economic ledgers](https://github.com/andrewkoumoudjian/bunting/blob/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2/packages/ledger/src/lib.rs), [origin](https://github.com/andrewkoumoudjian/bunting/blob/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2/apps/bunting-server/src/storage.rs), [FIX dispatch](https://github.com/andrewkoumoudjian/bunting/blob/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2/apps/bunting-server/src/session_host.rs), [archive](https://github.com/andrewkoumoudjian/bunting/blob/0fdbd130a59b212ac6cae1d3b78000acd94fb9b2/bunting-rs/src/archive.rs)
