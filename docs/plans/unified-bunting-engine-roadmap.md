# Unified Bunting engine roadmap

Status: active evidence-gated implementation plan

This roadmap implements ADR 0018, ADR 0019 and the [`RIT-class feature specification`](../specs/rit-class-market-simulation.md). It is derived from the RIT binary audit, official documented RIT capabilities, the current NBC translation ledger, the RITC and QUARCC port audits, the OrderBook-rs 0.10.3 surface, and the current Bunting vertical slice. It replaces any plan that completes `nbc-v1` as a separately selectable production kernel or leaves matching as an independently consumed peer package.

## Hard gates

1. Proprietary RIT payloads remain outside Git and outside every Cargo/build path.
2. A feature cannot be called compatible without an evidence ID, implementation owner and test. Unknown RIT/NBC formulas remain blocked rather than guessed.
3. Every increment preserves one production matcher and one authoritative engine state.
4. No empty module or future package is created; each change lands a compiling vertical slice.
5. Full repository native/Wasm checks pass before each implementation PR completes.
6. A machine-readable parity check eventually rejects any engine-relevant ledger row without an implementation/test disposition.

## Dependency order

```text
unified transition/state contract
  -> complete OrderBook-rs facade
  -> instrument/participant/run state
  -> RIT core validation, ledger and market data
  -> NBC timing/profile migration
  -> scheduler, events and full-state snapshots
  -> liquidity and built-in agents
  -> tenders/news/assets/settlement/scoring
  -> origin/Worker integration
  -> exhaustive parity gate
```

The RIT core precedes NBC migration because the unified state must first represent the broader run, instrument, participant, risk and valuation contract. NBC-specific timing then plugs into that state instead of defining another kernel.

## Reviewable increments

### 1. `feat/unified-bunting-engine-foundation`

Create the first compiling `packages/bunting-engine` slice with engine/config/profile versioning, a bounded multi-listing run aggregate, minimal immutable scenario definitions, authoritative state, command application and one run-level sequence. Move the tested `packages/orderbook` adapter into a private engine module so the central package directly owns OrderBook-rs, migrate production callers, then remove the transitional crate. Compose existing market types/events, ledger and risk packages without absorbing persistence or tRPC. Tests prove all mutations cross one transition API, multi-listing state is deterministic and no internal mutable book reference escapes. Use [`the persisted implementation prompt`](../prompts/implement-unified-bunting-engine-foundation.md).

### 2. `feat/bunting-engine-orderbook-full-capabilities`

Expose and regression-test the complete useful OrderBook-rs 0.10.3 surface through the engine: all audited order types, cancel/replace/mass cancel, STP, fees, book risk, kill/halt, expiry, lifecycle, snapshots, replay helpers, L1/L2, metrics, iterators, analytics and market impact. The canonical command schema expands only with compiling operations and tests.

### 3. `feat/bunting-engine-rit-parity-core`

Implement ledger rows `0001`-`0028`: run clock/status, rich instrument metadata, market/limit lifecycle, cancellation policies, L1/L2/history, participant orders, positions, cash/BP/NLV, P&L, fees/rebates, grouped risk and admission controls. Exact formulas without evidence remain versioned Bunting-native policies and are not labeled RIT-compatible.

### 4. `feat/bunting-engine-nbc-compatibility`

Move NBC strict configuration, step lifecycle, `DONE` synchronization and proven matcher semantics around the unified engine. Convert the current translated `NbcOrderBook` into a development-only differential oracle, then remove it from the production graph once fixtures pass. Preserve ADR 0017 class/resource provenance on every translated behavior.

### 5. `feat/bunting-engine-run-market-data`

Add bounded committed L1/L2/trade/history/private projections, topic subscriptions, reset/gap recovery and RIT RTD data mappings. Implement ledger rows `0047`, `0050`, `0051` and the engine side of `0052`; Windows COM/Excel remains an adapter.

### 6. `feat/bunting-engine-simulated-liquidity`

Add engine-owned liquidity actors for deterministic seeding, path following, replenishment and withdrawal. Each actor has explicit seed/config/version, bounded state, normal command submission and snapshot/replay coverage. Link RIT `0041` and proven NBC liquidity evidence.

### 7. `feat/bunting-engine-market-making`

Implement pure, independently verified volatility, imbalance, inventory and quoting models only when the first vertical slice exists. Built-in market makers remain actors, not matcher code. Start with simple bounded estimators and exact tick/lot conversion; defer GARCH and spectral work until evidence shows value.

### 8. `feat/bunting-engine-virtual-traders`

Port proven NBC populations and add versioned fundamental, noise, momentum, institutional, spiking and market-making agents only where evidence or an explicit Bunting-native model exists. All RNG streams are named, seeded, snapshotted and included in state hashes. RIT trader formulas remain unresolved until evidence exists.

### 9. `feat/bunting-engine-quarcc-lifecycle`

Implement portable participant-side lifecycle, ID mapping, duplicate/out-of-order reports, deferred fills, reconciliation, kill switch, positions and snapshots in focused reusable packages. Integrate it through the public engine client boundary; never give it internal engine mutation authority.

### 10. `feat/bunting-engine-tenders-news-events`

Implement RIT ledger rows `0029`-`0035` and `0057`: scheduled public/private news, tender lifecycle and policies, assets, leases, conversion jobs, compliance/supervisory events and distressed outcomes. Unknown allocation and settlement formulas require new evidence or an explicitly Bunting-native version.

### 11. `feat/bunting-engine-scoring-termination`

Implement versioned scoring inputs, rankings, end conditions, closeout and termination ordering. NBC equations require JAR-linked evidence; RIT scoring remains compatibility-blocked until formula evidence exists. Bunting-native scoring is allowed with an explicit profile and golden vectors.

### 12. `feat/bunting-engine-unified-recovery`

Define one versioned snapshot containing all authoritative and deterministic component state. Add canonical state hashes, event replay, incompatible-version rejection, corruption recovery, and native/Wasm golden equivalence. An OrderBook-rs snapshot remains nested inside this package.

### 13. `feat/bunting-engine-origin-worker-integration`

Migrate command transactions, origin records, cache packages, tRPC procedures and Worker composition to the unified engine. Preserve optimistic origin versions and commit-before-ack/cache/stream. Build output remains one native Rust Worker with direct tRPC dispatch and no REST router.

### 14. `test/bunting-engine-complete-parity`

Generate a checked parity manifest from the feature ledger and reference matrix. Fail when an engine-relevant row lacks an implementation, explicit evidence block, or test. Run OrderBook-rs regression, NBC differential, RIT field/lifecycle, RTD mapping, risk/ledger, agents/liquidity, tenders/news/assets, scoring/termination, recovery/state hash, Worker and native/Wasm suites.

## Evidence work that can proceed in parallel with implementation

- capture official RIT documentation and authorized isolated traces for the unknowns listed in `unresolved-evidence.md`;
- resolve `ritc_mm` license before adapting any source text;
- complete NBC JAR translation ledger entries and differential fixtures;
- establish QUARCC transition behavior from contracts/tests without copying unlicensed implementation;
- audit upstream OrderBook-rs 0.10.3 capability availability at the exact released crate rather than newer source.

New evidence changes compatibility claims and may refine formulas, but it does not reopen the single-engine authority decision.

## Completion contract

Completion means the parity manifest has no unexplained engine-side omissions; every implemented feature has tests; the useful OrderBook-rs surface remains available; NBC, RIT, RITC-built-in-agent and QUARCC-participant behavior all compose around one state; full snapshot/replay is deterministic; and no separately selectable production market kernel remains.
