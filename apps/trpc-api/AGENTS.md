# Edge API instructions

This package is the plain Rust Cloudflare Worker entrypoint.

- ADR 0016 requires direct tRPC fetch dispatch; do not use `worker::Router` or expose REST resources.
- Do not route commands to a Durable Object. A Rust `RunStreamCoordinator` may coordinate committed subscriptions only after the ADR 0016 gate.
- Use `bunting-engine`; never implement matching or mutate engine state here.
- Use `bunting-worker-cache` for immutable upstream snapshot packages.
- Authenticate before exposing private procedures or cache-derived private data; derive participant identity from verified claims.
- Mutating procedures require idempotency and an expected origin version.
- Publish only committed events.
- Keep request, response, snapshot, subscription, and backlog sizes bounded.
