# Port note: Rust RITC market-making implementation

## Sources

- Existing implementation: `ref/ritc_mm/src/main.rs`
- Existing scenario duplicates: `ref/ritc_mm/app/src/main/resources/scenarios/`
- Pure strategy comparison: `joaquinbejar/market-maker-rs@36899f3e910997400bc95c3a8f3606776c002fbe`
- Adapter comparison: `nautechsystems/nautilus_trader@c28b1335c95abbf1bef2385def9a75a1b3862f76`
- Lifecycle/reconciliation comparison: `ref/quarcc-trading-engine`

## License status

No confirmed license was identified for `ref/ritc_mm`; do not copy or mechanically translate its implementation text until ownership and license are recorded. `market-maker-rs` is MIT. NautilusTrader is LGPL-3.0 and is used as an integration contract, not a source-vendoring target.

The scenario files under `ref/ritc_mm/app/.../scenarios` have the same Git blob SHAs as the five verified files under `ref/nbc_engine`. They are one provenance lineage and are catalogued in `docs/ports/nbc-scenario-catalog.md`.

## Architectural conclusion

The existing file is not a crate to port. It is a behavior inventory that must be split into three reusable boundaries and one host:

1. a native RIT transport and venue-model adapter;
2. pure market-making analytics and desired-quote generation;
3. protocol-neutral desired-versus-live order reconciliation;
4. a native host that composes those parts with canonical risk and account state.

None of those parts belongs in matching, ledger or the authoritative Durable Object runtime.

## Exact Bunting target layout

### Native venue adapter

```text
clients/ritc-adapter/
  AGENTS.md
  Cargo.toml                    # added only when implementation starts
  src/
    lib.rs
    config.rs                   # base URL, API key reference, retry/rate policy
    error.rs
    models/
      case.rs
      security.rs
      book.rs
      order.rs
      pnl.rs
    http/
      transport.rs              # trait plus reqwest implementation
      client.rs
      query.rs
    parse.rs                    # RIT payload -> normalized observations/reports
    clock.rs                    # injected native monotonic clock for timeouts only
    fixtures.rs
    bin/ritc-mm.rs              # optional native host, never a kernel dependency
  tests/
    fixtures/
    contract.rs
    reconnect.rs
```

Responsibilities:

- authentication and endpoint construction;
- async HTTP and optional venue WebSocket transport;
- request/response models and typed errors;
- retry, timeout and rate-limit policy;
- conversion from venue decimals into validated instrument units;
- venue snapshots, open orders, fills, position and P&L reports;
- recorded contract fixtures.

It does not calculate quotes, own authoritative position, infer fills from request success or enforce the final Bunting risk decision.

### Pure strategy analytics

```text
crates/market-making/
  AGENTS.md
  src/
    lib.rs
    observation.rs              # bounded normalized book/trade observation
    parameters.rs               # versioned model configuration
    volatility/
      ewma.rs                    # first implementation
      garch.rs                   # deferred
    imbalance.rs                # bounded queue/order-flow features
    avellaneda_stoikov.rs       # pure reservation/spread calculation
    inventory.rs                # skew and side suppression suggestions
    quote.rs                    # DesiredQuotes and exact intent conversion
    state.rs                    # warmup and estimator state
    snapshot.rs
    spectral.rs                 # optional and deferred
```

Responsibilities:

- consume explicit observations, inventory input and logical/host-supplied horizon;
- update bounded estimator state;
- calculate continuous or decimal quote proposals;
- clamp and round proposals into exact `PriceTicks` and `QuantityLots` intents;
- return typed errors and readiness state;
- snapshot and restore strategy-owned estimator state.

It has no sockets, Tokio, persistence, wall-clock reads, sleeps, global configuration, live order IDs or account mutation.

### Protocol-neutral reconciliation

```text
crates/order-reconciliation/
  AGENTS.md
  src/
    lib.rs
    ids.rs                      # client/local/venue ID mapping
    report.rs                   # normalized ack/reject/fill/cancel/snapshot reports
    state.rs                    # live order and pending-operation state
    transition.rs               # deterministic idempotent report application
    planner.rs                  # minimal desired/live diff
    action.rs                   # submit/cancel/replace/requery intents
    snapshot.rs
```

