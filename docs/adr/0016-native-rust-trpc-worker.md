# ADR 0016: Native Rust tRPC Worker with conditional Rust stream coordination

- Status: Accepted
- Date: 2026-07-12
- Supersedes: ADR 0015 and every public REST route decision
- Amends: ADR 0013 only to permit a user-approved Rust Durable Object for stream coordination after the gate below
- Retains: ADR 0011 committed-sequence, reset, coalescing and backpressure semantics

## Context

Bunting exposes tRPC, but its market authority, command transaction and Cloudflare deployment should stay as close as possible to native Rust. The current `apps/edge-api` implementation uses `worker::Router` and provisional REST resources. ADR 0015 replaced those routes with a TypeScript tRPC Worker in front of a private Rust service, adding a second public runtime and an internal HTTP hop.

Cloudflare documents complete Rust Workers through `workers-rs`, including Fetch, streaming responses, WebSockets, service bindings and Rust Durable Object classes. Official tRPC remains a TypeScript router and client system. A native Rust server therefore cannot claim TypeScript router inference internally; it must implement and test the pinned tRPC wire contract explicitly.

tRPC query and mutation dispatch does not require a Durable Object. Long-lived subscriptions may eventually need one point of coordination per run, but that decision must follow measured connection and recovery evidence rather than becoming market authority by default.

## Decision

Use one public Rust Cloudflare Worker. The final path is `apps/trpc-api`; the existing `apps/edge-api` path moves mechanically in its own sprint before semantic package renaming.

The Worker uses one direct `#[event(fetch)]` entrypoint. It does not use `worker::Router`, expose REST resources, or retain private REST migration handlers. The public surface is only the versioned tRPC endpoint:

```text
GET|POST /trpc/<procedure-or-batch>
```

`system.health`, command operations, snapshots and subscriptions are tRPC procedures. There is no separate `/health`, `/v1/runs/...`, public cache, FIX or internal REST route.

### Rust protocol boundary

Create focused reusable packages only when implementation and tests are added:

- `packages/bunting-api-contract`: Rust procedure names, input/output types, stable errors, capability metadata and schema export;
- `packages/trpc-wire`: bounded parsing and encoding of the pinned tRPC HTTP/SSE protocol without market semantics;
- `packages/trpc-client`: a native client for first-party Rust adapters, excluded from the Worker graph when native networking requires it.

The protocol boundary supports the explicitly versioned subset in `schemas/trpc/bunting.v1.json`: single queries, single mutations, bounded query batching, structured errors, exact JSON encoding and HTTP subscriptions. Mutation batching is rejected. Inputs and outputs wider than JavaScript's safe integer range use validated decimal strings.

Pin `@trpc/server` and `@trpc/client` `11.18.0`, source git head `6aec1578a899df50a17e4e78d5512a099b574c18`, as development-only conformance oracles after reference intake. The production Rust Worker does not depend on JavaScript tRPC packages. Differential tests send identical fixtures through the official Fetch adapter and `packages/trpc-wire`, then compare status, headers and normalized envelopes. Compatibility is limited to the tested subset; unsupported tRPC features reject explicitly.

Generate the browser TypeScript declarations and client wrapper from the Rust-owned versioned contract. The generated client uses the pinned official `@trpc/client` transport. The generated artifact is checked against the schema and differential suite; it is not hand-maintained as a second contract.

### Authority and command execution

The direct Rust handler performs authentication, tenancy, bounds, procedure lookup and typed decoding, then invokes the existing Rust command transaction in process. Matching, ledger projection, idempotency, origin commits, snapshot publication and canonical events remain in Rust. No protocol adapter can acknowledge a mutation before the origin commit succeeds.

The current participant-header trust model is removed during the cutover. Participant identity comes from verified authentication claims and cannot be supplied independently by the caller.

### Subscriptions and the Durable Object gate

