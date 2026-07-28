# ADR 0022: Native competition venue and Cloudflare publication wrapper

- Status: Accepted
- Date: 2026-07-28
- Supersedes: ADR 0013's and ADR 0018's selection of one Cloudflare Worker as the primary deployment target
- Clarifies: ADR 0020's inbound TCP boundary

## Context

Bunting now has an implemented native server that accepts inbound FIX/TCP, while ADR 0020 says both that Bunting does not expose an inbound raw TCP listener and that no Worker route accepts inbound raw TCP. Those statements collapse the product and one deployment platform into the same boundary. Cloudflare Workers cannot accept inbound raw TCP, and carrying Worker bindings and browser-JS randomness configuration through reusable packages makes the native competition venue depend on a platform it does not need.

The competition platform requires a process that owns one run, accepts concurrent participant sessions and can be operated and recovered locally. Cloudflare remains useful for globally distributing immutable public artifacts, but its runtime constraints must not shape the market engine or native venue.

## Decision

The native Rust server is Bunting's primary competition venue. It may accept inbound FIX/TCP and owns the in-process application call into the single authoritative `bunting-engine`.

The engine and reusable packages are host-neutral. No crate under `packages/` may require the Cloudflare `worker` crate or a browser-JS randomness backend. Cloudflare-specific adapters live under `apps/bunting-worker`.

The Cloudflare Worker is a read-only publication wrapper for immutable run archives, leaderboards and public book snapshots. It does not accept participant commands, own origin truth or accept inbound raw TCP. The existing Worker command and outbound-FIX paths are transitional until the publication cutover is complete; documentation and validation must not describe that transition as finished before the code is removed.

## Consequences

Native development, testing and language bindings no longer compile Cloudflare bindings by default. Competition availability is no longer coupled to Worker TCP limitations or D1. The native venue must provide the durability, bounded concurrency and recovery behavior previously deferred to the hosted path.

The Worker loses its role as an alternate venue authority. Publication occurs only after the native venue commits authoritative state, and immutable content-addressed objects remain suitable for edge caching.

## Rejected alternatives

### Prohibit every inbound raw TCP listener

Rejected because ADR 0020's operational restriction applies to Worker routes, not to the native venue. The native server is explicitly an inbound FIX acceptor.

### Keep the Worker as a peer venue authority

Rejected because two authority paths would require cross-runtime parity for command ordering, recovery and scoring, while the competition needs one defensible run record.

### Keep Cloudflare dependencies in reusable packages

Rejected because feature unification and root Wasm flags make host-neutral code inherit a browser-JS deployment assumption.

## Validation

- `cargo tree --locked -p bunting-engine` contains no `worker` crate;
- `cargo check --locked -p bunting-engine --target wasm32-unknown-unknown` succeeds with the explicit unsupported host-entropy backend and without a browser-JS cfg;
- default workspace commands do not compile Cloudflare bindings;
- the Worker build runs from `apps/bunting-worker` with platform configuration scoped there;
- native FIX acceptance and Worker publication are tested independently;
- no Worker route accepts inbound raw TCP or mutates competition authority after publication cutover.

## Operational impact

Operators run the native venue for competition rounds and deploy the Worker only when public edge distribution is required. Existing Worker authority deployments need an explicit migration and rollback plan before their mutation routes are removed.

## Security impact

Participant credentials and mutation authority stay on the native venue. Published Worker data is immutable and public by construction, reducing the Worker credential and origin-store attack surface. Native listener authentication, per-session bounds and transport security remain mandatory.

## References

- [`0020-native-worker-browser-and-fix-boundaries.md`](0020-native-worker-browser-and-fix-boundaries.md)
- [`../architecture.md`](../architecture.md)
- [`../deployment.md`](../deployment.md)
- [Cloudflare TCP sockets](https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/)
- [Cloudflare Rust Workers](https://developers.cloudflare.com/workers/languages/rust/)
