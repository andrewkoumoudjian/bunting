# Bunting architecture

Binding document. Reconciled 2026-10-09 against `main@1d857d1` and ADRs
0022, 0028–0033. Sections are labeled **Current** (true in the code today) or
**Target** (accepted, not yet implemented). Never describe a Target item as
done; the implementation log records when one lands.

The previous version of this file described the Cloudflare Worker /
OrderBook-rs architecture; it is preserved in Git history (`1d857d1`).

## 1. Purpose

Bunting is a Rust market-simulation and exchange-testing platform for
education, research and trading competitions. It is not a real-money
exchange.

One headless, deterministic engine owns market truth. Two products are built
on it: a RIT-class instructor/student simulator and a QUARCC competition
venue. Every transport, UI and host is an adapter.

## 2. Binding principles

1. **One authority.** `bunting-engine` owns run state, logical time, listings,
   the order book, order ownership, risk admission, the single economic
   ledger, scoring, canonical events and market-data projections (ADR 0018,
   0028). No other crate, app, client or agent mutates market state.
2. **One book.** The engine's private price-time book (`src/book.rs`) is the
   only matcher (ADR 0029). OrderBook-rs is a dev-dependency differential
   oracle only.
3. **One ledger.** Every economic fact — fills, fees, tenders, OTC, fines,
   cashflows, scoring marks — posts through `bunting-ledger`'s single ledger
   (ADR 0028 item 3, Slice 9).
4. **Listing identity.** Venue-sensitive commands, books, trades and depth
   use `ListingKey (VenueId, InstrumentId)`; holdings aggregate by
   `InstrumentId` (ADR 0028 item 2).
5. **Determinism.** No wall clock, ambient randomness or host I/O inside
   transitions. Fixed-point checked arithmetic at every boundary (ADR 0009,
   0010). Same inputs ⇒ same events, state hash and scores.
6. **Commit before publish.** No acknowledgement, report or market-data
   update leaves the server before the origin commit is durable.
7. **Bounded everything.** Queues, buffers, sessions, journals, histories and
   per-participant resources have explicit limits with named rejections.
8. **Participants are outside.** Strategies, QUARCC execution engines,
   built-in agents and client apps submit ordinary commands and consume
   committed reports (ADR 0014 as carried by 0018).
9. **Two interfaces only.** FIX and the Bunting Native Protocol (ADR 0031,
   Target). Cloudflare only publishes immutable exports (ADR 0022).
10. **NBC is reference evidence**, not a compatibility target (ADR 0032).
11. **Host-neutral core.** Engine and protocol packages compile for
    `wasm32-unknown-unknown`; the server stays buildable natively; no
    WASIX-only dependencies (ADR 0033).

## 3. Topology

### Current

```text
FIX client (contestant engine, bunting TUI)
   │  TCP; TLS only via a trusted terminating proxy
   ▼
bunting-server  (std threads, blocking sockets; ships as WASIX module, ADR 0027)
   ├─ FIX acceptor: one thread per session (simfix-wire/session/mapping),
   │   each subscribed to the committed-event distributor (20 ms delivery poll)
   ├─ admin HTTP: /health, /admin/runs/<id>
   ├─ scenario runtime thread: built-in agents (bunting-runtime + bunting-agents)
   ├─ AuthoritativeWriter: sleep to 100 ms boundary, then arrival-ticket FIFO (ADR 0024)
   └─ bunting-application ─► command-transaction ─► origin store (Memory | File)
                                                      owns live RunState per run;
                                                      bunting-engine applies in place
                        File = BUNTWAL2 journal (genesis + one command record
                        per input: input, result, events, hash chain) +
                        state-only checkpoint every 8,192 commands + flock lease

Cloudflare: nothing built (Worker removed in eed8e00)
```

### Target

```text
FIX client ──┐                         ┌── certified app / TUI / bindings (bunting-client)
             ▼                         ▼   mutual TLS 1.3 in-process (ADR 0031)
        gateway: sessions, RTT probes, identity from certificate
             │
             ▼
   admission sequencer (ADR 0030): release = t_rx − d̂ + D_max + L(p,v)
             │  one recorded order of inputs (humans, agents, admin, schedule)
             ▼
   single writer owning live RunState ── bunting-engine transition in place
             │
             ├─► command journal (inputs + events + hash chain) + periodic checkpoints
             └─► committed-event distributor ──► per-participant private streams
                                               └─► per-listing public streams
                                                   (outbound hold per ADR 0030)
   archive = genesis snapshot + journal ──► independent replayer / judge
   immutable exports ──► Cloudflare publication (read-only)
```

