# Port note: Rust RITC market-making implementation

## Sources

- Existing implementation: `ref/ritc_mm/src/main.rs`
- Comparison source: `joaquinbejar/market-maker-rs@36899f3e910997400bc95c3a8f3606776c002fbe`
- Adapter comparison: `nautechsystems/nautilus_trader@c28b1335c95abbf1bef2385def9a75a1b3862f76`

## License status

No confirmed license was identified for `ref/ritc_mm`; do not copy or mechanically translate it until ownership and license are recorded. `market-maker-rs` is MIT. NautilusTrader notices and license requirements must be preserved for any adapted material.

## Current implementation inventory

The existing monolithic binary contains:

- RIT REST response types;
- blocking HTTP client and API key handling;
- polling loop and wall-clock sleeps;
- Avellaneda–Stoikov reservation price and spread logic;
- GARCH(1,1) volatility state;
- radix-2 FFT and spectral order-flow analysis;
- queue-rate/imbalance estimation;
- inventory and position limits;
- quote placement, cancellation and reconciliation;
- calibration constants and clamps;
- P&L and open-order inspection.

This is useful research code, but it mixes transport, time, strategy math, risk and order lifecycle in one process.

## Retain

- typed RIT request/response models after validating the actual API contract;
- explicit strategy configuration instead of hidden magic constants;
- reservation-price and spread decomposition;
- inventory skew and hard position limits;
- volatility, order-flow and queue signals as separately testable components;
- quote distance and spread clamps;
- desired-versus-live quote reconciliation;
- warmup state and calibration fixtures;
- observability for proposed quotes, risk decisions and execution outcomes.

## Reject or redesign

- `reqwest::blocking` and synchronous polling in reusable code;
- embedded API credentials and fixed localhost URLs;
- `thread::sleep` as strategy time;
- one-file architecture;
- direct `f64` conversion into authoritative order prices and quantities;
- recursive heap-allocating FFT in a hot polling loop;
- assuming theoretical queue parameters are calibrated because constants exist;
- strategy-owned position truth;
- cancel-all/replace behavior without idempotency and reconciliation state;
- using this strategy implementation inside the authoritative matching kernel.

## Proposed package split

### Native connector

Create a native-only `ritc-adapter` client or place it under a general connector crate:

- async HTTP transport;
- typed endpoints and errors;
- authentication injected from configuration;
- explicit retry and rate-limit policy;
- no strategy logic;
- mock transport and recorded fixtures;
- optional WebSocket support if the RIT environment exposes it.

This crate may use Tokio and native TLS. It is not part of the Worker/Wasm kernel.

### Strategy analytics

Create pure modules for:

- mid-price and book observation normalization;
- volatility estimator;
- queue/imbalance estimator;
- optional spectral feature estimator;
- Avellaneda–Stoikov quote model;
- inventory skew;
- quote clamps and tick rounding.

Each module consumes explicit inputs and returns values or typed errors. No module reads time, network, position or global configuration.

### Strategy state machine

A strategy instance owns:

- configuration version;
- estimator state;
- last observation sequence;
- desired bid/ask intents;
- outstanding client-order IDs;
- last reconciliation result;
- explicit warmup/readiness state.

It receives account/order state from the connector or Bunting events. It never declares an order filled solely because a request succeeded.

### Risk and reconciliation

Risk remains outside the quote formula:

- maximum absolute position;
- maximum order/notional size;
- stale-data halt;
- maximum open orders;
- drawdown or loss limits where supported;
- kill switch.

The order manager computes a minimal diff between desired quotes and live orders. Reconciliation must handle partial fills, rejected cancels, duplicate responses, delayed acknowledgements and reconnects.

## Numeric policy

Authoritative Bunting orders are expressed in ticks and lots. Strategy analytics may use decimal/floating calculations only behind an explicit conversion boundary:

1. validate all inputs are finite;
2. calculate a proposed continuous price/size;
3. apply documented side-aware rounding and clamps;
4. convert to exact `PriceTicks` and `QuantityLots`;
5. pass through normal risk and matching.

For deterministic built-in simulation agents, prefer fixed-point/decimal formulations or record the resulting intents as canonical events. Cross-platform `f64` results must not be assumed byte-identical without golden-vector verification for the chosen Wasm/native targets.

The FFT is optional. It should not enter the first port. First demonstrate incremental, allocation-bounded value over simpler imbalance and volatility signals.

## Market-maker-rs borrowing plan

High-value comparison areas:

- `src/strategy/avellaneda_stoikov.rs` for formula decomposition and typed errors;
- `src/strategy/interface.rs` for strategy interfaces;
- risk modules for limit/circuit-breaker separation;
- execution modules for connector/order-manager boundaries;
- backtest fill models for test scenarios.

Do not adopt its entire feature surface, concurrency model, metrics stack or decimal type automatically. Extract the smallest pure interfaces and preserve MIT attribution for any copied code.

## Implementation sequence

1. Record the RIT API contract and example payloads.
2. Move response/request models into a connector module with fixture tests.
3. Extract Avellaneda–Stoikov calculation into a pure function with golden vectors.
4. Extract inventory skew, volatility and imbalance one at a time.
5. Add exact tick/lot conversion and side-aware rounding tests.
6. Implement a desired-quote model independent of live order IDs.
7. Implement reconciliation against a mock connector.
8. Add position, stale-data and kill-switch controls.
9. Add the native RIT connector.
10. Integrate the same pure strategy with Bunting-native events if desired.
11. Evaluate GARCH and spectral features only after a baseline strategy is measured.

## Required tests

- published or independently calculated Avellaneda–Stoikov vectors;
- zero/negative/NaN/infinite parameter rejection;
- gamma and intensity edge cases;
- inventory sign moves reservation price in the expected direction;
- tick rounding never crosses an unintended price boundary;
- position limit disables the risk-increasing side;
- stale data disables quoting;
- partial fill causes correct desired/live reconciliation;
- duplicate acknowledgement is idempotent;
- reconnect reconstructs outstanding-order state from the venue;
- estimator warmup and snapshot/restore;
- bounded buffers and no per-tick unbounded growth;
- optional native-versus-Wasm deterministic vectors for built-in agents.

## Copy status

- Code copied from `ref/ritc_mm`: none.
- Code copied from `market-maker-rs`: none.
- Current use: architecture decomposition, formula inventory and test-plan source.
