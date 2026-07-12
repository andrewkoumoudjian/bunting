# Worker instructions

Workers are plain Cloudflare Workers. Do not add a Durable Object binding without a new user-approved ADR.

Use `workers-rs` official APIs. `apps/edge-api` owns HTTP/WebSocket routing and calls Bunting adapters; the remaining directories under `workers/` are consumer scaffolds.

Workers Cache is mandatory for immutable checksum-addressed OrderBook-rs snapshot packages. Cache entries are recoverable accelerators, not locks or accepted-command journals.

Never rely on global memory or isolate affinity for correctness. Commit origin events before acknowledgement or stream publication.