Responsibilities:

- local, client and venue order ID mapping;
- explicit pending-submit, live, partially-filled, cancel-pending, canceled, rejected and terminal states;
- duplicate and out-of-order report handling;
- desired-versus-live quote diffing;
- reconnect reconstruction from authoritative venue snapshots;
- stale order cleanup and kill-switch cancel planning;
- deterministic, bounded, snapshot-capable state transitions.

This crate is shared with native venue adapters and captures the valuable order-lifecycle/reconciliation behavior from the C++ QUARCC reference without its threads or callbacks.

### Risk and canonical state

Existing crates remain authoritative:

```text
crates/risk-engine/             final order admission and kill switch
crates/ledger/                  authoritative positions, cash and P&L
crates/market-events/           canonical commands and execution events
clients/ritc-adapter/           normalized external account reports only
```

A strategy may suppress a side because of inventory, but this is not a substitute for canonical position/notional/open-order limits. Every generated action passes the same risk path as a user order.

## Primitive-to-module map

| Existing primitive | Bunting destination | Port treatment |
|---|---|---|
| `CaseResp` | `clients/ritc-adapter/src/models/case.rs` | independently define from captured API payloads |
| `SecResp` | `models/security.rs` plus `parse.rs` | normalize position/security metadata; do not trust as canonical Bunting state |
| `BookLevel` / `BookResp` | `models/book.rs` | parse decimal strings/numbers, validate, bound depth |
| `OrderResp` | `models/order.rs` -> normalized reconciliation report | explicit status mapping and unknown-status rejection |
| `HistBar` | adapter model; optional estimator warmup input | bounded history only |
| `PnlResp` | adapter report/diagnostics | external comparison, not canonical ledger authority |
| HTTP client and headers | `http/transport.rs`, `http/client.rs` | rewrite async with injected credentials |
| polling loop | native host | replace sleeps with explicit cadence and cancellation token |
| GARCH state | `market-making/volatility/garch.rs` | defer; implement only after EWMA baseline |
| radix-2 FFT | `market-making/spectral.rs` | defer; use bounded iterative implementation only if measured |
| queue-rate estimator | `market-making/imbalance.rs` | rewrite as pure bounded state with golden vectors |
| Avellaneda-Stoikov | `market-making/avellaneda_stoikov.rs` | independently verify formula and adapt pure MIT comparison code if useful |
| inventory skew | `market-making/inventory.rs` | strategy suggestion; canonical limits stay in risk engine |
| quote clamps/rounding | `market-making/quote.rs` | side-aware exact tick conversion with tests |
| cancel/place logic | `order-reconciliation/planner.rs` | state-machine rewrite, not direct port |
| open-order tracking | `order-reconciliation/state.rs` | normalized venue reports and reconnect snapshot |
| constants | versioned configuration | no hidden global constants |

## Retained behavior

- typed RIT request/response models after validating the actual API contract;
- explicit strategy configuration and model version;
- reservation-price and spread decomposition;
- inventory skew and hard external controls;
- volatility and imbalance as separately testable estimators;
- quote distance/spread clamps;
- desired-versus-live quote reconciliation;
- warmup/readiness state;
- observability for proposed quotes, risk decisions and executions.

## Rejected behavior

- `reqwest::blocking` and synchronous polling in reusable code;
- embedded credentials and fixed localhost URLs;
- `thread::sleep` as strategy time;
- a one-file architecture;
- direct `f64` values at authoritative order boundaries;
- recursive allocation-heavy FFT in the polling loop;
- strategy-owned position or P&L truth;
- cancel-all/replace behavior without idempotent reconciliation;
- using strategy analytics inside the matching kernel;
- copying the duplicate NBC scenario files into a second lineage.

## Numeric policy

Authoritative orders use ticks and lots. Strategy analytics may use `rust_decimal` or carefully bounded floating-point calculations only behind this conversion contract:

