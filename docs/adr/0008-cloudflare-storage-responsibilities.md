# ADR 0008: Explicit Cloudflare storage and delivery responsibilities

- Status: Accepted
- Date: 2026-07-11

## Context

Cloudflare offers several storage and delivery products with different consistency, latency and size characteristics. Using them interchangeably would create correctness bugs. Live trading state requires a single authoritative sequencer and transactional recovery, while global discovery, immutable artifacts, caching and analytics have different needs.

## Decision

Assign one responsibility to each platform primitive.

## Durable Object memory

Use for active run state:

- books;
- ledgers;
- risk views;
- agent state;
- logical clock;
- connection indexes;
- hot projections.

Memory is an accelerator and can disappear on reconstruction.

## Durable Object SQLite

Authoritative within a live run:

- canonical event batches;
- snapshots;
- idempotency records;
- scheduled-event status;
- FIX session state;
- FIX outbound resend journal;
- compact run metadata needed for recovery.

## D1

Control-plane relational data:

- tenants and users;
- scenario metadata and immutable version indexes;
- run directory and ownership;
- strategy metadata;
- completed result and leaderboard indexes;
- administrative audit indexes.

D1 is not queried in the matching hot path.

## R2

Large immutable or append-produced artifacts:

- replay archives;
- historical event chunks;
- strategy source bundles by content hash;
- exported datasets;
- large logs and diagnostics;
- completed snapshot bundles.

## KV

Read-heavy non-authoritative metadata that tolerates delayed propagation:

- public feature flags;
- public allowlists;
- cached discovery metadata;
- version lookup hints.

KV never owns live orders, balances, positions, run clocks or exact rate counters.

## Workers Cache

Cache only public, safe, versioned GET responses:

- instrument definitions by version;
- published scenario versions;
- completed public results;
- immutable replay chunks;
- static assets.

Never cache mutating responses, authentication, private account data, active book authority or WebSocket upgrades. Prefer normal cache headers and tiered caching behavior; use the Cache API only where programmatic control is required.

## Queues

At-least-once asynchronous derived work:

- replay export;
- result aggregation;
- notification delivery;
- analytics derivation;
- log compaction.

Every consumer is idempotent. Queue completion never determines whether an order was accepted.

## Analytics Engine and logs

Operational telemetry only. Canonical financial and matching facts remain in the event journal.

## Consequences

Positive:

- correctness boundaries are explicit;
- eventual consistency cannot silently affect matching;
- performance tuning can target each workload;
- recovery and audit paths are clear.

Negative:

- data projections and export pipelines are required;
- developers must understand multiple products;
- cross-run queries are eventually updated from run-local truth.

## Validation

- architecture tests forbid prohibited dependencies in hot-path crates;
- duplicate Queue delivery is harmless;
- cache tests prove private and mutable endpoints are not cached;
- loss of KV, Cache or analytics does not change run outcomes;
- Durable Object reconstruction succeeds without D1 or KV reads in the matching path.

## References

- Durable Object SQLite: https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/
- KV consistency model: https://developers.cloudflare.com/kv/concepts/how-kv-works/
- Cache limitations: https://developers.cloudflare.com/workers/cache/limitations/
- Queue delivery guarantees: https://developers.cloudflare.com/queues/reference/delivery-guarantees/
