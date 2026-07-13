# ADR 0011: Streaming market-data protocol

- Status: Superseded in ownership details by ADR 0013 and transport details by ADR 0016; protocol semantics retained
- Date: 2026-07-11
- Updated: 2026-07-12

## Retained protocol decisions

- versioned JSON first;
- snapshot followed by absolute L1/L2 updates;
- zero quantity removes a level;
- every frame identifies the highest committed Bunting event sequence represented;
- public book/status data may coalesce;
- trades and private execution/account records cannot be silently dropped;
- bounded subscriptions, depths, frames, and backlog;
- slow-consumer disconnect with a usable recovery cursor;
- publish only after the origin commit succeeds.

## Replacement ownership model

A plain Cloudflare Worker accepts WebSockets. No Durable Object owns the connection or book.

Resume is based on committed event sequence and origin recovery. It never depends on a Durable Object activation epoch or an isolate-local projection ring. On an unavailable tail, the Worker sends `stream.reset` and a current OrderBook-rs snapshot, normally recovered through Workers Cache.

OrderBook-rs `engine_seq` may be included for book-event diagnostics. Bunting event sequence remains the durable cross-component cursor.

See ADR 0013.
