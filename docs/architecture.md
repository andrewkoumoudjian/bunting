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
9. **Two interfaces only.** FIX and the Bunting Native Protocol (ADR 0031;
   BNP v1 built per ADR 0040). Cloudflare only publishes immutable exports (ADR 0022).
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
   │   each subscribed to the committed-event distributor (woken on publish)
   ├─ admin HTTP: /health, /admin/runs/<id>
   ├─ scenario runtime thread: built-in agents (bunting-runtime + bunting-agents),
   │   submitting through the sequencer from their own location
   ├─ per-connection reader thread stamps t_rx on arrival (real delay counts)
   ├─ admission sequencer thread, the only committer: release at
   │   t_rx + L(team, venue) (ADR 0035); venue messages sent L(venue, team)
   │   after the venue produced them
   └─ bunting-application ─► command-transaction ─► origin store (Memory | File)
                                                      owns live RunState per run;
                                                      bunting-engine applies in place
                        File = BUNTWAL3 journal (genesis + one command record
                        per input: input, result, events, admission, chain) +
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
   admission sequencer (ADR 0035): release = t_rx + L(p,v); outbound + L(v,p)
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
| `packages/admission-sequencer` | ADR 0035 latency: location map for teams, venues and the hub with per-direction seeded jitter (`t_rx + L(p, d)`, outbound `L(s, p)`, team-to-team paths), published access-latency estimator, bounded `(release, arrival)` sequencer, `AdmissionRecord` (sans-I/O) |
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
2. The connection's reader thread stamps `t_rx` (venue monotonic clock) the
   moment bytes arrive, so the team's real delay is inside it; the session
   maps the message to a canonical command and admits it without waiting.
   Real delay (kernel TCP RTT via netlink `sock_diag`, FIX probe RTT) is
   measured and published, never compensated (ADR 0035).
3. The sequencer thread releases admitted work in `(release, arrival)` order
   at `t_rx + L(p, d)` (the team's virtual distance to the addressed venue
   or hub, from the latency map; per-destination FIFO per connection), stamps the command's expected sequence
   and `logical_time = release`, and executes it under the writer gate.
4. The origin's `LiveRun` checks idempotency and expected sequence and applies
   the command in place (`RunState::apply`). `ApplyError::Unchanged` leaves the
   run untouched; `ApplyError::Poisoned` rolls it back by re-executing the
   inputs since the last checkpoint (slice 14).
5. File origin appends one command record (input, result, events, admission
   record, hash chain)
   and `fdatasync`s it before acknowledging; a failed append stops the store
   until restart. Every 8,192 commands the live state becomes the rollback
   base and is written as a state-only checkpoint.
6. `PublishingOrigin` publishes the committed events to the bounded
   committed-event distributor; every connected session maps the batch to its
   own participant's execution reports (slice 12), so resting makers receive
   unsolicited fills. Each batch is sent `L(s, p)` after commit, `s` being
   where its command was applied, then crosses the team's real connection. One live FIX
   session per participant. Per-participant live-order caps are engine risk
   (`RiskLimits.max_live_orders`).

Measured cost: see slices 13 (before) and 14 (after) in the implementation
log. Per-command cost no longer grows with the size of the run.

7. Public feeds (slice 21, ADR 0036): right after each commit, on the
   sequencer thread, `PublishingOrigin` computes the anonymous public view
   of every listing the commit touched (its trades and visible-depth level
   changes against the last published depth) and attaches it to the batch.
   A FIX session subscribed to a listing (`V` 263=1) receives a full-depth
   snapshot taken at the venue when the request arrives there, then one
   `X` per later commit touching that listing, each sent `L(v, p)` after
   commit from that listing's venue, with per-entry report sequence `83`.
8. Consolidated tape (slice 22, ADR 0036): `PublishingOrigin` also hands
   each commit's public changes to `ConsolidatedTape`
   (`apps/bunting-server/src/consolidated.rs`), the processor at the hub.
   A change is applied when it reaches the hub (`durable + L(v, hub)` plus
   the processing delay; the `bunting-tape` thread releases due changes)
   and published per instrument with one report sequence; sessions
   subscribed with `V` 207=0 receive each record `L(hub, p)` later.
