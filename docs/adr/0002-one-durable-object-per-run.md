# ADR 0002: One authoritative Durable Object per market run

- Status: Superseded by ADR 0013
- Date: 2026-07-11
- Superseded: 2026-07-12

## Historical decision

The bootstrap architecture assigned one Durable Object to each run for sequencing, SQLite persistence, WebSocket ownership, and hot state.

## Replacement

Bunting now targets a plain Rust Cloudflare Worker. OrderBook-rs is restored from immutable Workers Cache snapshot packages with origin event/version fallback. Concurrency is controlled through an optimistic expected-version commit in the origin store, not a per-run Durable Object.

No implementation may reintroduce a Durable Object requirement without a new user-approved ADR.
