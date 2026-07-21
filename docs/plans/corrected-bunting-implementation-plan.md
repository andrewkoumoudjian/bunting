# Corrected Bunting implementation plan

Status: active, persisted 2026-07-13; product-alignment sequence accepted 2026-07-15

The production hardening sequence is tracked in
[`production-readiness-plan.md`](production-readiness-plan.md). Its Phase 1
safety slice is the active milestone; it preserves ADR 0013 authority and ADR
0016's measured, non-authoritative stream-coordinator gate.

## Product-alignment execution sequence

This sequence reconciles the RIT-class product contract with the narrower
implemented order and market-data slice. The competition invariant is that
every participant-visible action and observation is available through FIX;
the TUI is the reference FIX client rather than a privileged control path.
Assets and leases, OTC, options, Excel RTD/VBA, voice, multi-run hosting, and a
web UI are deferred until after the competition MVC.

The public competition profile moves before competition-message implementation
to `bunting.fixlatest.competition.v1`: FIXT.1.1 session semantics, FIX 5.0 SP2
application semantics, and FIX Latest Orchestra as the normative dictionary
source. Standard FIX 5.0 SP2 messages replace extensions where possible;
Bunting `U*` messages remain only for genuinely product-specific tenders,
score, and run control.

The wire and session implementation must come from an existing pure-Rust,
Wasm-compatible FIX crate. Evaluate maintained RustyFIX first and FerrumFIX
second, using the pinned FerrumFIX reference only as evidence. Adopt a crates.io
release under `reference-adoption.md`; never depend on `ref/`. A spike must
prove `wasm32-unknown-unknown`, sans-I/O operation, bounded buffers, fixed-point
field decoding, FIXT.1.1 recovery, and FIX 5.0 SP2/Orchestra support. If no
candidate satisfies the session requirements, retain the sans-I/O
`simfix-session` core and replace only its wire/dictionary layer. Keep the old
stack temporarily as a differential oracle and retain `simfix-mapping` as the
FIX-to-application boundary.

- [x] **Phase 0 - hygiene:** archive dirty tRPC-era worktree state with pushed
  tags, prune all stale worktrees and branches, remove empty scaffold trees and
  generated Worker output, and mark tRPC-era plans as superseded.
- [x] **Phase 1 - compile speed:** tune dev/release profiles, remove the chart
  git dependency through licensed source adoption or an approved fallback,
  path-filter Wasm CI on pull requests, and document focused checks/sccache.
- [x] **Phase 2 - TUI event loop:** move socket ownership to an I/O task with
  bounded 256-event and 64-command channels; coalesce redraws behind a dirty
  flag; never block rendering on sends; drop market deltas, never private
  reports or acknowledgements, under backpressure.
- [x] **Phase 3 - unified CLI:** ship one `bunting` binary with `server`, `tui`,
  `relay`, `init`, and `version` commands; retain old binary names as one-release
  compatibility shims and install one release artifact.
- [x] **Phase 4 - runnable everywhere:** provide zero-config local defaults,
  bounded isolated hosted sessions, and concrete hosted/Cloudflare relay
  deployment guides and smoke gates.
- [x] **Phase 5 - server runtime:** extract deterministic scheduling and agents
  from the TUI fixture into `packages/bunting-runtime`; host it in the server
  with authenticated participant roles and a single commit-before-ack writer.
- [x] **Phase 5.5 - FIX adoption and upgrade:** accept an ADR-backed existing
  engine, port server/TUI/relay/Worker paths, prove differential session
  recovery, replace the profile, and generate participant dictionaries before
  any competition extension is implemented.
- [x] **Phase 6 - competition MVC:** land discovery/account, news/tenders, then
  risk/score/admin as three reviewable slices with audience enforcement and
  versioned PnL, commission, news, tender, risk, fine, and score policies.
- [x] **Phase 7 - TUI parity:** apply the Longbridge interaction idioms, consume
  authoritative FIX account state, add news/tenders/leaderboard/instructor
  workflows, and snapshot-test every tab.
- [x] **Phase 8 - hardening:** run a real TCP TUI/server black-box suite,
  QuickFIX/J or quickfix-go FIXT.1.1/FIX 5.0 SP2 interop, and deterministic
  golden full-run ledger/score/transcript tests in CI.

