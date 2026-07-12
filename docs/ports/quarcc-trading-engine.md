# Port note: C++ QUARCC trading engine

## Source

- Local source: `ref/quarcc-trading-engine`
- Bunting source snapshot: `bd90e7e9f7402bb3e7f5e1f085b8e291c016f798`
- Key paths:
  - `engine-cpp/include/trading/core/order_manager.h`
  - `engine-cpp/src/core/order_manager.cpp`
  - `engine-cpp/include/trading/core/trading_engine.h`
  - `engine-cpp/src/core/trading_engine.cpp`
  - `engine-cpp/include/trading/core/position_keeper.h`
  - execution, journal, order-store and feed interfaces under `engine-cpp/include/trading/interfaces/`
  - `engine-cpp/tests/unit/test_order_manager.cpp`

## License status

No repository-level license was identified during this audit. Until ownership and license are recorded, use this source only as an internal behavioral oracle. Do not copy or mechanically translate implementation text into Bunting.

## Architectural conclusion

The reference combines two concerns that Bunting must separate:

1. exchange-side matching, risk, ledger and canonical events;
2. participant/venue-side order lifecycle, identifier mapping and reconciliation.

The first belongs in the deterministic kernel. The second belongs in the new protocol-neutral `crates/order-reconciliation` boundary and native/protocol adapters. Neither inherits the C++ threads, callbacks, gRPC or native storage model.

## Retained behavior

- explicit order lifecycle states and invalid-transition rejection;
- typed correlation among client, local and external order IDs;
- duplicate and out-of-order execution handling;
- deferred reconciliation when venue messages arrive unexpectedly;
- kill-switch activation as durable state;
- position projection from executions;
- journal/store tests as command/event fixture sources;
- gateway and feed interfaces at adapter boundaries.

## Rejected behavior

- `std::jthread`, mutexes and cross-thread event queues in domain code;
- callback-driven recursive mutation;
- gRPC or sockets in the market kernel;
- native SQLite abstractions inside domain crates;
- wall-clock reads;
- floating-point prices or quantities;
- race recovery caused only by separate gateway/dispatch threads;
- one order manager owning both exchange matching and remote broker workflow.

## Implemented Rust compatibility port

`crates/quarcc-trading-engine` provides WASM-safe Rust enums, request and response records, market-data records, position records, and a transport-neutral `ExecutionService` trait matching the names and numeric discriminants of the legacy `quarcc.v1` surface. It contains no threads, sockets, filesystem access, SQLite dependency, ambient clock, or matching engine.

The records retain legacy floating-point fields because changing them would break existing generated clients. Those values are quarantined at the compatibility boundary and require checked fixed-point conversion before entering Bunting domain logic. Native gRPC code generation and a distributable Python wheel remain packaging work; the shared Rust contract passes `wasm32-unknown-unknown`.

## Exact Bunting mapping

| C++ concept | Bunting destination | Port form |
|---|---|---|
| legacy service records and names | `crates/quarcc-trading-engine` | behavior-derived WASM-safe compatibility contract |
| order command and exchange lifecycle events | `crates/market-events` | canonical versioned commands/events and stable rejection codes |
| exchange matching state | `crates/orderbook` | pinned `OrderBook-rs` adapter; no independent matcher |
| local/client/external ID map | `crates/order-reconciliation/ids.rs` | typed mapping with collision and idempotency rules |
| ack/reject/fill/cancel transitions | `crates/order-reconciliation/transition.rs` | explicit deterministic state machine |
| desired/live order diff | `crates/order-reconciliation/planner.rs` | bounded submit/cancel/replace/requery intents |
| execution gateway | native/FIX/NBC/RITC/Nautilus adapters | transport trait at client/host boundary |
| journal | `crates/origin-store` and Worker D1 adapter | canonical event stream with expected sequence and idempotency |
| order store | hot aggregate projection and snapshots | rebuilt from events; snapshot non-authoritative |
| position keeper | `crates/ledger` | checked execution-event projection |
| kill switch | `crates/risk-engine` and reconciliation planner | durable admission state plus deterministic cancel plan |
| feed registry | Worker subscriptions | non-authoritative fan-out after commit |
| protobuf/gRPC | protocol adapters only | no transport-generated types in canonical events |

## Reconciliation state requirements

At minimum model:

- pending submit;
- acknowledged/live;
- partially filled;
- cancel pending;
- replace pending when added later;
- canceled;
- rejected;
- fully filled;
- externally discovered after reconnect;
- quarantined unknown/collision state.

Every normalized venue report includes venue/source identity, report ID or deduplication key, client/local/external IDs when available, cumulative and last quantity, exact price, logical/received sequence metadata and reason code.

Request success never implies acknowledgement or fill. Duplicate reports are idempotent. Out-of-order reports produce a documented transition or quarantine result rather than hidden deferred callbacks.

## Implementation sequence

1. Inventory lifecycle enums, fields, rejection paths and C++ tests.
2. Write a language-neutral transition table.
3. Define typed IDs in `market-types`.
4. Define normalized venue reports and reconciliation actions.
5. Implement pure transition/replay logic in `order-reconciliation`.
6. Convert representative C++ test sequences into semantic JSON fixtures without copying source text.
7. Implement canonical order events and matching separately.
8. Implement kill-switch and limits in `risk-engine`.
9. Project executions into `ledger`.
10. Integrate reconciliation in RITC, FIX, Nautilus and other external adapters.

## Required tests

- submit, acknowledgement, partial fill, full fill and cancel paths;
- cancel versus fill ordering;
- fill before acknowledgement;
- duplicate external execution/report ID;
- unknown or colliding external ID;
- reconnect snapshot discovers missing live order;
- cancel reject returns order to the correct live state;
- kill switch blocks new submits and produces deterministic bounded cancel intents;
- replay and snapshot/restore yield identical reconciliation and ledger state;
- generated transition sequences never reach impossible states;
- no expected result depends on C++ thread scheduling.

## Differential-testing posture

The C++ source is an OMS/lifecycle reference, not the exchange matching oracle. Comparison is semantic: transitions, accepted/rejected actions, quantities, mappings and position deltas. Liquibook, OrderBook-rs and exchange-core remain the exchange-side matching/risk/accounting oracles.

## Copy status

- Code copied: none.
- Code translated: none; public field names and enum discriminants were behavior-derived.
- Target crate boundaries: `crates/quarcc-trading-engine` for compatibility and `crates/order-reconciliation` for future broker lifecycle state.
- Current use: WASM-safe shared Rust contract plus behavioral oracle for future native and Python packaging.
