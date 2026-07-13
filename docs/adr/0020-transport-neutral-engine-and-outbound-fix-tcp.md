# ADR 0020: Transport-neutral engine and outbound FIX/TCP

- Status: Accepted
- Date: 2026-07-13
- Supersedes: ADR 0016 only where it makes tRPC the permanent sole public
  application boundary; ADR 0019 statements that require FIX or RIT adapters to
  call tRPC
- Preserves: ADR 0011 committed-stream semantics, ADR 0018 single-engine
  authority, ADR 0019 private matcher ownership, and the current Rust Worker as
  a transitional deployable implementation

## Context

The Rust Worker currently implements a bounded tRPC-compatible fetch subset.
That is a usable browser-facing transport, but it is not an engine authority
boundary and does not need to mediate in-process Worker components. Treating it
as the universal boundary also adds an unnecessary RPC hop for FIX.

Cloudflare Workers can initiate outbound TCP connections but do not accept
inbound raw TCP. Bunting therefore acts as a FIX initiator/client: the Worker
opens a socket to an external acceptor and the established FIX session carries
bidirectional application traffic.

## Decision

`bunting-engine` exposes a transport-neutral, sans-I/O Rust command boundary.
Worker components and built-in agents invoke Rust application functions in
process. No protocol transport exists inside market authority.

The human Rust/WASM application uses a browser-compatible Worker fetch and
streaming transport. The current tRPC-compatible handlers are transitional and
may remain while that client migrates, but new architecture and engine APIs do
not depend on tRPC-specific types.

FIX uses direct outbound TCP from a Worker-owned session object to an external
FIX acceptor. The FIX connection is bidirectional after initiation. Bunting
does not expose an inbound raw TCP listener, and FIX mapping calls the Bunting
application transaction in process rather than issuing an HTTP or RPC request.

FIX and human sessions may choose direct execution or optional QUARCC-managed
execution. Every built-in Bunting agent uses QUARCC. In every mode the resulting
canonical command crosses the same engine validation, risk, matching, ledger,
event, origin-commit, and committed-publication path.

## Consequences

Transport adapters remain outside `bunting-engine`, and tRPC compatibility can
be retired without changing market semantics. A Worker FIX session object owns
socket/session/reconnect state but cannot own market state. Browser clients
continue to work through fetch/stream APIs because raw TCP is unavailable to
them.

The Worker application path needs a transport-neutral name. `apps/trpc-api`
will move atomically to `apps/bunting-worker` after its active routes and
deployment paths are classified.

## Rejected alternatives

### Route FIX through tRPC

Rejected because the session already runs inside the Worker and can call the
application transaction directly; an internal network envelope adds failure
and serialization without an authority benefit.

### Accept inbound FIX/TCP in the Worker

Rejected because Workers cannot provide an inbound raw TCP listener. External
acceptors must receive the outbound connection initiated by Bunting.

### Use raw TCP for the browser client

Rejected because browsers do not expose arbitrary raw TCP. The Rust/WASM client
requires a browser-compatible fetch/streaming transport.

### Put sockets or protocol state in `bunting-engine`

Rejected because transport/session recovery has different authority and
portability from deterministic market state.

## Validation

- no FIX mapping or Worker session performs an RPC/network hop to execute a
  local Bunting application transaction;
- no Worker route accepts inbound raw TCP;
- browser fetch/stream recovery retains committed sequence/reset/backpressure
  semantics;
- direct and QUARCC-managed FIX modes produce ordinary canonical commands;
- built-in agents cannot bypass QUARCC;
- FIX session snapshots remain outside engine snapshots;
- native/Wasm/Worker builds preserve one production matcher and one engine
  authority path.

## Operational impact

Each configured FIX session needs an external acceptor address, bounded socket
and reconnect policy, durable sequence/journal state, and explicit credential
handling. Worker deployment retains browser routes and gains outbound socket
capability; no inbound port or second market service is deployed.

## Security impact

Outbound destinations, credentials, message sizes, queues, reconnects, and
session resets are allowlisted and bounded. FIX session logs redact secrets.
Direct in-process calls still authenticate the session actor and enforce venue
authorization, risk, idempotency, and expected-version checks.

## References

- `docs/architecture.md`
- `docs/plans/corrected-bunting-implementation-plan.md`
- ADR 0011, ADR 0016, ADR 0018, and ADR 0019
- Cloudflare Workers TCP sockets documentation