The profile upgrade is a hard gate between Phases 5 and 6. A phase is complete
only when its documented validation gate passes; specification text never
counts as implementation evidence.

Phase 3 completed on `codex/bunting-product-alignment`: the native server and
terminal now expose reusable library entrypoints behind `apps/bunting-cli`; the
release workflow builds one `bunting` executable and packages the old names as
one-release aliases. CLI parsing, compatibility dispatch, configuration init,
focused Clippy/tests, and a native release build passed. The workspace Wasm
retry was deferred to the final gate because the active toolchain did not have
`wasm32-unknown-unknown` installed; the CLI itself target-gates all native
dependencies behind an inert Wasm stub.

Phase 4 completed on the same branch: `bunting server` now boots an ephemeral,
bounded loopback run with no file or environment setup; hosted-native profiles
require isolated file state, an immutable scenario, one static FIX binding and
loopback administration behind mutual TLS. The deployment guide records native
and Cloudflare relay gates. Raw Workerd `2026-07-16` loaded the release JS/Wasm
bundle, returned compatible health, instantiated the FIX Durable Object, and
proved that the deliberately absent D1 backend fails closed rather than
falling back to non-authoritative state.

Phase 5 completed on the same branch: `packages/bunting-runtime` now owns the
deterministic sans-I/O wake queue, bounded action cascades, mandatory QUARCC
participant state, authenticated `BuiltInAgent` identities, and portable
snapshot/restore. Both the TUI fixture and native server host that package. The
server serializes runtime and FIX recovery/commit work through one writer,
releases the writer before socket output, and therefore cannot acknowledge a
command before the origin transaction returns committed events. Focused tests
proved authenticated agent commits and deterministic resume; a live
zero-configuration run advanced origin state to committed sequence 6 and event
sequence 18 through the server-hosted runtime.

Phase 5.5 completed on the same branch under ADR 0021. Full RustyFIX and
FerrumFIX sessions failed the bounded Worker/recovery gates, while exact
`rustyfix-dictionary 0.7.4` passed native and Wasm with only `fixt11` and
`fix50sp2`. The shared server, TUI, relay and Worker now use FIXT.1.1,
`DefaultApplVerID(1137)=9` and `bunting.fixlatest.competition.v1`; the
first-party bounded session retains snapshot recovery, with a checked legacy
transition golden. The Bunting-owned Orchestra overlay is deterministically
generated from the profile. Focused tests, Wasm checks, the Worker release
build and raw Workerd health/FIX-Durable-Object/D1-failure-closed smoke passed.

Phase 6 completed on the same branch as three authority-preserving slices.
Standard SecurityList, PositionReport and News messages now carry discovery,
account and audience-filtered news projections; Bunting `U6`, `U9`, `UA` and
`UB` cover targeted tenders, score, run control and risk/admin. Participant
tender decisions and operator mutations share the same idempotent atomic origin
commit as orders, including the D1 adapter. Every projection names the exact
Bunting-native PnL, zero-commission, news, tender, risk, fine and NLV-rank
policy. Tests prove private-news denial, targeted tender authority, deterministic
score ordering and an exact balanced fine debit; no RIT formula equivalence is
claimed.

Phase 7 completed on the same branch. The Longbridge-derived tab, table,
command-palette and popup idioms now render authoritative FIX discovery,
account, position/PnL, news, tender, risk and score projections. Tender and
role-authorized run/news/score/fine workflows emit bounded profile messages;
the peer Logon exposes the server-validated role before administration controls
are shown. A full-frame deterministic golden covers every terminal tab, and the
embedded TCP fixture exercises the negotiated competition projection set.

Phase 8 completed on the same branch. A headless black-box harness now drives
the real TUI TCP/session/projection client through the native server acceptor.
The exact QuickFIX/Go `v0.9.10` engine serializes FIXT.1.1 Logon and FIX 5.0 SP2
SecurityListRequest frames accepted by Bunting, then parses Bunting's Logon and
SecurityList responses in CI. A checked full-run golden pins the canonical
state hash, balanced fine/dividend journal, NLV-rank score report and complete
committed event transcript.