1. validate all inputs and parameters are finite and in domain;
2. calculate a proposed continuous price/size;
3. apply model clamps;
4. apply documented side-aware rounding;
5. convert to exact `PriceTicks` and `QuantityLots`;
6. reject zero, overflow or invalid tick/lot output;
7. submit through normal canonical risk and matching.

Built-in deterministic simulation agents should prefer fixed/decimal formulations. Cross-platform floating results require native/Wasm golden-vector agreement or must be recorded as external strategy intents rather than recomputed during replay.

## Reference adoption decisions

### `market-maker-rs`

Use as a selective MIT source/test candidate, not a whole-crate dependency. Its default crate depends on OrderBook-rs, and importing that graph would pull concurrency and runtime assumptions into a pure strategy package.

Candidate material:

- `src/strategy/avellaneda_stoikov.rs`;
- `src/strategy/glft.rs` after the baseline;
- `src/strategy/interface.rs` as interface comparison;
- risk and execution boundaries;
- backtest fill cases as test ideas.

Any adapted code records exact paths/commit and strips unrelated execution, API and order-book dependencies.

### NautilusTrader

Use its adapter guide as a contract for layering and reconciliation. Do not vendor NautilusTrader. The Bunting Nautilus adapter and RITC adapter may share normalized connector/reconciliation concepts, but remain separate clients.

### QUARCC C++ engine

Use lifecycle, external ID mapping, deferred/out-of-order fill and kill-switch cases to strengthen `order-reconciliation`. Do not port callback dispatch, threads, gateway races, gRPC or SQLite.

## Implementation sequence

1. Create `clients/ritc-adapter`, `crates/market-making` and `crates/order-reconciliation` manifests only when the first compiling vertical slice is implemented.
2. Capture and document RIT REST payload fixtures.
3. Define transport and normalized report traits; implement a mock transport.
4. Implement typed RIT models and parsing with bounds.
5. Implement the reconciliation transition table and generated transition tests before live transport.
6. Implement a minimal desired-quote planner using exact static inputs.
7. Implement independently verified Avellaneda-Stoikov vectors.
8. Add exact tick/lot rounding and clamps.
9. Add EWMA volatility and simple imbalance; measure each independently.
10. Add native async HTTP transport and reconnect reconstruction.
11. Compose the native host with canonical risk and kill-switch behavior.
12. Evaluate GARCH only after the baseline is stable.
13. Evaluate spectral features last and retain them only with demonstrated incremental value.
14. Optionally expose the pure strategy to Bunting-native simulation agents under a separate model version.

## Required tests

### Adapter contract

- success and error payload parsing;
- missing/unknown fields according to documented compatibility policy;
- authentication injection without secret logging;
- timeout, retry classification and rate-limit behavior;
- bounded order-book/history payloads;
- reconnect and venue snapshot reconstruction.

### Strategy analytics

- independently calculated Avellaneda-Stoikov vectors;
- zero, negative, NaN and infinite parameter rejection where applicable;
- inventory sign moves reservation price in the expected direction;
- side-aware tick rounding never creates an unintended crossing;
- estimator warmup and snapshot/restore;
- bounded buffers and no per-tick unbounded growth;
- native/Wasm vectors for any built-in deterministic model.

### Reconciliation

- request success does not imply fill;
- partial fill updates remaining quantity and desired/live diff;
- duplicate acknowledgement/report is idempotent;
- fill-before-ack and cancel-reject sequences are handled explicitly;
- reconnect rebuilds live orders from the venue snapshot;
- stale quote creates a bounded cancel plan;
- kill switch prevents new submits and plans cancellation;
- ID collisions and unknown venue IDs reject or enter a documented quarantine state.

## Copy and dependency status

- Code copied from `ref/ritc_mm`: none; blocked by unresolved license.
- Code copied from `market-maker-rs`: none yet.
- Whole-crate `market-maker-rs` dependency: rejected.
- NautilusTrader vendoring: rejected.
- New Bunting manifests: not yet added; only target boundaries and scoped instructions are committed.
- Current use: API/behavior inventory, package decomposition, provenance and test planning.