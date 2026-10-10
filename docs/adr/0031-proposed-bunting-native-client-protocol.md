# ADR 0031: Two client interfaces — FIX and a certified Bunting native protocol

- Status: **Accepted** (2026-10-09) by owner direction, recorded in ADR 0033.
  **Partly implemented** (slice 25): BNP v1 participant order entry, private
  stream with resume, direct L2 feeds and the reference client are built per
  [ADR 0040](0040-bunting-native-protocol-v1.md), which replaces this ADR's
  codec candidate (fixed binary layout, not `postcard`) and identity mapping
  (registered certificate fingerprint, not subject). Instructor/admin
  control, run/news streams, CRL reload, the TUI migration, bindings and the
  FIX/BNP parity test remain Target.
- Would supersede: the browser procedure contract's role in ADR 0016/0020
  (`bunting-api-contract` browser procedures, `browser-wire`,
  `schemas/browser`), which has had no server since the Worker removal
  (`eed8e00`).
- Keeps: ADR 0021/0023 FIX profile and concurrent FIX sessions; ADR 0022
  (Cloudflare publishes read-only artifacts; it never accepts commands).

## Context

The owner's direction (2026-10-09):

- end users use **an app that connects to a hosted server**;
- the server and clients communicate over **Bunting's own protocol between a
  certified server and certified clients**, **and FIX** — no other interfaces.

Observed state at `1d857d1`:

- FIX/TCP is implemented (`simfix-*`, `apps/bunting-server/src/session_host.rs`).
- The server has no in-process TLS: `TlsConfig` is `Disabled` or
  `Terminated { trusted_proxy, require_mutual_tls }`
  (`apps/bunting-server/src/config.rs:37-43`). The TUI already links `rustls`
  and `tokio-rustls` as a client.
- A JSON browser contract exists with no host. Its domain types
  (`ActorRole`, `ActorIdentity`, `Audience`, decimal-string IDs) are also used
  by the server and must not be deleted with it.
- FIX lacks things an app needs: unsolicited private streams with resume
  cursors, account/position snapshots, news/tenders, instructor control and
  RTT probes at sub-millisecond resolution (ADR 0030).

## Decision

Bunting exposes exactly two participant interfaces, both thin adapters over
the same in-process application service:

1. **FIX** — FIXT.1.1 / FIX 5.0 SP2 competition profile, for standard
   tooling and contestants' own engines. Unchanged except for the event
   fan-out and RTT probing proposed in the exploration note and ADR 0030.
2. **Bunting Native Protocol (BNP)** — for the Bunting app, bindings and
   built-in tooling.

### BNP shape

- **Transport:** TCP with TLS 1.3 terminated *in the server process*
  (`rustls`), so certificate identity reaches the session without a proxy
  hop. QUIC may be added later behind the same message layer.
- **Certification:** an operator CA issues the server certificate and one
  client certificate per participant (or team/instructor/admin). Mutual TLS
  authenticates the connection; the certificate subject maps to an immutable
  `ActorIdentity`. No password logon, no client-chosen identity field. Revocation
  is a CRL/deny-list the server reloads. *Certified* means the identity is
  certified — a client binary is never trusted to behave.
- **Framing:** length-prefixed frames with a protocol version, message type
  and bounded size; payloads encoded with one deterministic binary codec
  (candidate: `postcard`, subject to `docs/reference-adoption.md` review). A
  JSON debug encoding of the same types is allowed for tests and tooling only.
- **Message families:**
  - session: `Hello`/`Welcome` (versions, limits, run manifest hash),
    `Ping`/`Pong` with server monotonic µs stamps (feeds ADR 0030), heartbeat;
  - order entry: submit/cancel (and later replace) addressed by `ListingKey`,
    client order IDs, idempotency keys;
  - private stream: execution reports, order states, positions, cash,
    reservations — each with the committed event sequence and a resume cursor;
  - public stream: per-listing L1/L2/trades with sequence numbers, snapshot on
    subscribe, explicit gap → snapshot recovery, bounded per-subscriber queues;
  - run stream: lifecycle, clock, news, tenders, scoring updates;
  - instructor/admin: start/pause/advance, scenario control, broadcasts —
    only for certificates carrying those roles.
