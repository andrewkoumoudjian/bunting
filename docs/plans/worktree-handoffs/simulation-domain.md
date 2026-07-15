# Simulation-domain worktree handoff

## Branch and base

- Branch: `codex/simulation-domain`
- Base: `codex/product-contract` at `b935000`
- Production matcher: `orderbook-rs = 0.10.3`, privately owned by
  `bunting-engine`.

## Completed requirement IDs

- `SD-01` scenario and run: strict versioned scenario input, canonical scenario
  hash inclusion, run/iteration initialization, stopped/active/paused/terminated
  lifecycle, lockstep/accelerated/paced logical clock modes, deterministic
  snapshot/replay equality, and reason-bearing administrator changes.
- `SD-02` market lifecycle and projections: GTC/IOC/FOK/GTD/DAY and the released
  post-only, iceberg, reserve, pegged, trailing-stop and market-to-limit matcher
  surface; listing halts, mass cancel, typed rejection, L1, aggregated/raw L2
  state, trade history, OHLC, time-and-sales, impact and participant-private
  projections.
- `SD-03` ledger and risk: per-currency settled/reserved/accrued/scheduled cash,
  positions and cost/P&L/fee/margin/NLV fields, balanced atomic journal postings,
  portfolio limits, shortability, buying power, gross/net groups,
  concentration/stress policy, and candidate-state rollback across matching,
  legacy reservations and portfolio state.
- `SD-04` instruments and workflows: economic instruments separate from
  listings; equity, currency, bond, option, future, commodity and synthetic
  definitions; scheduled cashflows and lifecycle actions; tenders, bilateral
  OTC, composites, capacity-constrained facility jobs, scoped news and
  deterministic scoring/ranking reports.
- `SD-05` built-in agents: seven separately implemented Bunting-native policies
  (noise, liquidity, informed, momentum, spiking, institutional and options
  flow), named deterministic RNG streams, serializable policy recovery state,
  mandatory QUARCC participant commands, provenance/units metadata, golden
  vectors and distribution checks.

## Provenance and unresolved formulas

- Matcher behavior uses only the released OrderBook-rs API authorized by ADRs
  0018 and 0019. Bunting wraps the API; no second matching authority or engine
  reference is exposed.
- NBC-compatible workflow concepts are based on the authorized evidence named
  by ADR 0017 and `docs/ports/nbc-behavior-evidence.md`. This change does not
  claim byte-for-byte equivalence beyond recorded differential evidence.
- RIT source remains prohibited to copy. Exact hidden RIT valuation, margin,
  scoring, tender, agent intensity, news timing, facility conversion and
  exercise/delivery formulas remain unresolved. The implemented formulas are
  explicitly versioned `Bunting-native v1` policies and do not claim RIT
  compatibility.
- Paced mode records deterministic logical-time state; wall-clock scheduling is
  an application concern and is deliberately outside the authoritative state.

## Changed APIs

- `bunting-market-events` adds the versioned simulation command/event types,
  order time-in-force and special-order policies, plus a separate
  `SimulationCommandRequest` envelope. It does not enlarge the participant
  `CommandPayload`, so Worker/FIX participant routing remains unchanged.
- `RunState::transition_simulation` is the authoritative administration entry
  point. `RunState::transition` remains the participant order entry point and
  enforces active-run and listing-halt checks for full simulation scenarios.
- `ScenarioDefinition::with_simulation` binds full simulation configuration
  into the canonical scenario hash. `RunState::simulation` exposes immutable
  projections and recovery state.
- `PortfolioLedger` and `check_portfolio` provide the exact ledger and
  portfolio-risk boundaries. The legacy reservation ledger remains the
  matching transaction component during migration.
- `bunting-api-contract::SIMULATION_DOMAIN_PROCEDURES` records the product
  procedure inventory without claiming deployed Worker routes.

## Tests and verification

- `cargo check --locked --workspace --all-targets` passed.
- `cargo test --locked -p bunting-engine --test simulation_domain` passed all
  lifecycle, replay, workflow, fixture and OrderBook-rs policy tests.
- Package tests cover balanced-posting rollback, hard-versus-penalty portfolio
  risk, agent golden vectors/RNG recovery/distribution behavior, and contract
  procedure classification.
- The final root metadata, format, clippy, workspace test, dependency-pin, Wasm
  and whitespace gates are recorded in the completion message.