9. Order-by-order feeds (slice 23, ADR 0036): once any session asks for
   one (`V` 266=N), `PublishingOrigin` also keeps every listing's
   displayed orders (`bunting_application::displayed_orders`, keyed by the
   book's time priority as the public reference) and attaches each
   commit's order changes and trade references to its public update.
   Broker identifiers (slice 24) come from the listing's `broker_ids`
   policy and each order's journaled `anonymous` flag
   (`bunting_application::order_broker`, `trade_brokers`).

### Target

- Resume cursors so reports missed while disconnected are replayed (today
  they are not).

## 7. Admission and fairness

**Current (slice 16, ADR 0035):** latency for FIX sessions works as on a real network:
real connectivity counts as it is; the published virtual team-to-venue
latency map (teams, venues and the hub at locations; team-to-team distance
included) is added in both directions; teams choose the venue for each
order, with no router and no trade-through protection; measured access
latency is journaled and published on `/admin/admission`. Built-in agents
submit through the same sequencer from their location in the map (slice
18), and the sequencer thread is the venue's only committer; agents learn
of fills other participants cause from the committed-event distributor.
They still read the book and receive reports without the venue-to-agent
delay. With a file origin their runtime is checkpointed each tick and
resumes exactly once after a restart (slice 19). Public per-venue feeds
(trades and L2 depth changes) reach each subscriber over its own path from
that venue (slice 21); the consolidated tape reaches them from the
processor at the hub after each venue's path there (slice 22), and
order-by-order feeds over the same paths as price levels (slice 23).

**Target:** the same model for every input — FIX, BNP, agents, schedule —
with agents also observing over their virtual paths, team-to-team
messages (OTC negotiation, shared data) addressed over team-to-team paths ([exploration](research/2026-10-10-cross-venue-market-data.md)).
All admission inputs are journaled; replay never re-measures the network.

## 8. Interfaces

**Current:** FIX (FIXT.1.1 / FIX 5.0 SP2 competition profile, ADR 0021/0023),
tag 207 required for venue. Bunting Native Protocol v1 (slice 25, ADR 0040,
[`specs/bnp-v1.md`](specs/bnp-v1.md)): fixed-layout binary frames over
in-process TLS 1.3 with mutual authentication; identity is the client
certificate's fingerprint in the run roster; orders, cancels, kill switch,
private reports keyed by committed event sequence with resume from a
retained window, direct L2 feeds, listings, open orders and account
snapshots, probes and Ping/Pong. Same sequencer and latency path as FIX.
Native builds only. Reference client: `packages/bunting-client` and
`apps/bunting-trader`. Admin HTTP on loopback. The browser contract has no
host.

**Target (ADR 0031):** instructor/admin control and run/news streams over
BNP; L3 and consolidated feeds over BNP; the TUI, a GUI app and bindings on
`bunting-client`; a FIX/BNP parity test. Nothing else accepts participant
traffic.

## 9. Persistence, replay and archive

**Current (slice 14):** the origin owns each run's live state. File mode
journals `BUNTWAL3` frames (8-byte length, SHA-256, one JSON entry): a genesis
snapshot per run, then one command record per committed input with its
result, canonical events, optional ADR 0030 admission record and a hash chain
over the whole record. The journal is never
compacted. A state-only checkpoint (snapshot + chain per run) is written every
`checkpoint_interval` commands; restart verifies the journal up to the
checkpoint by fingerprint and chain, then re-executes the rest and requires
identical records, so every restart is a determinism check. An incomplete
tail is cut off; a complete corrupt frame, a checkpoint ahead of the journal
or a chain mismatch fails closed; pre-format-3 stores are refused. Poison on
ambiguous writes; Unix-only `flock` writer lease; per-run bounds
`max_commands_per_run` and `max_events_per_run`.

Built-in agents (slice 19): every agent command is an ordinary journaled
input. With a file origin the agent runtime is checkpointed next to it
(`<origin>.agents.json`, atomic replace) once per tick, after its agents
decide and before any decision is submitted; the checkpoint carries the
pending actions and the event sequence handed to the agents. A restarted
venue restores it, hands the agents other participants' commits since then,
and re-submits the pending actions: one already in the journal resolves to
its recorded events instead of committing again, so each agent action
takes effect exactly once. A run with agent commands but no agent
checkpoint, or a checkpoint for another runtime configuration, refuses to
start.

Archive v2 (slice 20, ADR 0025 as expanded by 0028 item 5):
`CompetitionArchive` is a run's genesis snapshot plus every journaled command
record in commit order (orders, cancels, agent commands, simulation
administration and admission records), an optional checkpoint, and the final
chain value and state hash. Replay goes through `RunRecovery`, the same path
a restart uses: records the checkpoint covers are checked by fingerprint,
sequence and chain, the rest are re-executed and must reproduce the record
exactly. `bunting export-archive` reads a file origin's journal without the
writer lease and writes a verified archive; `bunting replay`, `score` and
`judge` consume it. Version 1 archives are refused. Schedule inputs join the
archive automatically once they are journaled commands (Step 6).

## 10. Time and calendar

**Current (slice 26, ADR 0037 stage 1):** one run clock. The sequencer
stamps every released input (FIX orders, built-in agents, competition
commands) with run time: under `Paced`, run time follows venue time at
`step_ns` per `step_interval_ns` and freezes while the run is not active;
under `Lockstep` and `Accelerated` it moves only by operator `Advance`. The
engine refuses an input behind the clock and first applies everything due
by its time (scheduled actions, GTD expiries). A venue timer submits a
journaled `ClockTick` through the sequencer at `RunState::next_due`, so
due items apply on time with no other input. The local and competition
scenario runs a real-time paced clock. No venue calendar yet.

**Target (ADR 0037 stages 2–4):** per-venue calendar and session phases in
run time (open/close auctions, listing halts, DAY expiry, end-of-day marks,
overnight carry).

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
