# ADR 0004: FIX 4.4 session semantics over WebSocket with a native TCP bridge

- Status: Superseded by ADR 0015
- Date: 2026-07-11

## Context

Users require FIX compatibility. Standard FIX engines normally connect to a TCP acceptor. Cloudflare Workers currently provide an API for outbound TCP connections but do not expose an inbound raw TCP listener for a Worker.

Workers and Durable Objects do support inbound WebSocket upgrades. The same raw FIX tag-value messages can be transported as binary WebSocket frames while preserving FIX session semantics.

## Decision

The authoritative FIX endpoint is:

```text
wss://<host>/v1/runs/{run_id}/fix
Sec-WebSocket-Protocol: bunting.fix44.v1
```

Rules:

- one complete raw FIX message per binary WebSocket frame;
- SOH delimiters are retained;
- no JSON wrapper in the normal FIX channel;
- the Durable Object owns incoming and outgoing sequence numbers, heartbeat state, resend processing and the outbound journal;
- FIX 4.4 is the first supported application version.

Ship a native Rust package and binary, `bunting-fix-bridge`, which:

- listens on a configurable local TCP address;
- accepts a standard FIX initiator or connects to a local FIX acceptor;
- incrementally frames complete FIX messages from partial TCP reads;
- forwards complete raw messages over authenticated WSS;
- writes returned messages to TCP unchanged;
- preserves ordering and applies bounded backpressure;
- never silently rewrites sequence numbers or resets sessions.

This makes existing FIX tools standards-compatible from their perspective while keeping Cloudflare ingress within supported primitives.

An optional separate adapter may use Worker outbound TCP to initiate a connection to an external FIX acceptor. That is not the inbound user path.

## Initial message scope

Session: Logon, Logout, Heartbeat, Test Request, Reject, Resend Request and Sequence Reset.

Application: New Order Single, Cancel, Cancel/Replace, Order Status Request, Execution Report, Cancel Reject, Business Message Reject, Market Data Request, Snapshot and Incremental Refresh.

## Consequences

Positive:

- no external always-on TCP gateway is required for normal users;
- standard local FIX clients remain usable;
- shared Rust framing and session tests cover server and bridge;
- WebSocket authentication and Durable Object routing remain Cloudflare-native.

Negative:

- FIX-over-WebSocket is a Bunting transport convention, not standard FIX/TCP;
- users requiring direct internet FIX/TCP must run the bridge or a future external gateway;
- reconnect semantics must coordinate WebSocket transport and FIX sessions carefully.

## Rejected alternatives

### Pretend the Worker can accept raw TCP

Rejected because it is not a supported inbound Worker primitive.

### External TCP gateway as the only architecture

Rejected for the initial product because it adds operational infrastructure and another authoritative failure domain.

### JSON representation of FIX

Rejected for the FIX channel because it harms interoperability and changes framing semantics. Native JSON APIs remain separate.

## Validation

- partial and coalesced TCP-read tests;
- binary WebSocket round trips preserve exact bytes;
- session tests cover sequence gaps, resend, gap fill, duplicate flags, heartbeats and reset;
- interoperability tests use at least two independent FIX implementations;
- bridge reconnect never silently loses or reorders accepted messages.

## References

- Cloudflare TCP sockets: https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/
- `ref/ironfix`
- `ref/fixer`
- `ref/ferrumfix`
