# OrderBook-rs example adoption map

Audited upstream revision:

```text
joaquinbejar/OrderBook-rs@575de34260b0fce346372074b6b938df058693a8
```

The examples are MIT-licensed. Bunting should normally call the demonstrated APIs rather than copy their surrounding demo program. When a test or fixture is closely adapted, its source path and commit must be recorded in the test header or fixture metadata.

## Example-by-example decision

| Example | What it demonstrates | Bunting decision |
|---|---|---|
| `prelude_demo` | `DefaultOrderBook`, IDs, limit orders, best bid/ask, spread, midpoint | Use as the smallest upstream smoke test; do not copy logging or `current_time_millis`. |
| `basic_orderbook` | broad order entry, lookup, market orders, crossing limits, cancellation, snapshots | Use as API inventory and integration-test donor. Replace generated IDs and wall clock with Bunting inputs. |
| `market_trades_demo` | market execution, `TradeListener`, `TradeInfo` | Prefer per-call `TradeResult` for request attribution; adapt trade-normalization tests. |
| `depth_analysis` | cumulative depth and target-depth queries | Call upstream methods directly for L2 and execution planning. |
| `market_metrics` | midpoint, spread, VWAP, microprice, imbalance | Call upstream metrics directly. Keep floating analytics out of canonical cash and quantity accounting. |
| `market_impact_simulation` | simulated fills, slippage, liquidity range | Call directly for pre-trade analytics; results are advisory, not acceptance authority. |
| `intelligent_order_placement` | queue ahead, tick placement, target queue position | Reuse directly in market-making strategy utilities. |
| `functional_iterators` | lazy depth traversal and early termination | Reuse directly; do not materialize duplicate Bunting depth structures. |
| `aggregate_statistics` | pressure, thin-book checks, depth distributions | Reuse as derived analytics. |
| `enriched_snapshots` | one-pass snapshots with selected metrics | Use for public snapshot responses when profiling supports it. |
| `trade_listener_demo` | synchronous callback notification | Use only at an adapter boundary. A callback cannot commit Bunting state recursively. |
| `trade_listener_channels` | native threads, channels, `BookManagerStd` | Do not copy into the Worker. The useful concept is symbol-aware routing; thread sleeps and channel processors are native-only. |
| `orderbook_snapshot_restore` | `create_snapshot_package`, JSON, checksum validation, restore | Adopt almost verbatim as the Workers Cache recovery flow, with Bunting cache keys and origin fallback. |
| `multi_threaded_orderbook` | eight-thread native access | Keep as upstream/native performance evidence; not Worker runtime code. |
| `orderbook_hft_simulation` | 30-thread stress simulation | Keep as an upstream benchmark and regression reference. |
| `orderbook_contention_test` | concurrent hot-level behavior | Run upstream when evaluating upgrades; do not port to Worker orchestration. |
| `price_level_debug` | low-level level state | Use for diagnostics and upstream bug reproduction only. |
| `price_level_transition` | price-level state transitions | Adapt invariant tests when a Bunting boundary depends on the behavior. |

## Additional high-value upstream examples

The current upstream tree also includes examples added after the original 19-example index.

### `risk_limits`

Adopt the configuration and typed-error pattern:

- `RiskConfig::new().with_*`;
- explicit account identity through `Hash32`;
- matching on `OrderBookError::RiskMaxOpenOrders`, `RiskMaxNotional`, and `RiskPriceBand`;
- verifying that a clean order succeeds after a rejection.

Bunting still owns participant cash, inventory, and cross-instrument portfolio limits that are outside the upstream book-level risk model.

### `kill_switch_drain`

Adopt the sequence verbatim at the behavior level:

1. engage the kill switch;
2. reject new order flow;
3. continue allowing cancellation;
4. drain through mass cancel;
5. release the switch only through an authorized operator command.

### `gtd_expiry_sweep`

Use host-supplied millisecond cutoffs. Never make Worker wall time an implicit replay input. Journal or persist the exact cutoff used by a sweep.

### `wire_roundtrip`

Evaluate later for compact market-data and order-entry frames. JSON remains the initial public protocol until the wire feature passes Wasm-size and compatibility review.

## Patterns safe to adapt closely

The following are strong copy/adaptation candidates under MIT attribution:

- checksum package serialize/validate/restore sequence from `examples/src/bin/orderbook_snapshot_restore.rs`;
- typed risk configuration and rejection assertions from `examples/src/bin/risk_limits.rs`;
- halt-and-drain assertions from `examples/src/bin/kill_switch_drain.rs`;
- trade normalization through `TradeInfo::from_trade_result` from `examples/src/bin/market_trades_demo.rs`;
- focused snapshot, partial-fill, deterministic mass-cancel, and replay tests from upstream `tests/`.

## Patterns not to copy into the Worker

- `std::thread`, `mpsc`, and sleep-based coordination;
- Tokio book managers or runtime timers merely to process matching;
- demo loggers and printing;
- ambient `current_time_millis` in canonical execution;
- UUID generation as a Bunting external order-ID policy;
- NATS, file journals, memory mapping, or native socket transports.
