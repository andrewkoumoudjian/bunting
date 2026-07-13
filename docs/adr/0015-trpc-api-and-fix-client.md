# ADR 0015: Public tRPC API with client-side FIX compatibility

- Status: Accepted
- Date: 2026-07-12
- Supersedes: ADR 0004; the REST/WebSocket transport selections in ADR 0006 and ADR 0011
- Retains: ADR 0011 committed-sequence, reset, coalescing and backpressure semantics

## Context

Bunting needs one typed public API for browsers, SDKs, automated participants and protocol adapters. The current Rust Worker exposes provisional REST routes, while ADR 0004 specifies a separate raw-FIX-over-WebSocket endpoint. Those surfaces duplicate authentication, command validation, recovery and streaming behavior, and raw FIX transport makes the server own a second public session protocol.

tRPC provides the intended TypeScript router and client boundary. A Rust server that only imitates tRPC's wire format would not provide router-derived client types and would create an unofficial protocol implementation. The authoritative market transaction, ledger, origin commit and recovery path must remain in Rust.

FIX compatibility is a participant-side concern. Standard FIX applications can connect to a local bridge that owns the FIX session and uses the same typed Bunting client as every other participant.

## Decision

The only public application API is a versioned tRPC router hosted by a plain TypeScript Cloudflare Worker at `apps/trpc-api`. The initial deployment composes two ordinary Workers without a Durable Object:

```text
browser / SDK / strategy -----------+
                                     +--> public tRPC Worker
FIX initiator -> local FIX bridge --+          |
                                                | private Service Binding
                                                v
                                     authoritative Rust Worker
                                     -> command transaction
                                     -> origin commit
                                     -> immutable cache publication
```

The public Worker owns tRPC routing, public authentication, input schemas, transport bounds and router-derived client types. It holds no market authority and cannot acknowledge a mutation until the Rust Worker returns a committed result.

The Rust Worker remains the authoritative application service. Its current REST handlers become a private service-binding protocol during migration. They are not a supported public API, and the binding authenticates the calling Worker independently of participant credentials. Participant identity comes from verified public authentication claims rather than a caller-selected participant header.

The router is versioned as `bunting.v1`. Initial procedure families are:

- `system.health` for deployment and dependency identity;
- `runs.*` and `instruments.*` for capability-aware discovery and provisioning as those functions are implemented;
- `orders.submit`, `orders.cancel`, `orders.replace` and `orders.get`, with unsupported engine capabilities rejected explicitly;
- `market.snapshot` and `accounts.snapshot` for recovery;
- `market.subscribe` and `accounts.subscribe` for committed public and private streams.

Every mutation carries `commandId`, `correlationId`, `expectedSequence` and `logicalTimeNs`. Identifiers, event sequences and integer values wider than JavaScript's safe range cross the API as validated decimal strings. tRPC errors include a stable Bunting error code and retry classification in structured error data. Command mutations are sent as individual calls; tRPC request batching is bounded and is not an atomic-command mechanism.

Subscriptions retain ADR 0011 semantics: every item identifies the highest committed Bunting event sequence represented; public depth may coalesce; trades and private reports cannot disappear silently; slow clients receive a usable recovery cursor; and an unavailable tail produces a typed reset followed by a snapshot. The selected tRPC subscription transport must work on plain Cloudflare Workers and is pinned with the router dependency before implementation.

Ship a typed TypeScript client from the router type under `clients/typescript-sdk`. Browser, SDK, Nautilus and compatibility adapters use that client or a language-specific client proven against the same contract fixtures; they do not call private Rust routes.

Ship FIX compatibility under `clients/fix-bridge` as a local/native client bridge. It:

- accepts standard FIX/TCP locally;
- owns Logon, Logout, Heartbeat, Test Request, resend, gap-fill and sequence-reset state in a bounded durable local session store;
- maps supported application messages to tRPC procedures;
- maps committed Bunting results and private stream items to Execution Report, Cancel Reject, Business Message Reject and market-data messages;
- keeps FIX session sequence numbers distinct from Bunting event sequences;
- preserves client order IDs, correlation and duplicate-command behavior explicitly;
- never forwards raw FIX frames to the Worker or mutates market state outside ordinary tRPC commands.