## 4. Repository ownership

| Path | Responsibility |
|---|---|
| `packages/market-types` | Identifiers and checked fixed-point values |
| `packages/market-events` | Canonical commands, events, envelopes, reject codes |
| `packages/bunting-engine` | Run state, owned book, admission, ledger integration, simulation domain (tenders, OTC, news, facilities, scoring), snapshots/hashes |
| `packages/ledger` | Single economic ledger: cash, reservations, fees, positions, cost basis, P&L, marks, FX |
| `packages/risk-engine` | Pure admission over ledger counters |
| `packages/admission-sequencer` | ADR 0030 latency model: windowed-min RTT estimator, `physical`/`equalized`/`geographic` release, seeded path jitter, bounded `(release, arrival)` sequencer, `AdmissionRecord` (sans-I/O) |
| `packages/origin-store` | `OriginStore` trait; writer-owned `LiveRun` (in-place apply, idempotency index, event-hash chain, rollback); `RunRecovery`; in-memory store |
| `packages/command-transaction` | Thin command/simulation call shape over `OriginStore::execute` |
| `packages/bunting-application` | Transport-neutral service: identity, commands, projections, FIX mapping, competition views |
| `packages/bunting-runtime`, `packages/bunting-agents` | Deterministic built-in participant scheduling and policies (always via QUARCC execution) |
| `packages/quarcc-*` | Participant-side execution engine, Bunting adapter, Wasm binding |
| `packages/simfix-*` | FIX framing, session state machine, application mapping |
| `packages/bunting-api-contract` | Shared identity/role types and the browser procedure schema (browser part retired under ADR 0031) |
| `packages/browser-wire` | Unserved browser JSON transport — **retire** under ADR 0031 |
| `bunting-rs` | Curated composition crate and competition archive replay |
| `apps/bunting-server` | Venue host: acceptor, admin, writer, storage, scenario runtime |
| `apps/bunting-tui`, `apps/bunting-cli` | Native participant/operator terminal and CLI |
| `bindings/*` | C ABI, Python, C++ over `bunting-rs` |

Dependency direction: `packages/*` → `bunting-rs` → `apps/*`, `bindings/*`.
Packages never depend on apps.

## 5. Order book (Current)

Per listing, a `BTreeMap` of price levels, each a FIFO keyed by monotonically
increasing priority; orders keyed by canonical 128-bit `OrderId`. No clocks,
randomness or shared ownership; `Clone`; serialized canonically and covered
by the state hash. Semantics: execution at resting price; GTC, IOC, FOK
(feasibility against displayed + hidden), GTD on the logical clock, DAY
(rests until session close exists — Target, Slice 4); post-only; iceberg with
refresh at the back of the level; market orders sweep and cancel the
remainder; self-match permitted (prevention is policy above the book).
`OrderKind` is `Limit | Market | LimitWithPolicy`. New order types need
Bunting semantics, book tests and oracle coverage before entering the schema.

## 6. Command path

### Current

1. Session parses and bounds the FIX message; identity comes from configured
   credentials. Session-local command and order IDs are namespaced per
   participant session (slice 12) before they become canonical IDs.
2. `AuthoritativeWriter::execute_interval` waits for the interval boundary and
   arrival turn.
3. Application reads the committed sequence through a borrowing
   `read_run` closure and maps the message to a canonical command with
   `logical_time` from wall-clock epoch milliseconds.
4. The origin's `LiveRun` checks idempotency and expected sequence and applies
   the command in place (`RunState::apply`). `ApplyError::Unchanged` leaves the
   run untouched; `ApplyError::Poisoned` rolls it back by re-executing the
   inputs since the last checkpoint (slice 14).
5. File origin appends one command record (input, result, events, hash chain)
   and `fdatasync`s it before acknowledging; a failed append stops the store
   until restart. Every 8,192 commands the live state becomes the rollback
   base and is written as a state-only checkpoint.
6. `PublishingOrigin` publishes the committed events to the bounded
   committed-event distributor; every connected session maps the batch to its
   own participant's execution reports (slice 12), so resting makers receive
   unsolicited fills. Per-participant live-order caps are engine risk
   (`RiskLimits.max_live_orders`).

