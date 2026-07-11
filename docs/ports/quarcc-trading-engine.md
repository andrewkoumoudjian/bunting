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

## What the reference establishes

The engine contains a useful separation between order-management behavior and external services:

- an order manager sequences lifecycle changes;
- an execution service sends orders and returns acknowledgements/fills;
- local and broker identifiers are mapped;
- fills may arrive before expected acknowledgement state and are deferred;
- an order store and journal preserve lifecycle information;
- a position keeper projects executions;
- a kill switch changes admission and cancellation behavior;
- market feeds and protobuf/gRPC interfaces sit outside core order state.

These are valuable business invariants even though the runtime architecture is not suitable for Bunting.

## Retain

- explicit order lifecycle states and invalid-transition rejection;
- correlation between client/local/external order IDs;
- duplicate and out-of-order execution handling as protocol concerns;
- deferred reconciliation when external messages arrive in an unexpected order;
- kill-switch activation as durable state, not a transient boolean;
- position projection from executions rather than direct position mutation;
- journal/store test cases as sources for command/event fixtures;
- clean gateway and feed interfaces at adapter boundaries.

## Reject

- `std::jthread`, mutexes and cross-thread event queues in the domain model;
- callback-driven recursive state mutation;
- gRPC or sockets inside the market kernel;
- native SQLite abstractions inside domain crates;
- direct wall-clock reads;
- floating-point prices or quantities;
- recovery logic whose only purpose is reconciling races between gateway and dispatch threads;
- a single order manager owning both exchange matching and remote broker workflow.

Bunting's simulated exchange and a participant-side order-management client are distinct systems. The authoritative matching kernel should not inherit broker OMS responsibilities.

## Bunting mapping

| C++ concept | Bunting destination | Port form |
|---|---|---|
| order lifecycle model | `market-events`, `matching-engine` | behavior-derived state machine and stable rejection codes |
| local/broker ID map | protocol/client adapters | typed mapping table with explicit idempotency and reconciliation |
| execution gateway | native/FIX/NBC adapters | trait at the host/client boundary; never a kernel dependency |
| journal | Durable Object event append | canonical event stream with expected sequence |
| order store | hot aggregate projection plus snapshots | rebuilt from events; snapshot is non-authoritative |
| position keeper | `ledger` | execution-event projection with checked fixed-point arithmetic |
| kill switch | `risk-engine` | event-sourced participant/run state |
| feed registry | Worker stream subscriptions | non-authoritative fan-out after commit |
| protobuf/gRPC | protocol adapters only | no generated transport types in canonical events |

## Implementation sequence

1. Inventory lifecycle enums, order fields, rejection paths and tests.
2. Write a language-neutral transition table before Rust code.
3. Convert representative C++ test sequences into JSON fixtures containing commands and expected semantic outcomes.
4. Implement typed IDs and lifecycle events in `market-types` and `market-events`.
5. Implement pure transition logic without a gateway, journal or database.
6. Implement kill-switch and limit checks in `risk-engine`.
7. Project fills into `ledger`.
8. Add protocol-specific reconciliation only in clients/adapters.

## Required tests

- valid submit, accept, partial fill, full fill and cancel paths;
- cancel versus fill ordering;
- duplicate external execution ID;
- fill before external acknowledgement in an adapter reconciliation test;
- local/external ID collision rejection;
- kill switch blocks new orders and emits deterministic cancel intents for existing orders;
- replay yields identical lifecycle, position and hash state;
- generated transition sequences never reach an impossible order state;
- no C++ thread scheduling assumption appears in expected results.

## Differential-testing posture

Because the C++ engine is an OMS rather than a complete exchange matching engine, differential comparison is semantic, not byte-for-byte. Fixtures should assert state transitions, accepted/rejected actions, quantities and position deltas. Liquibook, OrderBook-rs and exchange-core are better oracles for exchange-side matching behavior.

## Copy status

- Code copied: none.
- Code translated: none.
- Current use: behavioral inventory and future golden-fixture source.