Initial FIX application scope remains New Order Single, Cancel, Cancel/Replace, Order Status Request, Execution Report, Cancel Reject, Business Message Reject, Market Data Request, Snapshot and Incremental Refresh. Each operation is capability-gated; for example, NBC replace remains unsupported until its selected profile defines it.

## Consequences

One typed router becomes the public contract, so authentication, bounds, errors and recovery semantics are shared by direct clients and protocol adapters. The Rust market engine and origin transaction remain authoritative, while FIX session complexity moves to the participant-side bridge where standard local FIX/TCP is available.

The initial deployment adds one service-binding hop and a TypeScript build alongside the Rust Worker. Router and internal-service schemas must be versioned and tested together, and language clients without router-derived TypeScript types need contract-fixture conformance tests.

ADR 0004's raw-FIX-over-WebSocket endpoint is not implemented. Existing provisional Rust REST routes remain migration-only internal handlers and may be removed after the private service contract replaces them.

## Rejected alternatives

### Emulate a tRPC server in Rust

Rejected because wire compatibility alone does not provide tRPC's router-derived types and would make Bunting maintain an unofficial implementation of the protocol.

### Keep public REST/WebSocket beside tRPC

Rejected because two public application APIs would drift in authentication, error, capability and recovery behavior. Specialized adapters belong on the client side of the tRPC contract.

### Keep raw FIX over Worker WebSocket

Rejected because it gives the server a second public session protocol and prevents the FIX adapter from sharing the typed Bunting client path.

### Give the tRPC Worker market authority

Rejected because matching, ledger projection, idempotency and origin commits already have one Rust authority boundary and must remain atomic there.

## Validation

- router type tests and runtime schema tests cover every procedure input, output and structured error;
- contract fixtures prove decimal-string preservation beyond JavaScript's safe integer range;
- public authentication cannot forge participant identity across the private binding;
- duplicate commands and stale expected sequences return the same typed outcomes through tRPC as the Rust transaction path;
- mutations are acknowledged only after an origin commit;
- subscription reconnect, reset, coalescing and slow-consumer tests retain ADR 0011 semantics;
- FIX tests cover partial/coalesced TCP reads, Logon, heartbeat, resend, gap fill, reconnect, bounded backpressure and durable session restart;
- FIX application conformance covers submit, cancel, supported replace/status operations, partial/full fills, rejects and market-data recovery;
- at least two independent FIX implementations interoperate with the bridge;
- both Workers build for their Cloudflare targets and staging smoke tests exercise the public tRPC endpoint only.

## Operational impact

Deploy the private Rust Worker and its D1 migrations before the public tRPC Worker, then bind the public Worker to the version-compatible private service. The Rust service is not assigned a public route in production. Releases record both artifact versions and reject incompatible router/service contracts during health checks.

The FIX bridge is user-operated or packaged as a native client binary; it is not a Cloudflare deployment dependency. Its local session store has explicit retention, reset and backup behavior.

## Security impact

The public Worker validates credentials, tenancy, procedure bounds and payload schemas. The private Worker independently authenticates the service binding and accepts actor identity only from a protected internal assertion. Secrets, participant-private streams and origin/cache internals are never exposed through public error data.

The local FIX listener defaults to loopback, requires explicit configuration for non-loopback binding, bounds message size and session queues, redacts credentials from logs and does not silently reset sequence state.

## References

- `docs/architecture.md`
- `tests/fixtures/api/trpc-fix-contract.v1.json`
- ADR 0011, streaming market-data protocol semantics
- ADR 0013, Worker and OrderBook-rs authority
- ADR 0014, market-engine and execution-engine boundaries
- `clients/fix-bridge/AGENTS.md`
- `ref/quickfixj`, `ref/fixer`, `ref/ferrumfix` and `ref/ironfix` as conformance or component candidates under the adoption policy
