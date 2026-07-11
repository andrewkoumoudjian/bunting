# ADR 0009: Checked fixed-point numerics for market correctness

- Status: Accepted
- Date: 2026-07-11

## Context

The C++, Java scenario and Rust RITC references use floating-point values for prices, quantities, parameters or P&L. Binary floating point can introduce non-associative arithmetic, invalid tick comparisons, platform-dependent rounding and replay differences. A matching engine needs exact price ordering and accounting conservation.

Some statistical agent models legitimately use floating point internally. Their outputs still must cross a deterministic quantization boundary before becoming market commands.

## Decision

Use integer fixed-point domain types for all correctness-critical market state:

- `PriceTicks(i64)`;
- `QuantityLots(i64)`;
- `MoneyMinor(i128)`;
- `LogicalTimeNs(u64)`;
- `EventSequence(u64)`;
- integer basis points and scaled probabilities where practical.

Instrument metadata defines tick size, lot size, currency minor-unit scale and any display scale. API decimal strings are parsed and quantized at the boundary. Values not exactly representable under the instrument definition are rejected with stable reason codes unless the endpoint explicitly defines a rounding policy.

All arithmetic is checked. Overflow, underflow, division by zero and invalid sign transitions are explicit errors, never wrapping behavior.

Accounting multiplication uses widened intermediates and documented rounding. Fees and mark-to-market policies are versioned.

Agent models may use deterministic floating-point calculations only in isolated model crates when necessary. Before producing an order they must:

1. validate finite values;
2. apply documented clamps;
3. quantize to integer ticks/lots with a versioned rule;
4. record relevant model parameters;
5. produce only fixed-point commands.

Floating-point model calculations are not used for book ordering, fill allocation, positions, cash, fees, risk limits or scoring totals.

## Consequences

Positive:

- exact tick and lot validation;
- deterministic ordering and replay;
- easier conservation proofs;
- clear external decimal conversion;
- reduced cross-language ambiguity.

Negative:

- conversion code is more explicit;
- very large notionals require careful scale design;
- statistical models need quantization boundaries;
- UI and SDKs must not assume JSON numeric precision is sufficient.

## Rejected alternatives

### `f64` everywhere

Rejected because it is unsuitable for authoritative price and money equality.

### Arbitrary-precision decimal everywhere

Rejected for the hot path because it increases size and cost; exact integer units are sufficient after instrument quantization.

### Silent rounding at every boundary

Rejected because it hides invalid client inputs and can create unexpected priority changes.

## Validation

- property tests prove cash and quantity conservation;
- tick/lot conversion golden tests cover positive, negative, boundary and overflow cases;
- serialization uses strings or safely bounded integers where JSON precision could be lost;
- lints flag floating-point arithmetic in correctness-critical crates;
- agent quantization is versioned and tested.

## References

- `crates/market-types`
- `ref/quarcc-trading-engine/engine-cpp/src/core/order_manager.cpp`
- `ref/ritc_mm/src/main.rs`
