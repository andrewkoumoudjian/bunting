# Corrected Bunting implementation plan

Status: active, persisted 2026-07-13

This plan implements ADR 0018, ADR 0019, and ADR 0020. It supersedes active
roadmap text that treats tRPC as the permanent universal transport or routes
FIX through an RPC hop. Evidence and licensing gates in
`reference-functionality-audit.md`, `reference-adoption.md`, ADR 0017, and the
port records remain binding.

## Architecture and invariants

```text
Rust/WASM human client -- browser-compatible transport --+
FIX initiator -------- outbound bidirectional TCP --------+--> Worker application
built-in policies -- mandatory QUARCC, in-process Rust ---+        |
optional human/FIX QUARCC execution ----------------------+        v
                                                         bunting-engine
                                                         validation -> venue risk
                                                         -> OrderBook-rs matching
                                                         -> ledger -> canonical events
                                                         -> atomic origin commit
                                                         -> committed publication
```

- `bunting-engine` is the sole market authority and owns the only production
  matcher.
- QUARCC is reusable participant execution state. Human and FIX clients may
  bypass it; every built-in agent must use it.
- No participant component receives mutable engine state.
- FIX is an outbound Worker TCP initiator. The Worker never accepts inbound raw
  TCP, and no RPC hop exists between FIX mapping and the application
  transaction.
- Browser transport remains browser-compatible, with the human application
  implemented in Rust/WASM.
- Transport success is distinct from command acceptance and execution.
- QUARCC state never overrides committed venue truth.
- Snapshot plus replay equals uninterrupted execution, and portable packages
  behave deterministically on native and Wasm targets.

## Completed foundation

- [x] Create `packages/bunting-engine` and move the private OrderBook-rs 0.10.3
  adapter into it.
- [x] Add bounded multi-listing state, one transition boundary, exact
  ledger/risk composition, snapshots, state hashes, origin migration, and
  Worker integration.
- [x] Remove the standalone production order-book package.
- [x] Repair CI to inspect `packages/bunting-engine` and
  `cargo tree -p bunting-engine`.

## Phase 1: portable QUARCC

### Execution core

- [x] Rename the compatibility seed to `packages/quarcc-execution-engine` while
  retaining the `quarcc.v1` public compatibility module.
- [x] Implement typed local/client/venue IDs, exact price/quantity/money,
  capabilities, configuration, intents, actions, market observations,
  normalized reports, lifecycle, positions, participant risk, reconciliation,
  snapshots, restore, and deterministic replay.
- [x] Support `IntentReceived`, `PendingSubmit`, `Live`, `PartiallyFilled`,
  `PendingCancel`, `PendingReplace`, `Cancelled`, `Filled`, `Rejected`,
  `Expired`, `ExternallyDiscovered`, and `Quarantined`.
- [x] Make report deduplication, out-of-order reports, fill-before-acknowledgment,
  invalid transitions, kill-switch cancel planning, and bounded buffers
  explicit.
- [x] Keep sockets, Worker APIs, Tokio, filesystem, SQLite, and ambient time out
  of the core.

### Bunting adapter and Wasm binding

- [x] Add `packages/quarcc-bunting-adapter` to map QUARCC actions to canonical
  commands and committed private reports back to normalized reports, preserving
  command IDs, expected sequences, and acceptance-versus-fill semantics.
- [x] Add `packages/quarcc-execution-wasm` with JSON/`JsValue` submit,
  market-data, report, reconcile, snapshot, and restore methods.
- [x] Prove native/Wasm compilation, bounded output, duplicate idempotency,
  snapshot/replay equivalence, report-permutation properties, and deterministic
  reconnect reconciliation.

## Phase 1B: NBC compatibility in the unified engine

- [x] Move proven strict scenario parsing, provenance hashes, deterministic
  seed configuration, logical-step scheduling, event ordering, `DONE`
  synchronization, termination, and capability metadata into
  `packages/bunting-engine/src/compatibility/nbc`.
- [ ] Port only evidence-backed or explicitly Bunting-native fundamental,
  momentum, noise, market-making, institutional, spiking, normal, stressed,
  HFT-dominated, mini-flash-crash, and flash-crash models.
- [ ] Require provenance, formula version, units, named RNG streams, bounds,
  golden vectors, distributional tests, and snapshot state for every model.
- [ ] Keep unknown formulas inert.
- [x] Retain the translated NBC matcher solely as a development differential
  oracle; production NBC commands use the unified OrderBook-rs matcher.
- [x] Remove every production engine selector or dependency that can choose the
  NBC matcher.

## Phase 2: built-in agents

