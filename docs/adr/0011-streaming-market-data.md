# ADR 0011: Authoritative streaming market-data protocol

- Status: Accepted
- Date: 2026-07-11

## Context

Bunting must stream L1/L2 books, trades and private execution/account projections with low latency while preserving deterministic recovery and Cloudflare Durable Object constraints.

The authoritative `MarketRun` object already owns the committed book and canonical event order. Introducing a second independently maintained streaming book would create eventual-consistency and recovery risk. At the same time, WebSocket clients can be slow, Durable Objects may hibernate and in-memory fan-out buffers are not durable.

The protocol therefore needs explicit answers for snapshots, deltas, cursors, hibernation, batching, backpressure and reconnect.

## Decision

### Ownership

The initial `MarketRun` Durable Object is the authoritative WebSocket publisher for its run. The Edge API authenticates the upgrade and routes it to the object. A separate fan-out tier is deferred until measurements justify it.

The object accepts sockets with the Durable Object Hibernation WebSocket API.

### Publication point

Only committed event batches may produce stream records. Publication happens after the SQLite transaction succeeds and the committed events are applied to hot state.

### Protocol shape

The first protocol is versioned JSON over WebSocket. Binary encoding is deferred until fixtures and profiling exist.

A socket may subscribe to multiple authorized channels. Initial book channels are `book.l1:<instrument>` and `book.l2:<instrument>` with a bounded requested depth.

The server sends either:

1. a successful resume from the current bounded projection ring; or
2. `stream.reset` followed by current channel snapshots.

Every subsequent `stream.batch` contains:

- protocol version;
- run ID;
- activation-local `stream_epoch`;
- monotonic activation-local `record_id`;
- highest canonical `EventSequence` reflected;
- one or more channel messages.

### Book representation

L2 is aggregated price-level depth. Public L3/order-by-order depth is not included initially.

L2 delta entries contain side, exact `PriceTicks` and the **absolute resulting `QuantityLots`** at that visible level. Zero removes the level. L1 carries the complete current best bid/ask projection.

Snapshots and selected deltas include a deterministic checksum over the visible sorted projection.

### Cursor and recovery model

`EventSequence` remains authoritative and durable. `stream_epoch`/`record_id` are derived delivery cursors backed by a bounded in-memory projection ring.

A new object activation creates a new stream epoch. If a client presents a retained cursor from the current epoch, records may be replayed. Otherwise the server sends `stream.reset` and fresh snapshots.

The ring is not written as a second durable event log. Long-range audit/recovery uses canonical events or replay archives.

### Hibernation

A socket attachment stores only small connection metadata:

- identity/role reference;
- authorized subscriptions;
- protocol version;
- last acknowledged stream cursor;
- optional FIX session reference.

After hibernation/re-activation, the projection ring may be absent. The object initializes a new epoch and sends a reset/snapshot before new deltas to affected connections.

### Batching

Logical messages are batched by committed command/event batch and bounded frame byte/count limits. No periodic timer is used solely to flush market data because scheduled callbacks prevent hibernation.

### Backpressure

Clients acknowledge the highest fully applied stream record. The server bounds unacknowledged record count and bytes per connection.

- L1/L2, status and bars may be coalesced to their latest absolute state.
- Trades may be batched but are not silently removed.
- Private orders, executions, positions, account and risk records are never silently dropped.
- Persistent slow consumers receive a typed warning and are disconnected with recovery metadata.

The server does not treat a successful `send()` call as proof that the client applied the message.

## Consequences

Positive:

- streamed state comes directly from the committed authoritative book;
- absolute L2 changes are idempotent and coalescible;
- hibernation and activation loss have an explicit reset path;
- no second durable projection log is required;
- slow-client behavior is deterministic and bounded;
- private records cannot disappear silently.

Negative:

- the run object initially pays fan-out CPU;
- resume across activation generally requires a resnapshot;
- application-level acknowledgements add protocol complexity;
- high socket counts may eventually require a derived fan-out tier.

## Rejected alternatives

### Publish before persistence

Rejected because clients could observe orders/trades that are not durable.

### Use incremental quantity deltas

Rejected because coalescing/retransmission becomes order-sensitive and less robust.

### Treat WebSocket reliability as sufficient recovery

Rejected because disconnect, object activation and application-level slow consumers still require snapshots and cursors.

### Persist every projection frame

Rejected initially because canonical events already provide truth and projection persistence duplicates storage and migration burden.

### Timer-based periodic fan-out loop

Rejected because timers prevent WebSocket hibernation and are unnecessary when committed event batches provide natural flush points.

### Separate eventually consistent streaming book

Rejected until a measured fan-out bottleneck justifies the consistency and recovery complexity.

## Validation

- snapshot plus following deltas equals a current direct snapshot;
- absolute changes are idempotent;
- current-epoch resume and resume miss both recover;
- hibernation/activation reset precedes new deltas;
- checksums match native and Wasm builds;
- unauthorized channels reject;
- bounded-depth and frame limits reject oversized requests;
- public coalescing preserves latest book state;
- private records are either delivered in order or the socket is closed with a usable recovery cursor;
- slow consumers cannot cause unbounded memory growth.

## References

- Cloudflare Durable Objects WebSocket Hibernation documentation
- `ref/workers-rs` Durable Object and WebSocket examples
- `docs/core-implementation-questions.md`