- **One schema crate** (rename or split `bunting-api-contract`) owns the BNP
  message types; a client crate (`bunting-client`) implements connection,
  TLS, resume and gap recovery once, and is used by the TUI, the GUI app and
  a network surface added to the C/Python/C++ bindings.

### The app

A native desktop app built on `bunting-client` connects to a hosted server.
The existing Ratatui TUI becomes the first BNP client (it is already a
participant-side app); a GUI app follows using the same crate. Toolkit choice
(e.g. egui, iced, Tauri) is *unresolved* and does not affect the protocol.

### Retired

The browser procedure set, `browser-wire` and `schemas/browser` are removed
once BNP covers their procedures. Cloudflare remains a read-only publisher
of archives and leaderboards (ADR 0022).

## Consequences

- One app-facing protocol designed for streaming, resume and identity instead
  of request/response JSON; FIX stays the interoperability surface.
- The server takes on TLS and certificate handling, plus an operator workflow
  to issue and revoke client certificates.
- Bindings keep their in-process `bunting-rs` surface (observed:
  `bindings/bunting-ffi` exposes archive replay and a local handle) and gain a
  `bunting-client` surface so contestants can trade from Python or C++.
- Two adapters must stay semantically equal; a parity test drives the same
  orders through FIX and BNP and compares committed events.

## Rejected alternatives

- **Host the existing browser JSON contract in the server:** request/response
  JSON over HTTP is a poor fit for ordered private streams and resume, and the
  owner wants an app, not a browser client.
- **gRPC/protobuf:** brings an HTTP/2 stack and code generation for little
  gain over a small Rust-owned schema; a gRPC gateway can be added later as an
  adapter if third parties need it.
- **WebSocket JSON:** browser-friendly, but browsers are out of scope and
  WebSocket adds a framing layer over TLS without benefit for a native app.
- **FIX only:** possible, but app features (accounts, news, instructor
  control, µs RTT probes) would need many custom FIX messages.

## Validation

1. mTLS: a connection without a valid client certificate, with a revoked one,
   or with a certificate for another run is refused before any application
   message.
2. Identity: a BNP message cannot name a different participant than the
   certificate; attempts are rejected and audited.
3. Resume: a client disconnects mid-run, reconnects with its cursor and
   receives exactly the missed private events; a public subscriber with a gap
   receives a snapshot.
4. Parity: the same order sequence through FIX and BNP yields identical
   committed events and final hash.
5. Bounds: oversized frames, floods and slow consumers are rejected or
   disconnected with the named limit and a resume cursor.

## Operational impact

Operators run a small CA (or use an existing one), issue per-participant
client certificates with the run roster, distribute them with the app, and
publish the server certificate fingerprint. Health output shows sessions per
interface, queue depths and certificate expiry.

## Security impact

Identity moves from shared secrets to certificates, removing credential reuse
across participants. Private streams are filtered by certificate identity
before fan-out. Private keys stay on the client; certificate loss is handled
by revocation. TLS in-process removes the trusted-proxy assumption, so
`Terminated` mode should be kept only for deployments that need it.

## References

- [ADR 0016](0016-native-rust-trpc-worker.md), [ADR 0020](0020-transport-neutral-engine-and-outbound-fix-tcp.md), [ADR 0021](0021-fixt11-fix50sp2-dictionary-adoption.md), [ADR 0022](0022-native-competition-venue-and-publication-worker.md), [ADR 0023](0023-concurrent-participant-fix-sessions.md)
- [ADR 0030 (proposed)](0030-proposed-latency-modeled-continuous-admission.md) — Ping/Pong requirements
- [Exploration note, 2026-10-09](../research/2026-10-09-exploration-and-next-steps.md) — G1, G11
- RFC 8446 (TLS 1.3); RFC 5280 (X.509 certificates and CRLs)
