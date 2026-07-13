# RIT-class market-simulation feature specification

Status: planned Bunting requirements; this document does not claim implementation or exact RIT internal equivalence

## Purpose and evidence boundary

Bunting is building a RIT-class educational market-simulation platform on its existing deterministic, event-sourced and Cloudflare-native foundation. This specification reconciles three evidence sources:

1. **Officially documented:** the Rotman RIT overview describes instructor casefiles, periods/iterations, centralized limit-order books, API/RTD access, AI order flow, institutional workflows, multi-marketplace cases, targeted news, product families, monitoring and reports.
2. **Binary-observed:** the static MSI audit records exact packaged types, fields, routes, RTD topics and protocol surfaces under [`../research/rit-binary-audit/`](../research/rit-binary-audit/).
3. **Bunting-added:** state models, package boundaries, deterministic ordering, tRPC procedures, event schemas and implementation policies are independent Bunting decisions.

Rotman publishes external behavior and says server casefiles are open and customizable for RIT instructors. That does not establish source access or redistribution rights for the server application or casefile corpus. Bunting therefore implements independently from documented behavior and authorized observations; unresolved formulas remain unresolved.

Official capability source: [Rotman Interactive Trader overview and features](https://www.rotman.utoronto.ca/faculty-and-research/education-labs/bmo-financial-group-finance-research-and-trading-lab/rit-market-simulator/overview--features/), verified 2026-07-13.

## Binding architecture

`packages/bunting-engine` is the central market-simulation package. It directly integrates released `orderbook-rs = 0.10.3` as its private production matcher and owns the authoritative run transition. The adapter and tests have moved into the engine package and the transitional `packages/orderbook` crate is removed.

The production dependency direction is:

```text
market types/events, ledger rules, risk rules
                  |
                  v
        packages/bunting-engine
        - private OrderBook-rs adapter
        - run and logical clock
        - listings and participants
        - scenarios and scheduled actions
        - agents, news, tenders and assets
        - settlement, scoring and full snapshot
                  |
       +----------+-----------+
       v                      v
origin/cache/transaction   bunting-rs composition
       |                      |
       +----------+-----------+
                  v
       native Rust tRPC Worker

FIX initiator -> client-side FIX bridge -> tRPC
RIT REST/VBA/RTD -> external compatibility adapters -> tRPC
```

There is no second matching engine, server-side FIX endpoint, REST router, Axum service, or transport-owned market state. A Rust stream-coordination Durable Object remains conditional under ADR 0016 and never owns commands or matching.

## Authoritative run aggregate

One run owns many independently traded listings and one run-level sequence:

```rust
pub struct RunState {
    pub metadata: RunMetadata,
    pub clock: ScenarioClock,
    pub listings: BTreeMap<ListingKey, ListingState>,
    pub participants: BTreeMap<ParticipantId, ParticipantState>,
    pub contracts: BTreeMap<ContractId, ContractState>,
    pub facilities: BTreeMap<FacilityId, FacilityState>,
    pub scheduled_actions: BoundedSchedule,
    pub news: BoundedNewsLog,
    pub scores: BTreeMap<ParticipantId, ScoreState>,
    pub agents: BTreeMap<AgentId, AgentState>,
    pub sequence: u64,
}

pub struct ListingKey {
    pub venue_id: VenueId,
    pub instrument_id: InstrumentId,
}
```

The concrete types must use checked IDs and bounded collections. A listing is a venue-specific tradable OrderBook-rs book; an economic instrument may have several listings. One command may stage changes across several candidate books, accounts and facilities, but it commits one canonical event batch and one next run version or nothing.

The generalized transition remains:

```text
authenticate and decode tRPC command
  -> recover the complete run candidate
  -> validate role, scenario state and expected sequence
  -> reserve all affected cash, inventory and capacity
  -> run risk and stage matching/product operations
  -> calculate ledger, penalties, scoring and projections
  -> build one ordered canonical event batch
  -> atomically commit origin version and projections
  -> publish/cache only committed public/private/admin outputs
```

## Feature requirements

The requirement IDs below supplement the binary-derived `RIT-FEATURE-*` ledger. “Documented” means the external capability exists; it does not prove Rotman’s internal formula or ordering.

### Cases, runs and instructor control

| ID | Requirement | Engine owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-CASE-001` | Define a versioned, bounded `ScenarioDefinition` containing listings, participants, agents, news, events, risk, scoring and pacing requirements. | `bunting-engine::scenario` | Officially documented casefiles; Bunting-native schema. Canonical serialization and content hash are deterministic. |
| `SIM-CASE-002` | Validate structural references, units, duplicate IDs, tick/lot bounds, product dependencies, recipient targets and engine capabilities before publication. | `bunting-engine::scenario` | Bunting-added. Invalid scenarios return stable field errors and cannot instantiate a run. |
| `SIM-CASE-003` | Publish immutable scenario versions and create iterations pinned to scenario hash, seed, engine version and scoring version. | engine run metadata | Official periods/iterations; binary `0001`-`0003`. Recreating the same tuple produces the same initial hash. |
| `SIM-CASE-004` | Support stopped, active, paused and terminated lifecycle with start, pause, resume, bounded advance and terminate commands. | `run`, `clock` | Binary `0001`-`0002`; official controls. Invalid transitions are deterministic. |
| `SIM-CASE-005` | Separate logical time from wall-clock pacing and support lockstep, accelerated and paced modes without making an isolate timer authoritative. | `clock`, scheduler | NBC evidence plus official speed controls. Replay is independent of wall-clock scheduling. |
| `SIM-CASE-006` | Record every live parameter, liquidity, limit and pacing change as an authenticated admin command with effective logical time, reason and expected sequence. | run policy | Official live control; Bunting-added audit contract. Direct D1 mutation is prohibited. |
| `SIM-CASE-007` | Reset between iterations from immutable scenario state and named initial seeds, with no retained participant, book, agent or RNG state. | run initialization | Official multiple iterations. Reset equivalence is verified by canonical state hash. |
| `SIM-CASE-008` | Treat legacy XLSX casefiles as an optional native import/export format into the canonical Bunting scenario schema, never as executable formulas or authoritative runtime state. | external casefile tool | Official Excel casefiles. Import requires separate format evidence and redistribution authority. |

### Matching, orders and market data

| ID | Requirement | Engine owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-MKT-001` | Maintain a private OrderBook-rs book per listing and expose no mutable matcher handle. | `matching` | ADR 0019; binary `0007`, `0015`-`0019`. Only engine transitions mutate a book. |
| `SIM-MKT-002` | Support the useful upstream limit, market, IOC, FOK, post-only, iceberg/reserve, pegged, trailing-stop and market-to-limit surface incrementally. | `matching`, command policy | OrderBook-rs evidence. Each exposed order type has canonical events and regression tests. |
| `SIM-MKT-003` | Preserve partial fills, price-time priority, ownership, STP, fees, expiry, mass cancel, kill/halt and typed rejection behavior. | `matching`, lifecycle | Upstream evidence; RIT priority details remain unresolved. Snapshot/replay preserves order state exactly. |
| `SIM-MKT-004` | Support single, scoped bulk and compatibility-expression cancellation without embedding an unbounded expression runtime. | command policy | Binary `0009`. Grammar and atomicity remain compatibility-gated. |
| `SIM-MKT-005` | Publish bounded committed L1, aggregated/raw L2, trades, OHLC/history, time-and-sales, volume, metrics and impact views. | market-data projection | Binary `0015`-`0019`, `0047`, `0050`. Reset and gap recovery use committed sequence cursors. |
| `SIM-MKT-006` | Keep participant-private live/open/historical order projections distinct from public market data. | private projection | Binary `0020`; official monitoring. Ownership and audience tests prevent leakage. |
| `SIM-MKT-007` | Record logical millisecond/nanosecond time on commands and events without promising a continuously executing one-millisecond Worker loop. | clock, event envelope | Official millisecond reporting; Bunting deployment constraint. |

### Accounts, accounting and risk

| ID | Requirement | Engine owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-ACCT-001` | Maintain exact per-participant, per-currency cash with settled, reserved, accrued and scheduled amounts. | ledger | Binary `0006`, `0022`, `0036`; official FX/accounts. No global single-currency shortcut after FX. |
| `SIM-ACCT-002` | Maintain positions, reservations, cost basis, realized/unrealized P&L, fees, rebates, interest, margin, penalties and net liquidation value. | ledger, valuation | Binary `0021`-`0025`, `0036`; official reports. Every mark names a versioned policy. |
| `SIM-ACCT-003` | Produce balanced typed transaction postings for trades, tenders, OTC, leases, usage, commissions, MTM, closeout, settlement, fines, interest, dividends and adjustments. | ledger journal | Binary `0036`. Replaying postings exactly reconstructs account state. |
| `SIM-ACCT-004` | Enforce instrument bounds, shortability, start/stop windows, buying power, inventory and open-order exposure before execution. | risk, command validation | Binary `0026`-`0028`; current Bunting partial coverage. Worst-case fill is included. |
| `SIM-ACCT-005` | Support named gross/net groups, notional, concentration, margin and scenario stress with hard-reject, allow-and-penalize and warning modes. | portfolio risk | Official risk capabilities; binary `0025`. Exact RIT formulas remain unresolved. |
| `SIM-ACCT-006` | Add VaR only as a versioned derived risk model with fixed-point admission/penalty outputs; floating-point cannot determine order priority or ledger equality. | risk analytics | Official VaR-oriented cases; Bunting-added numerical rule. |
| `SIM-ACCT-007` | Preserve risk, ledger and matching as one candidate transition so any late failure discards all reservations, fills and postings. | engine transition | Bunting integrity invariant. Property tests cover rollback and replay. |

### Deterministic agents and liquidity

| ID | Requirement | Engine owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-AGENT-001` | Agents observe bounded committed/candidate views and submit ordinary commands; they never mutate books, ledger or price directly. | agents | Official AI flow; ADR 0018. Agent commands pass normal risk and persistence. |
| `SIM-AGENT-002` | Derive named RNG streams from run seed, iteration, agent ID, listing and stream name; snapshot algorithm, version and state. | agents, RNG registry | NBC evidence and Bunting determinism. Same inputs reproduce identical events/hash. |
| `SIM-AGENT-003` | Provide versioned noise traders with bounded arrival, side, size and price-displacement distributions. | agent model | Official noise flow. Formula is Bunting-native unless stronger evidence is linked. |
| `SIM-AGENT-004` | Provide inventory-aware liquidity providers with configurable levels, spread, quantity, replenishment and withdrawal. | agent model | Official liquidity controls; binary `0041`; NBC evidence. Price emerges through orders. |
| `SIM-AGENT-005` | Provide informed/partially informed agents following versioned fundamental paths and information coefficients. | agent model | Official documented behavior. Hidden RIT formulas are not claimed. |
| `SIM-AGENT-006` | Add institutional, momentum, spiking and options-flow agents only with explicit model provenance, units, golden vectors and distributional tests. | agent models | NBC and official capability evidence. Each model lands as a working vertical slice. |

### Institutional workflows, venues and facilities

| ID | Requirement | Engine owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-INST-001` | Support deterministic multi-order/composite commands with explicit best-effort, minimum-fill, all-or-none and atomic-conversion policies. | composite transition | Official institutional workflow; binary `0011`-`0012`. Atomic policies stage all legs before commit. |
| `SIM-INST-002` | Model tenders as targeted, expiring state machines with create, bid/accept, decline, allocation and close outcomes. | tenders | Binary `0029`-`0030`; official tenders. Allocation/tie formulas remain evidence-gated. |
| `SIM-INST-003` | Model OTC negotiation separately from the CLOB through propose, counter, accept, reject, expire, book and authorized break transitions. | OTC | Binary `0013`; official OTC. Accepted trades pass credit, inventory and settlement checks. |
| `SIM-INST-004` | Separate economic instrument identity from venue listing identity; each listing has independent book, fees, liquidity and history. | instrument/listing registry | Official multi-marketplace support. Cross-listed books do not auto-synchronize. |
| `SIM-INST-005` | Provide consolidated BBO/depth and arbitrage estimates as projections without implicit smart routing. | market-data analytics | Bunting-added interpretation of official multi-venue cases. |
| `SIM-INST-006` | Model assets, leases, transport, storage, production and conversion as bounded capacity-constrained facilities with scheduled jobs. | facilities, settlement | Binary `0032`-`0035`; official commodities. Formula and allocation details are versioned policies. |

### News, products, settlement and scoring

| ID | Requirement | Engine owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-PROD-001` | Schedule immutable scenario news and accept audited live instructor news with public, participant, role or team audiences. | news | Binary `0031`; official targeted news. Filtering occurs before publication. |
| `SIM-PROD-002` | Represent equities, currencies, bonds, options, futures, commodities and synthetics as versioned instrument definitions outside matching logic. | instrument registry | Binary `0005`; official product list. Unsupported lifecycle operations reject explicitly. |
| `SIM-PROD-003` | Emit explicit dividends, interest, coupons, accrued-interest consideration, principal, variation margin, exercise, expiry and delivery events. | settlement | Binary `0036`-`0040`; official product behavior. Golden vectors define Bunting policies. |
| `SIM-PROD-004` | Support FX cash accounts, interest schedules, forwards, conversions and money-market cashflows without approximate persisted balances. | settlement, ledger | Official FX; binary `0006`. Checked conversions and accrual ordering are tested. |
| `SIM-PROD-005` | Support exact basket creation/redemption and facility recipes that atomically consume inputs/capacity and create outputs after configured delays. | settlement, facilities | Official synthetic/commodity behavior; binary `0034`. |
| `SIM-PROD-006` | Apply versioned distressed closeout, terminal marking and settlement rules at period/run termination. | termination, settlement | Binary `0035`, `0040`; formulas unresolved until specified. |
| `SIM-PROD-007` | Calculate versioned score inputs, rankings and iteration summaries from committed state only. | scoring | Binary `0043`-`0046`; official reports. No RIT-equivalence claim without formula evidence. |

### Connectivity, monitoring, reporting and recovery

| ID | Requirement | Owner | Evidence and acceptance |
|---|---|---|---|
| `SIM-OPS-001` | Keep Rust-owned `bunting.v1` tRPC as the only public application API, including scenarios, runs, markets, orders, accounts, news, tenders, OTC, admin and report procedures as implemented. | API contract/Worker | ADR 0016. No REST fallback in the Worker. |
| `SIM-OPS-002` | Keep FIX in the native client bridge, with its session sequence distinct from engine event sequence. | `clients/fix-bridge` | ADR 0015 retained by ADR 0016. FIX cannot mutate engine internals. |
| `SIM-OPS-003` | Implement RIT REST, VBA and RTD/Excel compatibility only in external adapters that authenticate and call tRPC. | clients/adapters | Binary `0048`-`0058`. Windows COM never enters the Worker graph. |
| `SIM-OPS-004` | Apply verified role/participant identity, per-participant API throttles and scenario availability without trusting caller-selected participant headers. | auth/gateway/run policy | Official anonymous/credentialed access; binary `0027`; ADR 0016. |
| `SIM-OPS-005` | Publish authorized public, participant-private and administrator projections with committed sequence, reset, gap and slow-consumer behavior. | projections/streams | Official monitoring; ADR 0011/0016. Private news/account state cannot leak. |
| `SIM-OPS-006` | Freeze iteration state and generate participant reports, transaction logs, P&L, time-and-sales, OTC activity and leaderboards from committed snapshots/events. | reporting app/package when implemented | Official reports. Heavy CSV/XLSX/Parquet work stays outside the Worker hot path. |
| `SIM-OPS-007` | Store authoritative commands/events/versions in origin, immutable public snapshots in Workers Cache, and large scenario/report artifacts in R2 only when implemented. | persistence/platform | Existing ADRs plus Bunting-added R2 boundary. Cache/R2 never coordinate transactions. |
| `SIM-OPS-008` | Snapshot all books, ledger, clock, schedules, tenders, news, facilities, scoring, agents, RNG and compatibility state; replay must reproduce one canonical state hash. | engine recovery | ADR 0018/0019. Native and Wasm golden hashes match. |
| `SIM-OPS-009` | Support inactive/paused remote-practice runs by persistence and on-demand reconstruction; optional wakeup coordination cannot become market authority. | Worker/platform | Official 24/7 availability reconciled with Cloudflare execution. |
| `SIM-OPS-010` | Generate the TypeScript client from the Rust contract and build student/instructor web applications as separate consumers of tRPC, not as engine packages. | generated client and web apps | Official student/instructor terminals; ADR 0016 client-generation boundary. |

Cloudflare platform roles remain governed by the existing ADRs and the official [D1](https://developers.cloudflare.com/d1/), [R2](https://developers.cloudflare.com/r2/), [Queues](https://developers.cloudflare.com/queues/) and [Durable Objects](https://developers.cloudflare.com/durable-objects/) documentation. Naming a service here does not approve its use; each binding lands only with a reviewed implementation and recovery contract.

## Procedure families

The canonical tRPC contract grows only with implemented vertical slices:

```text
scenarios.validate | publish | list | get
runs.create | get | start | pause | resume | advance | setPacing | terminate | subscribe
markets.listings | snapshot | trades | history | subscribe
orders.submit | replace | cancel | massCancel | submitComposite | list
accounts.snapshot | positions | cashflows | risk | subscribe
news.list | subscribe
tenders.list | accept | decline
otc.propose | counter | accept | reject
admin.publishNews | updateRunParameter | setParticipantLimits | monitor
reports.generate | status | get
```

Procedure names are requirements, not current API claims. Wide integer values remain validated decimal strings at the tRPC boundary, mutation batching remains rejected, and every mutation carries command, correlation and expected-sequence data.

## Package and dependency policy

Do not create a package for every heading above. Begin as cohesive modules inside `bunting-engine`; extract a reusable package only when a second real consumer proves a stable non-authoritative boundary. Existing focused market-type, event, ledger, risk, origin, protocol and client packages remain valid where their responsibilities are already real.

Candidate dependencies mentioned in research—schema, spreadsheet, RNG, statistics, graph, reporting and analytics crates—are not approved by this specification. Each requires the normal reference-adoption, license, exact-version, Wasm and dependency-graph review before addition. Native reporting or Excel dependencies must stay outside the Worker graph.

## Verification contract

Every implemented slice adds deterministic and recovery tests proportional to its authority. At minimum, the completed system proves:

- no negative remaining quantity and original quantity equals fills plus remainder;
- price/FIFO priority and partial-fill queue position follow the pinned upstream contract;
- buyer/seller quantities and ledger postings balance;
- position and cash equal replayed committed postings;
- failed multi-book candidate transitions leave no mutation;
- paused logical time does not execute scheduled actions;
- identical scenario/engine/model/seed/commands produce identical events and state hash;
- private data never appears in another audience’s stream;
- full snapshot plus tail replay equals uninterrupted execution;
- no production caller can reach OrderBook-rs except through `bunting-engine`;
- native and `wasm32-unknown-unknown` checks pass.

Implementation order and review boundaries are maintained in [`../plans/unified-bunting-engine-roadmap.md`](../plans/unified-bunting-engine-roadmap.md).
