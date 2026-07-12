# Edge API instructions

This package is the plain Rust Cloudflare Worker entrypoint.

- Do not route commands to a Durable Object.
- Use `bunting-orderbook`; never implement matching here.
- Use `bunting-worker-cache` for immutable upstream snapshot packages.
- Authenticate before exposing private routes or cache-derived private data.
- Mutating routes require idempotency and an expected origin version.
- Publish only committed events.
- Keep request, response, snapshot, subscription, and backlog sizes bounded.
