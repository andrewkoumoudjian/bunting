# Rust reference map for planned work

ADR 0013 changes the central decision: OrderBook-rs is now the production kernel, not only an oracle.

## Production dependencies

| Package | Pin | Role |
|---|---|---|
| `orderbook-rs` | `0.10.3` | matching, order book, snapshots, replay helpers, risk hooks, lifecycle, metrics |
| `pricelevel` | `0.8.4` | transitive order/price-level domain used by OrderBook-rs |
| `worker` | `0.8.5` | plain Cloudflare Worker and Workers Cache API |
| `proptest` | dev/test only when reintroduced | generated command and recovery invariants |

## References no longer needed for a custom book

`slotmap` and `intrusive-rs` remain useful data-structure references, but they are not part of the production matching plan. Do not build a competing arena/FIFO book with them.

Liquibook remains an independent matching oracle. Exchange-core remains an accounting and atomic-command oracle. Neither replaces the upstream Rust dependency.

## Work mapping

### Worker kernel integration

Use:

- OrderBook-rs per-call result APIs;
- snapshot package JSON and checksum validation;
- typed rejects, kill switch, risk, lifecycle, expiry, and engine sequence;
- workers-rs Cache API patterns.

### Recovery and streaming

Use:

- OrderBook-rs snapshot/restore and replay helpers;
- Workers Cache immutable content-addressed entries;
- Bunting origin expected-version events;
- absolute L2 updates derived from upstream depth.

### Options

Use Option-Chain-OrderBook as the preferred composition reference. It already builds on OrderBook-rs and demonstrates hierarchy-wide expiry, sequencing, and per-call result attribution.

### Market making

Use direct OrderBook-rs depth, queue, impact, metrics, and placement helpers. Adapt pure formulas from market-maker-rs only after unit and dependency review.

### Binary protocols

Evaluate OrderBook-rs's `wire` feature first. Evaluate IronSBE core/schema/codegen second. Do not import IronSBE native transports or Tokio servers into the Worker.

### Scenarios

Rand and Proptest remain useful for explicit deterministic streams and generated invariants. ABIDES, NeXosim, and NBC assets remain scenario/model references.

## Rejected implementation directions

- a new Bunting `BTreeMap`/SlotMap matching engine;
- Durable Object ownership as a required runtime;
- Worker global memory as sole book state;
- Workers Cache as a lock or accepted-command journal;
- native thread/channel examples inside Worker execution;
- wholesale copying of market-maker-rs, matchbook, IronSBE transport, or Option-Chain-OrderBook into the equity kernel.