- [ ] Add `packages/bunting-agents` with a transport-neutral `AgentPolicy`
  contract and `ManagedAgent<P>` composition that always includes QUARCC.
- [ ] Implement noise and liquidity policies: zero-intelligence/Poisson flow,
  side/size/price distributions, cancellation, static/multi-level/fundamental
  replenishment, and stress withdrawal.
- [ ] Implement market makers in order: fixed spread, inventory skew, EWMA
  volatility, book/order-flow imbalance, Avellaneda-Stoikov, GLFT, and
  queue-aware quoting.
- [ ] Implement fundamental and educational policies: informed variants,
  momentum, mean reversion, giveaway, ZIC, shaver, ZIP, Adaptive Aggressive,
  PRZI, and spiking.
- [ ] Implement HFT policies: Hawkes events, microprice, imbalance,
  queue-reactive quoting, spread capture, order-flow momentum, fast withdrawal,
  logical latency, and cross-venue arbitrage.
- [ ] Implement institutional policies: TWAP, VWAP, POV, arrival price,
  implementation shortfall, liquidity seeking, blocks, tender hedging, and
  multi-venue parent orders.
- [ ] Policies emit desired intents; QUARCC alone reconciles live orders.

## Phase 3: transport-boundary correction

- [x] Apply ADR 0020 across root instructions, architecture, RIT-class spec,
  roadmaps, FIX/client/Worker/deployment documentation, and active comments.
- [x] Classify each legacy tRPC reference as browser compatibility,
  transitional implementation, obsolete fixture, or historical ADR text; do
  not replace browser transport with raw TCP.
- [x] Rename `apps/trpc-api` to `apps/bunting-worker` atomically with Cargo,
  Wrangler, migrations, CI, scripts, release paths, and scoped instructions.
- [x] Preserve browser fetch/stream handlers, route internal work through Rust
  application functions, and prevent new code from depending on tRPC-specific
  packages.
- [x] Delete transitional tRPC packages after browser transport migration and
  conformance coverage make them unused.

## Phase 4: FIX 4.4 over outbound TCP

### Wire and session packages

- [x] Add `packages/simfix-wire`: bounded incremental SOH tag-value framing,
  partial/coalesced reads, retained tails, `BeginString`, `BodyLength`,
  `MsgType`, `CheckSum`, repeating groups, FIX 4.4 dictionaries,
  deterministic serialization, structured errors, native/Wasm compilation.
- [x] Add `packages/simfix-session`: logon/logout, heartbeat/test request,
  inbound/outbound sequences, resend/gap fill, `PossDupFlag`,
  `OrigSendingTime`, sequence reset, reconnect, bounded queues, journal, and
  explicit clock/store/transport traits. It consumes bytes and connection
  events but owns no socket.

### Mapping and Worker initiator

- [x] Add `packages/simfix-mapping` for `D`, `F`, `G`, `H`, and `V` inbound
  messages and ExecutionReport, OrderCancelReject, snapshot, and incremental
  outbound messages.
- [x] Support direct and QUARCC-managed execution per FIX session.
- [x] Add a Worker FIX-session Durable Object that owns the outbound TCP socket,
  FIX sequences/journal/reconnect state, optional QUARCC state, and recovery
  cursor, but never owns market/order-book/cash/position/event authority.
- [x] Keep mapping-to-application execution in-process with no network/RPC hop.
- [ ] Test QuickFIX/J, a second acceptor, Worker staging TCP, partial/coalesced
  frames, invalid lengths/checksums, gaps/resends/gap fills/duplicates,
  reconnect/reactivation, order lifecycle, market data, direct/managed modes,
  and Bunting idempotency recovery.

## Recovery ownership

- The engine snapshot owns run/listings/matcher/ledger/venue risk/scheduler/NBC
  compatibility/agent policies/mandatory agent QUARCC/RNG/canonical hash.
- The human WASM snapshot owns optional local QUARCC state, desired orders, ID
  maps, report cursors, and optional UI state; it is not authoritative.
- The FIX snapshot owns session identity/sequences/journal/gap and reconnect
  state, optional QUARCC state, and Bunting cursor; it is not part of the market
  snapshot.

## Completion gate

- [x] One production matcher and one market authority remain.
- [x] Every built-in agent uses QUARCC; human and FIX use is optional.
- [x] No raw inbound TCP listener and no FIX-to-engine RPC hop exist.
- [x] Full native, formatting, Clippy, workspace tests, dependency-tree, Wasm,
  Worker release build, migration discovery, stale-path, and `git diff --check`
  gates pass.
- [ ] Every reference-derived behavior has observed/inferred/Bunting-added/
  unresolved/prohibited-to-copy provenance.
