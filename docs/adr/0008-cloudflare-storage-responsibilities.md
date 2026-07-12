# ADR 0008: Cloudflare storage and delivery responsibilities

- Status: Superseded in part by ADR 0013
- Date: 2026-07-11
- Superseded in part: 2026-07-12

## Retained decisions

- D1 or another origin store owns accepted-command history, expected versions, idempotency, run metadata, and canonical events.
- R2 owns large immutable exports and replay artifacts.
- KV owns read-heavy metadata that tolerates propagation delay.
- Queues perform idempotent derived work.
- Analytics and logs are operational, not canonical matching facts.

## Replacement decisions

Durable Object memory and Durable Object SQLite are no longer required.

Workers Cache is now mandatory for immutable checksum-protected OrderBook-rs snapshot packages. It remains non-transactional and is not used as a lock, idempotency table, accepted-command journal, or sole participant ledger.

Cache keys include run, instrument, committed event sequence, and checksum. Cache loss or eviction triggers origin recovery and repopulation.

See ADR 0013.
