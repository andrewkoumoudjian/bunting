# ADR 0053: Replayable exchange publication and a lean single-writer runtime

- **Status: Accepted Target (2026-10-12), partly implemented.** This ADR persists the architectural direction; code, tests and the implementation log are required before any Target can be called Current. Current since 2026-10-20: the O(P) auction price sweep (§4, slice 52) and the first P0 step of §2: a failed post-commit public projection is a visible report-sequence gap with rebuilt caches, never a silent skip (slice 53). Journal-rebuildable versioned projections, BNP durable catch-up (§3), incremental projections, instrumentation and everything in §5 remain Target.
- **Date:** 2026-10-12.
- **Evidence baseline:** [`main@8e3ac2f`](https://github.com/andrewkoumoudjian/bunting/commit/8e3ac2f8901f5dab3bec57a934f1bda6b6ddd6c7).
- **Spec-by-spec evidence:** [Jane Street exchange talk comparison](../specs/jane-street-exchange-engine-requirements.md), based on a user-provided, independently unverified automated transcript (2026-10-12, T:L1–153).
- **Refines:** ADR 0018 (one engine), 0028 (one ledger/replay), 0029 (owned price-time book), 0035 (real plus virtual latency), 0036 (market data), 0037 (calendar and auctions), 0040 (BNP) and 0045 (FIX session WAL). **Supersedes none.** ADR 0050 reserves 0051/0052 for desktop work.

## Context

The talk describes a deterministic single matcher with TCP gateways, one ordered output for many downstream applications, retransmission, recovery through state-machine replay, an identical passive matcher, and cautious partitioning when market-wide risk needs atomicity (transcript T:L33–149). This is not a normative implementation standard; physical UDP multicast and one-in-flight gateway transactions are examples that do not automatically suit a latency-modeled Rust exchange simulator.

Bunting already has the right economic foundation: one sequenced writer, deterministic run state, per-listing price-time order books, one account ledger and risk authority, journaled commands/events, checkpoints, direct venue L2/L3 feeds, a consolidated hub tape, FIX session durability, and BNP private cursors. Replacing this kernel wholesale would multiply correctness risk.

The current code nevertheless exposes targeted gaps:

1. [`PublishingOrigin::execute_admitted`](../../apps/bunting-server/src/distributor.rs) converts `public_updates` errors to empty outputs with `unwrap_or_default()` and also defaults private projection failures. It can report a successfully committed economic change without an expected downstream update. This is an **observed failure path**, not proof that it occurred in a deployed run.
2. `public_updates` rereads complete visible depth for touched listings. With L3 enabled it rereads all displayed orders and diffs against process-local caches. This does work proportional to book size rather than changed orders.
3. [`bnp_host.rs`](../../apps/bunting-server/src/bnp_host.rs) can report `ResumeStatus::Gap` when a cursor predates [`EventDistributor`](../../apps/bunting-server/src/distributor.rs)'s 16,384 retained batches; a durable private BNP backfill like the FIX host's journal catch-up is not demonstrated.
4. [`Book::auction_price`](../../packages/bunting-engine/src/book.rs) sums bid/ask levels independently at every candidate price. Its O(P²) price-level complexity occupies the authoritative writer at auction time.
5. [`JournalWriter::append`](../../apps/bunting-server/src/commit_journal.rs) serializes a JSON record, hashes it and calls `sync_data()` for each committed input before publishing. This preserves durability but is a **candidate** throughput cost; no fresh sustained-load benchmark was run for this ADR.
6. There is restart replay but no live passive matcher or fenced promotion protocol. This is a deployment availability tradeoff rather than a current economic-correctness defect.

## Decision

### 1. Preserve the one-writer economic authority

Keep one logical, deterministic writer per run through `bunting-engine`, `origin-store` and the admission sequencer. The engine alone owns orders, matching, fills, risk, the single ledger, calendars, logical time, market rules and scoring. FIX/BNP sessions, agents, operators and timers remain outside the economic authority and submit ordinary commands. Preserve canonical 128-bit IDs, fixed-point arithmetic and bounded resource policies.

Preserve [ADR 0035](0035-latency.md): actual network latency counts, virtual team/venue/hub distance is added in both directions, no equalization or selectable modes. A common committed source order is not simultaneous wall-clock client delivery. Keep FIX and BNP as the only participant command interfaces. No new microservices, UDP multicast, consensus layer or new command transport is implied by this ADR.

### 2. Make every committed output deterministic and recoverable (P0)

Define a **versioned commit-to-publication contract**: for any committed run prefix `k`, the authoritative journal plus a versioned checkpoint and projection rules must produce exactly the same authorized outputs and sequence positions, independent of subscriber count, process restarts or later state changes.

- Canonical economic outcomes — both sides' fills, execution prices, ownership, fees, reservations, ledger postings and reject reasons affecting the market — remain bound to one sequenced, durable transaction. Derived outputs must never imply that only one side of a committed trade exists.
- Every persistent projection (private execution reports, L2 levels, L3 order-by-order references/changes, direct trades, delayed consolidated tape, and optional authorized drop copy) must be rebuildable from **committed records**, including order visibility/anonymity and maker identifiers. Do not depend on reading mutable future `RunState` to reconstruct an earlier market view.
- Projectors may use cached snapshots and book views as accelerators, but caches are not independent market authority. Either record canonical changed-order/changed-level deltas at commit time or implement a deterministic reducer that reconstructs them from the journal; use explicit schema and projection versioning.
- Remove success-shaped `unwrap_or_default()` fallbacks for *required* downstream projections. If an output cannot be derived **after** the input is irreversibly committed, do not roll back the trade: mark the affected stream unavailable, preserve the cursor, rebuild from the durable record, then replay or explicitly reset/resnapshot. Never silently publish a complete-looking feed with missing updates.
- Classify client-visible rejections as authoritative transaction results, durable FIX/BNP session outcomes or ephemeral transport responses. Any response that advances persistent state must have a recoverable source.
- Bounded downstream consumer failure may be isolated from successful matching; it must not leave that consumer believing its stale view is continuous.

Invariant: `Project(checkpoint, committed_inputs_and_effects[<=k], policy_version)` reconstructs the same canonical authorized outputs and source sequence for all subscribers. Any gap is explicitly detectable.

### 3. Complete stream recovery or require an explicit reset (P1)

Keep [ADR 0045](0045-fix-session-write-ahead-log.md)'s FIX session WAL (session sequence, mapped IDs, resend data, unsent reports; write ahead of socket send). Add bounded, authenticated **BNP durable private catch-up** for cursors beyond the in-memory retained window, or specify an explicit versioned reset/snapshot contract if authoritative history is no longer available. Avoid conflating transport delivery, economic exactly-once commitment and application sequence acknowledgement.

Direct venue L2/L3 and hub tape subscriptions must pair a snapshot position with strictly ordered subsequent increments, provide gap detection and a replay/reset path, and continue honoring venue-to-team virtual travel time. Set hard caps on history windows, replay CPU/memory and egress bandwidth; disconnect abusive and slow readers without stalling the economic writer.

### 4. Eliminate avoidable hot-path computation (P1)

- Replace auction candidate recomputation with a deterministic **O(P)** sweep over ordered price levels using cumulative demand and supply. Retain Bunting's exact tie-breaks: maximum matched volume, minimum imbalance, nearest reference, then lower price; preserve hidden liquidity and deterministic uncrossing.
- Off-thread auction calculation is allowed **only if** profiling shows a meaningful stall and the proposal carries a precise book/run version that the writer checks before atomic uncross. The economic effect remains an engine transition.
- Prefer committed incremental order/level changes to full L2/L3 book scans for every touched commit, with a replayable projection that preserves iceberg refresh and visible priority.
- Record per-stage performance: ingress stamp, admission wait, transition, serialization, checksumming, durable sync, projection, fanout, session WAL/wire encoding, actual connection and virtual transmission delays. No unmeasured claims of low-microsecond throughput.

### 5. Treat book layout, journal encoding and replicas as optional improvements (P2)

- Do not replace `Book` merely because the talk describes preallocated order locators. Consider generation-checked arenas/intrusive FIFO queues and pooling **after** cache-miss/allocation benchmarks, and only with price-time oracle, snapshot/replay and stale-handle proof. Clients never provide raw addresses as authority.
- Do not replace BUNTWAL3 or weaken `sync_data()` before a workload benchmark, file-format versioning, recovery/compatibility plan and power-loss tests. Evaluate compact binary frames, preallocated buffers and group commit only while preserving **durable commit before external acknowledgment/publication** under the chosen contract.
- A passive matcher may later consume the primary's **committed ordered prefix**, execute the identical state machine and compare event/state hashes. It may not independently sequence raw socket arrivals. Checkpoints accelerate rebuilding.
- Failover, if required by recovery-time objectives, must define a durable prefix, writer epoch and fencing, unresolved in-flight request handling, client reconnection/resend and operator promotion; no simultaneous authoritative leaders.
- Do not shard by symbol or venue or introduce distributed consensus until observed throughput/availability requirements justify the cross-partition risk and event-order complexity. Preserve atomic portfolio limits across cross-listed instruments.

### 6. Implementation sequence

1. **P0 — correctness:** deterministic publication schema, explicit projection failures, replayable L2/L3/private/tape history, complete rejection-state provenance and failure injection.
2. **P1 — recovery and cost:** BNP deep-gap repair, O(P) auction selection, incremental projections, end-to-end performance/fairness instrumentation.
3. **P2 — evidence-gated HA/performance:** journal and book layout experiments, isolated projector hosting where useful, optional passive follower. No mandatory multicast or symbol partitioning.

The diagram below is **Target and logical**, not deployed topology:

```text
FIX / BNP sessions (authentication, identity, arrival stamp, durable session state)
   -> admission sequencer (one ordered release, network + virtual latency)
   -> one deterministic bunting-engine writer (books, risk, ledger, time)
   -> verified durable commit journal (inputs, outcome, events, chain, sequence)
   -> deterministic versioned projection and recoverable cursors
         -> private fills / optional authorized drop copy
         -> per-venue L2 / L3 / trades
         -> consolidated hub tape
   -> bounded FIX / BNP stream delivery with venue->participant delays

Optional verified passive: committed prefix -> identical engine -> hash compare
Optional failover: fenced new writer epoch + explicit operator promotion
```

## Consequences

**Benefits:** Retains the proven economic simplicity while making downstream continuity auditable; removes silent missing market data, makes stream repair explicit, and reduces unnecessary auction/market-data work. A future passive can reuse the exact existing engine rather than introducing a competing matcher.

**Costs:** A versioned projection contract and cursor protocol, additional deterministic projection state, more recovery and failure-injection tests, and compatibility planning for any public feed, archive or journal change. Moving projection work outside the writer must not reorder source events or introduce future-state leakage.

**Compatibility:** No external protocol or file-format change is implemented by this ADR. Future changes require separate schema/version migration and conformance; preserve supported historical archive readers and FIX session WAL guarantees. The engine must remain host-neutral pure Rust.

## Rejected alternatives

1. **Copy physical multicast/retransmit servers wholesale.** It does not solve the missing-output correctness path by itself, and conflates shared source ordering with Bunting's explicitly different per-venue delivery timing.
2. **Make every symbol/venue a separate matcher now.** It complicates shared account/risk atomicity without an observed throughput ceiling.
3. **Acknowledge before persistence.** Violates existing irrevocable fills and commit-before-publish.
4. **Swallow projector errors or send empty success updates.** Hides missing committed history and creates stale books.
5. **Let a standby race ingress or automatically promote without fencing.** Risks divergent total order, duplicate fills and split brain.
6. **Immediately rewrite the order book with arenas.** Speculative cache gains do not justify losing proven matching and replay invariants.
7. **Require one in-flight transaction / per-topic optimistic locks.** Bunting's single authoritative sequencer and bounded client pipelining already provide a simpler contract.

## Validation

**P0:** Fault-inject before/after origin write and sync, during projection, fanout and socket write. Maker and taker recover the same committed fill once economically. A projector rebuilt from genesis/checkpoint+journal produces exactly matching L2/L3 order identities, priority, depth, trade sequence, private reports, hub tape and checksums. Deliberately inject post-commit projection failures: affected feeds visibly stop, preserve cursors and repair by replay, never silently skip. Test anonymous broker IDs, hidden order quantities and participant-specific privacy. Keep existing FIX crash-recovery and archived-run replay correct.

**P1:** Randomized small-book auction oracle verifies matched quantity, all tie breaks, hidden execution and O(P) algorithm behavior. Force BNP disconnection beyond 16,384 retained batches and venue restart; require durable catch-up or documented versioned reset. Record workloads, CPU, persistent-media type, build profile, clients/venues/books, p50/p99/p99.9 stage latency, throughput and allocation/cache metrics. Compare equivalent virtual paths for connection-order bias without removing intended real/virtual delays.

**P2:** Replay identical committed prefix through passive and compare event and state hashes; crash/promote at all boundaries to prove one leader and no phantom trades. Only adopt new journal/book structures after differential matching and replay parity, crash tests and an independently reproducible performance gain.

Record exact checks, workloads, hashes and implementation commits in `docs/implementation-log/`. Do not label a target “implemented” until there is source and test evidence.

## Operational impact

Nothing new is deployed by writing the ADR. The P0 implementation must expose projection-health state, cursor/lag, recovery and explicit snapshot-reset procedures; a future passive requires observability for hash divergence, lag, fencing and operator promotion. A broken downstream stream should be isolated without corrupting the authoritative economic transaction.

## Security impact

Durable replay endpoints must authenticate and authorize the run, participant, roles and requested ranges; enforce strict bandwidth/memory bounds; and reproduce live public/private visibility, iceberg hidden-quantity masking and broker anonymity. Stale internal order-slot handles and duplicate commands cannot alias other orders. Replica promotion requires single-writer fencing so no two nodes make contradictory trade promises.

## References

- [Spec-by-spec talk comparison](../specs/jane-street-exchange-engine-requirements.md) (user-supplied transcript T:L1–153, 2026-10-12, automated transcription not independently verified).
- [Book](../../packages/bunting-engine/src/book.rs), [engine](../../packages/bunting-engine/src/lib.rs), [origin store](../../packages/origin-store/src/lib.rs), [file journal](../../apps/bunting-server/src/commit_journal.rs), [publisher](../../apps/bunting-server/src/distributor.rs), [sequencer](../../apps/bunting-server/src/admission.rs), [BNP](../../apps/bunting-server/src/bnp_host.rs), pinned at [8e3ac2f](https://github.com/andrewkoumoudjian/bunting/commit/8e3ac2f8901f5dab3bec57a934f1bda6b6ddd6c7).
- [ADR 0018](0018-unified-bunting-engine.md), [0028](0028-proposed-headless-run-authority.md), [0029](0029-bunting-owned-deterministic-order-book.md), [0035](0035-latency.md), [0036](0036-public-market-data-feeds.md), [0037](0037-run-clock-calendar-and-auctions.md), [0040](0040-bunting-native-protocol-v1.md), [0045](0045-fix-session-write-ahead-log.md).