Reconciled 2026-07-15 on `codex/reconcile-bunting-product`. The product
contract, simulation domain, portable server, and Ratatui lanes now compile and
test as one workspace. This establishes one portable application/engine path,
native FIX order-entry parity with the Worker command path, deterministic
simulation replay, and an engine-backed FIX TUI fixture. It does not close the
unchecked expanded FIX mappings, external interoperability, or full Ratatui
workflow rows below; exact reconciliation evidence and requirement IDs are in
[`worktree-handoffs/reconciliation.md`](worktree-handoffs/reconciliation.md).

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

## Canonical product contract

- [x] Freeze `bunting.product.v1` as the transport-neutral application,
  deployment, identity, lifecycle, recovery and FIX-only competition contract.
- [x] Freeze `bunting.fixlatest.competition.v1` with FIXT.1.1 session and FIX
  5.0 SP2 application semantics, with standard messages where
  suitable, Bunting extension tags/messages elsewhere, explicit audiences and
  Cloudflare outbound-only TCP topology.
- [x] Add compile-time role/audience types and deny-by-default audience tests in
  `bunting-api-contract`, plus machine-readable product/FIX schemas and
  dictionary integrity tests.
- [x] Record the complete clean-room RIT workflow target and current Ratatui
  implemented/partial/missing state without copying proprietary UI material.
- [ ] Implement the expanded product procedures and FIX discovery, account,
  news, tender, OTC, asset/lease, report/score and admin mappings. The current
  application/FIX implementation remains a narrow order and market-data slice.
- [ ] Reach Ratatui workflow parity. The current TUI has market, orders, FIX
  session, order-ticket, price-chart, help and diagnostic slices; account,
  risk, news, tender, OTC, assets, reports/score and instructor workflows are
  missing.
- [ ] Specify and implement each unresolved RIT formula under an explicitly
  versioned Bunting policy with units, rounding, ordering and golden vectors.
  No current policy may be described as exact RIT compatibility.

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

- [x] Add `packages/bunting-agents` with a transport-neutral `AgentPolicy`
  contract and `ManagedAgent<P>` composition that always includes QUARCC.
- [ ] Complete noise and liquidity policies: the current Bunting-native
  dispatcher implements bounded deterministic first slices for
  zero-intelligence/Poisson flow,
  side/size/price distributions, cancellation, static/multi-level/fundamental
  replenishment, and stress withdrawal, but formula-specific provenance,
  distributional vectors and scenario integration remain incomplete.
- [ ] Complete market makers: the current dispatcher covers fixed spread,
  inventory skew, EWMA
  volatility, book/order-flow imbalance, Avellaneda-Stoikov, GLFT, and
  queue-aware quoting as Bunting-native policies, but model-specific units,
  golden vectors and compatibility evidence remain incomplete.
- [ ] Complete fundamental and educational policies: current Bunting-native
  branches cover informed variants,
  momentum, mean reversion, giveaway, ZIC, shaver, ZIP, Adaptive Aggressive,
  PRZI, and spiking, but exact source/evidence formulas remain unresolved.
- [ ] Complete HFT policies: current Bunting-native branches cover Hawkes
  events, microprice, imbalance,
  queue-reactive quoting, spread capture, order-flow momentum, fast withdrawal,
  logical latency, and cross-venue arbitrage, but distributional/replay evidence
  and engine scheduling integration remain incomplete.
- [ ] Complete institutional policies: current Bunting-native branches cover
  TWAP, VWAP, POV, arrival price,
  implementation shortfall, liquidity seeking, blocks, tender hedging, and
  multi-venue parent orders, but tender/facility integration and benchmark
  policy vectors remain incomplete.
- [x] Policies emit desired intents; QUARCC alone reconciles live orders.

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

## Phase 4: FIX over outbound TCP

### Wire and session packages

- [x] Add `packages/simfix-wire`: bounded incremental SOH tag-value framing,
  partial/coalesced reads, retained tails, `BeginString`, `BodyLength`,
  `MsgType`, `CheckSum`, repeating groups, standard dictionaries,
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
