# Bunting production-readiness plan

Status: active, persisted 2026-07-21

## Verified baseline

The audited baseline is `origin/main` at `7965909151dbdcd03d1588c4cfbc85127bd7c9f5`.
Bunting has a deterministic exchange core, but the hosted platform is not safe
for multi-participant production and market realism has not been validated.

The following defect families are confirmed against that baseline:

- public subscriptions serialize private-bearing canonical envelopes;
- transport-local command and order identifiers collide across sessions;
- commit races can return and publish a losing request's locally prepared output;
- TUI operator payloads diverge from the server contract, while replace and
  status remain advertised although the venue adapters reject them;
- every command rewrites the complete run projection, creating an unbounded D1
  row and recovery cost;
- one deployment token maps to one participant and provides no multi-user
  membership, revocation, role-claim, or session model;
- decimal `u64` sequences are sorted through signed SQLite integer casts;
- subscription responses are finite catch-up responses rather than live streams;
- role-scoped news compares strings against debug formatting;
- agent and market-policy realism has no calibration corpus, holdout suite,
  stylized-fact gate, or declared production profile.

## Binding architecture correction

ADR 0013's plain Worker plus optimistic origin commit remains authoritative.
ADR 0016 permits a per-run `RunStreamCoordinator` only after measured
plain-Worker streaming failure and prohibits that object from owning commands,
matching, the event store, or market state. A run-authority Durable Object is
out of scope and requires a new user-approved ADR.

## Execution sequence

### Phase 1 - safety slice

- [x] Add an allowlisted, transport-neutral public event projection in
  `bunting-application`; public transports must not accept `EventEnvelope`.
- [x] Split `Committed` and `Duplicate` outcomes everywhere. A duplicate must
  return the stored result, publish no locally prepared events or snapshots,
  and reload committed state when state is returned.
- [x] Persist idempotency by `(run, actor, authenticated session, local command
  ID)` and map session-local order IDs into a collision-free authoritative
  namespace.
- [x] Replace ad-hoc TUI operator JSON with shared typed request structures and
  remove unsupported replace/status actions from the advertised TUI surface.
- [x] Replace role audience debug formatting with an explicit stable role name.

Exit gate: deterministic barrier-driven native and D1 races publish once;
reconnect retries are idempotent; cross-session collision tests pass; public
serialization contains no actor, participant, order ownership, command,
correlation, reserve, or position data.

Implementation status 2026-07-21: the source slice is complete on
`codex/production-safety-slice`. Focused tests cover allowlisted serialization,
cross-session command/order separation, duplicate commit-race suppression,
typed TUI/server payload parity, unsupported-action removal, and stable role
names. The D1 migration was applied to SQLite and its composite uniqueness was
verified. A barrier-driven concurrent test against real D1 remains a release
gate because the repository's raw Workerd fixture deliberately returns 501 for
D1 rather than emulating it.

### Phase 2 - authentication

- [ ] Use short-lived signed claims containing actor, participant/team, run
  membership, role, session, capabilities, issue time, and expiry.
- [ ] Persist session issuance and revocation, bind session identity to the
  idempotency namespace, and apply per-actor and per-session limits before
  recovery.

### Phase 3 - bounded persistence and recovery

- [ ] Retain canonical append-only events, replace per-command `state_json`
  rewrites with compact mutable projections and periodic versioned checkpoints,
  and recover from a checkpoint plus a bounded event tail.
- [ ] Define active/completed run retention for command guards, events, orders,
  reports, snapshots, and checkpoints.
- [ ] Store sequence order without signed casts, using fixed-width decimal,
  high/low columns, or an explicit `i64::MAX` ceiling.
- [ ] Mark the whole-file origin backend development-only.

Exit gate: fault injection at every transaction boundary restores an identical
state hash, and checkpoint size and recovery time remain bounded at the declared
production load.

### Phase 4 - live publication and operations

- [ ] Rename the current finite response to `market.catchUp`, or implement a
  persistent stream with committed cursors, public/private projections,
  bounded queues, coalescing, slow-consumer disconnects, and reset recovery.
- [ ] Run ADR 0016's measured gate before adding a stream coordinator.
- [ ] Add structured run/session/command/sequence logs, commit and recovery
  latency histograms, duplicate/conflict counters, projection sizes, subscriber
  backlog, load/soak/restart tests, and operator runbooks.

### Phase 5 - empirical realism

- [ ] Declare a falsifiable fidelity profile, initially one lit continuous
  limit-order-book venue for US equities during regular trading hours.
- [ ] Version dataset license, venue, universe, dates, filters, hash,
  state-dependent intensities, size/placement distributions, seasonality,
  regimes, latency, fees, throttles, halts, auctions, RNG streams, parameters,
  holdouts, and tolerances.
- [ ] Validate spread/depth, arrivals, queue survival, queue-position fill
  probability, order-sign autocorrelation, return tails, volatility clustering,
  impact, and stress liquidity withdrawal across multiple seeds.
- [ ] Keep participant policies labeled prototype until private reports update
  inventory, remaining quantity, queue state, and future actions.

Persistence and realism work must not be built on known publication and identity
corruption paths. Phase 1 is the current implementation milestone.