Implement tRPC HTTP subscriptions first against committed origin sequences. A subscription always starts from an explicit sequence or snapshot and never treats isolate memory as authority.

ADR 0016 authorizes one optional Rust `RunStreamCoordinator` Durable Object per run only if a focused spike proves that a plain Worker cannot meet at least one of these requirements within the documented limits:

- bounded fan-out to multiple concurrent subscribers;
- deterministic committed-sequence ordering across isolates;
- reconnect without excessive origin polling;
- backpressure and slow-consumer disconnection with a usable cursor;
- acceptable measured latency and cost under the staging load profile.

If introduced, the Durable Object is implemented with `workers-rs` in the same Rust Worker deployment. It coordinates connections and committed notifications only. It does not match orders, accept commands authoritatively, own the event log, replace the origin expected-version commit, or make Workers Cache transactional. It reconstructs from origin state after eviction. Expanding it into a command sequencer or authoritative store requires another user-approved ADR.

## Consequences

The public protocol, market transaction and optional connection coordinator remain in Rust and one deployment graph. The design removes the TypeScript gateway and internal service hop from ADR 0015 while preserving tRPC client compatibility through an explicit, differential-tested wire subset.

The project now owns a compatibility adapter for a versioned tRPC subset. Upgrades require reference pin review and differential tests before any client or server version changes. Generated TypeScript types are required because the production server is not a TypeScript `AppRouter`.

## Rejected alternatives

### Public REST plus tRPC

Rejected because two public APIs would drift. The existing REST resources are deleted during the cutover.

### TypeScript tRPC gateway in front of Rust

Rejected because it adds another runtime, deployment and internal hop around an already portable Rust transaction path.

### Pretend the Rust server provides native tRPC type inference

Rejected because official tRPC inference comes from a TypeScript router type. Bunting instead owns a versioned Rust contract, generates client declarations and proves wire compatibility.

### Durable Object for every request immediately

Rejected because queries and mutations already have origin versioning and tRPC itself does not require a stateful coordinator. A stream-only Rust Durable Object remains gated by evidence.

## Validation

- official tRPC Fetch adapter differential fixtures cover every supported query, mutation, batch, error and subscription envelope;
- malformed paths, JSON, batch shapes, content types and oversized requests reject within fixed bounds;
- all wide integers round-trip beyond JavaScript's safe integer range;
- generated TypeScript declarations match the Rust contract hash;
- caller input cannot forge participant identity;
- duplicate commands and stale expected versions preserve exact existing outcomes;
- no mutation response precedes the origin commit;
- native and `wasm32-unknown-unknown` checks exclude unsupported runtime dependencies;
- staging proves subscription reset, resume, backpressure and committed ordering with and without the optional coordinator spike;
- the deployed Worker exposes no non-tRPC application route.

## Operational impact

The mechanical path move, protocol cutover and optional Durable Object are separate changes. The Rust Worker remains deployable before a coordinator exists. If the coordinator gate passes, Wrangler receives a Rust Durable Object binding and migration in the same release, with rollback and recovery tests before production use.

Release metadata records the Rust contract version, tRPC oracle version and source commit, generated client hash, Worker build and optional coordinator schema version.

## Security impact

The single Rust entrypoint enforces authentication, tenant identity, procedure allowlists, request bounds and structured error redaction. It never exposes origin/cache internals or accepts actor identity from an untrusted header.

The optional Durable Object receives only authenticated subscription context and committed public/private projections. It cannot execute market commands or read credentials beyond the minimum verified claims needed for stream authorization.

## References

- Cloudflare Rust Workers: https://developers.cloudflare.com/workers/languages/rust/
- Cloudflare WebSockets: https://developers.cloudflare.com/workers/runtime-apis/websockets/
- Cloudflare Durable Objects: https://developers.cloudflare.com/durable-objects/
- `ref/workers-rs`
- tRPC: https://github.com/trpc/trpc
- ADR 0011, ADR 0013 and ADR 0014