Measured cost: see slices 13 (before) and 14 (after) in the implementation
log. Per-command cost no longer grows with the size of the run.

### Target (Step 5 of the 2026-10-09 plan; Steps 1 and 3 landed in slices 12 and 14)

- Journal records gain the ADR 0030 admission metadata.
- The distributor gains public per-listing market-data streams and resume
  cursors so reports missed while disconnected are replayed (today they are
  not).

## 7. Admission and fairness

**Current:** ADR 0024 interval writer (sleep to a 100 ms boundary, then FIFO by
arrival ticket). Built-in agents commit under the writer lock outside that
queue.

**Target (ADR 0030):** continuous price-time matching per listing. A
deterministic sequencer orders all inputs — FIX, BNP, agents, schedule — by
`release = (t_rx − d̂(c)) + D_max + L(p, v)`, where `d̂` is half the windowed
minimum RTT and `L` is scenario path latency plus seeded jitter. Outbound data
is held to the same model. Modes `physical | equalized | geographic`. All
admission inputs are journaled; replay never re-measures the network.

## 8. Interfaces

**Current:** FIX (FIXT.1.1 / FIX 5.0 SP2 competition profile, ADR 0021/0023),
tag 207 required for venue. Admin HTTP on loopback. The browser contract has
no host.

**Target (ADR 0031):** FIX plus the Bunting Native Protocol over in-process
mutual TLS 1.3; certificate subject = actor identity; streams with resume
cursors; Ping/Pong for ADR 0030. One `bunting-client` crate shared by the
TUI, a GUI app and bindings. Nothing else accepts participant traffic.

## 9. Persistence, replay and archive

**Current (slice 14):** the origin owns each run's live state. File mode
journals `BUNTWAL2` frames (8-byte length, SHA-256, one JSON entry): a genesis
snapshot per run, then one command record per committed input with its
result, canonical events and an event-hash chain. The journal is never
compacted. A state-only checkpoint (snapshot + chain per run) is written every
`checkpoint_interval` commands; restart verifies the journal up to the
checkpoint by fingerprint and chain, then re-executes the rest and requires
identical records, so every restart is a determinism check. An incomplete
tail is cut off; a complete corrupt frame, a checkpoint ahead of the journal
or a chain mismatch fails closed; pre-format-2 stores are refused. Poison on
ambiguous writes; Unix-only `flock` writer lease; per-run bounds
`max_commands_per_run` and `max_events_per_run`. `CompetitionArchive` still
replays simulation commands only; the journal is its Step 4 input.

**Target (ADR 0025 as expanded by 0028 item 5):** archive = genesis snapshot +
complete journal of every input (orders, cancels, agent commands, admin,
schedule, admission metadata); the replayer verifies events, final hash and
scores from genesis and from checkpoints. Built-in agent runtime snapshots are
persisted with checkpoints so restarts resume identical RNG and wake state.

## 10. Time and calendar

**Current:** logical clock in `SimulationState`; GTD expiry index; FIX
admissions stamped from epoch milliseconds; no venue calendar.

**Target (Slice 4):** per-venue calendar and session phases in logical time
(open/close auctions, halts, DAY expiry, end-of-day marks, overnight carry);
admissions stamped from the run clock by the sequencer.

## 11. Hosting

**Current:** release ships the server as a WASIX module run by Wasmer
(ADR 0027) and native CLI/TUI/bindings for four targets. Durable file mode is
Unix-only.

**Direction (ADR 0033):** host not fixed. Keep the server native-buildable,
add no WASIX-only dependencies, prefer native static binaries and an OCI image;
a later ADR selects the host from measured native-versus-WASIX data.

## 12. Publication

Cloudflare may publish immutable, checksum-addressed archives, leaderboards
and public snapshots exported after commit. It never accepts commands, owns
sequences or holds origin truth (ADR 0022). No publisher is built today.

## 13. Validation gates

Every change runs the checks in root `AGENTS.md`. Engine changes also keep:
book unit tests and the OrderBook-rs differential oracle; ledger conservation
tests; snapshot/replay hash tests; replay-blessed goldens only (never
hand-edited). Performance claims require a recorded measurement (workload,
hardware, build, commit).
