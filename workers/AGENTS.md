# Worker instructions

Workers are plain Cloudflare Workers. ADR 0020 authorizes FIX-session Durable Objects for outbound TCP/session recovery only; any other Durable Object role still requires its own accepted decision.

Use `workers-rs` official APIs. `apps/bunting-worker` owns bounded browser fetch/stream routing and outbound FIX sessions, and it calls Bunting application functions in process; the remaining directories under `workers/` are consumer scaffolds.

Workers Cache is mandatory for immutable checksum-addressed OrderBook-rs snapshot packages. Cache entries are recoverable accelerators, not locks or accepted-command journals.

Never rely on global memory or isolate affinity for correctness. Commit origin events before acknowledgement or stream publication.
