# Bunting product contract

Status: canonical target contract, version `bunting.product.v1`

This specification fixes the product boundary that implementation lanes must
share. It describes required behavior, not current completeness. Current status
is recorded in the corrected implementation plan and the TUI parity matrix.

## Authority and application service

Bunting has one transport-neutral, sans-I/O Rust application service around the
unified `bunting-engine`. Every adapter supplies an authenticated actor and a
typed query or command; the service authorizes the actor, validates bounded
fixed-point input, recovers the expected run version, applies one engine
transition, commits the canonical event batch and only then returns or publishes
the committed result. Adapters cannot receive mutable engine state or invent
facts, sequence numbers, fills, balances or scores.

The service boundary is the public Rust contract consumed in process by native
servers, the Worker, FIX mapping, browser handlers and built-in agents. Transport
sessions, origin persistence and cache adapters remain outside `bunting-engine`.
FIX sequence numbers remain distinct from committed Bunting event sequences.

## Deployment contracts

### Native local or server deployment

A native host may accept inbound standard FIXT.1.1 with FIX 5.0 SP2 application
semantics over TCP or TLS. The acceptor
owns sockets, TLS, FIX sessions, bounded journals and reconnect policy, then
calls the application service in process. It is not a second exchange service.

### Cloudflare deployment

The native Rust Worker accepts browser-compatible HTTPS fetch/stream traffic.
D1 is the origin for accepted commands, canonical events, idempotency, versions
and recovery projections. Workers Cache contains only immutable,
checksum-addressed public snapshots and never coordinates a transaction.

Cloudflare Workers do not accept inbound raw TCP. Each Worker FIX session is an
outbound TCP initiator connecting to an allowlisted external FIX acceptor. Once
established, the connection is bidirectional and the external acceptor relays
participant messages to the Worker session. An internet user therefore reaches
a Cloudflare deployment through this topology:

```text
participant FIX initiator -> external FIX gateway/session relay (acceptor)
                         <- Worker-initiated outbound TCP/TLS FIX session
                                      -> in-process Bunting application service
                                      -> bunting-engine -> D1 commit
```

The relay authenticates network/session peers, applies bounded backpressure and
persists its side of the FIX session. It does not translate market semantics,
acknowledge venue commands, own origin state or change Bunting/FIX sequences.

## Identities and authorization

Authentication produces an immutable actor ID, role, tenant and optional team
and participant IDs. Caller-selected identity headers or FIX fields never
override verified claims.

| Role | Permitted authority |
|---|---|
| Participant | Discover eligible runs, observe public and own private state, and perform participant actions for its participant identity. |
| Team | Observe team-private state and manage team participants only through explicitly delegated operations. |
| Instructor | Publish instructional scenarios, create/control assigned runs, communicate with participants and view instructor projections; it has no platform-secret authority. |
| Administrator | Manage tenants, identities, credentials, policy installation and all audiences; every action is audited. |
| Built-in agent | A service actor bound to exactly one participant/team/run scope. It submits ordinary commands through mandatory QUARCC reconciliation and has no instructor/admin privilege. |

Every publishable item carries exactly one audience: public, participant ID,
team ID, instructor or administrator. Authorization is deny-by-default. An
administrator may inspect all audiences; an instructor cannot read participant
private state unless a separately versioned run policy projects that fact to an
instructor audience. Team and participant matching uses verified IDs.

## Scenario and run lifecycle

A scenario moves `draft -> validated -> published`. Publication materializes
defaults, canonicalizes the document and creates an immutable `(scenario ID,
version, content hash)`. Editing creates a new version.

A run pins scenario hash, seed, engine build, product contract, policy-set,
scoring, PRNG derivation, quantization, clock and pacing versions. Its lifecycle
is `created -> stopped -> active <-> paused -> terminated -> archived`. Start,
pause, resume, bounded advance, pacing changes and termination are authenticated
commands with expected version, effective logical time, reason and actor.

Logical time is authoritative. Wall-clock pacing determines when eligible work
is attempted, never its ordering. Supported modes are lockstep, paced and
accelerated. Events order by logical time, policy priority and deterministic
insertion sequence. Resetting an iteration reconstructs immutable scenario
state and named seeds without retaining book, ledger, agent or RNG state.

## Versioning and recovery

Every mutation carries command ID, correlation ID and expected run version.
Duplicate IDs return the original committed result; conflicting reuse rejects.
An optimistic-version loser reloads and may retry only under a bounded policy
without changing external IDs. Complete snapshots include all authoritative and
deterministic engine state; OrderBook-rs state is one nested component.

Streams start with a snapshot and continue with committed-sequence updates. A
resume cursor receives an available tail or a reset plus current snapshot.
Public book state may coalesce, but trades and private execution/account facts
cannot disappear silently. A slow consumer is disconnected with a usable
recovery cursor. No recovery guarantee depends on isolate affinity.

## FIX-only competition invariant

The profile in [`bunting-fix-competition-profile.md`](bunting-fix-competition-profile.md)
is a complete competition interface. A participant can observe every fact the
competition makes visible to that participant and perform every permitted
participant action through FIX alone. Browser and Ratatui clients are views over
the same service and cannot expose an exclusive competition capability.

Instructor and platform-administrator operations need not be participant FIX
operations, but the profile defines admin-audience extensions so native operator
tools can use the same session model. Unsupported engine capabilities reject
explicitly; they never silently fall back to a browser-only path.

## Versioned policy boundary for unresolved RIT behavior

The RIT installers are static clean-room evidence. They prove fields, protocol
surfaces and workflows, not hidden server formulas. The following policy IDs
must be pinned by every run and must never be called RIT-compatible without new
behavioral evidence and golden tests:

- `matching-policy.v1`: priority, hidden/reserve, STP, auctions, halts and replace priority;
- `validation-policy.v1`: validation order, status/reject mapping, throttling, burst windows and simulated delay;
- `clock-settlement-policy.v1`: tick/period transitions, pause/resume, inter-period settlement and termination;
- `accounting-policy.v1`: fees, rebates, marks, cost basis, VWAP, interest, coupons, dividends, settlement, distressed cover and fines;
- `risk-valuation-policy.v1`: buying power, NLV, realized/unrealized P&L and gross/net risk;
- `scoring-policy.v1`: Sharpe, score, ranking and tie behavior;
- `tender-policy.v1`: winner selection, reserve, targeting, compliance and ties;
- `news-agent-policy.v1`: news effects, path liquidity and every virtual-trader formula;
- `otc-policy.v1`: expression evaluation, counterparties, breaks and correction settlement;
- `asset-facility-policy.v1`: lease allocation/renewal, conversions, containment, transport/backhaul and distress;
- `product-policy.v1`: spreads, transport arbitrage, trade-at-settle, electricity, options, bonds, swaps, forwards and fixings;
- `protocol-recovery-policy.v1`: serialization details, persistence, replay, deduplication, ordering, snapshots and RTD disconnect/refresh behavior.

These are Bunting-added policy namespaces. Their first implementations require
explicit formulas, units, rounding, ordering, bounds, golden vectors and
snapshot compatibility. Unknown parameters remain inert until such a policy is
implemented.

## Machine-readable contract

[`schemas/product/bunting.product.v1.json`](../../schemas/product/bunting.product.v1.json)
freezes topology, roles, lifecycle, recovery and the FIX invariant. Rust role and
audience types live in `bunting-api-contract`; the profile dictionary lives
under `schemas/fix/`. The schema is additive only within `v1`; a breaking change
requires a new version and explicit compatibility/migration rules.
