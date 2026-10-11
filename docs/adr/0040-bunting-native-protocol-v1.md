# ADR 0040: Bunting Native Protocol v1

- Status: **Accepted** (2026-10-10), implemented in slice 25. Refines
  [ADR 0031](0031-proposed-bunting-native-client-protocol.md) in part: the
  codec candidate (`postcard`) is replaced by a fixed binary layout, and
  identity comes from the registered certificate fingerprint rather than the
  certificate subject. ADR 0031's interface decision (FIX plus BNP only)
  stands. Its operational impact (revocation needs a restart) is amended by
  [ADR 0041](0041-bnp-live-certificate-revocation.md): CRLs reload live.
  Amended in slice 32 after the independent review: private reports are
  delivered in committed sequence order (decision 6), the kill switch is per
  participant with an explicit reset (decision 10), and connections hold no
  session slot until they authenticate (decision 11). Extended in slice 35:
  order-by-order and consolidated-tape feeds and the published access delay
  (decision 12). Extended in slice 41 by
  [ADR 0042](0042-bnp-operators-and-run-stream.md): instructor and
  administrator certificates and the run stream. Extended in slice 43: a
  participant's resting and reduced reports carry its order's public
  reference (decision 13).
- Wire contract: [`docs/specs/bnp-v1.md`](../specs/bnp-v1.md).

## Context

ADR 0031 accepted two participant interfaces, FIX and a certified Bunting
Native Protocol (BNP), and left the BNP codec, the identity mapping and the
resume mechanics open. The owner's direction (2026-10-09, 2026-10-10):

- the client is an app that connects to a hosted server, over Bunting's own
  protocol with certified clients, plus FIX; nothing else;
- realism first: a team's real network and client stack delay counts as it
  is, virtual distance is added both ways, with no modes, equalization or
  compensation (ADR 0035). A slow client stack must cost its team.

Observed before this slice (`859c015`): FIX sessions stamp arrivals on their
reader thread, admit through the continuous sequencer, and receive committed
reports from the distributor after `L(s, p)`; public feeds and the outbound
hold were FIX-typed. The server had no in-process TLS.

## Decision

1. **Wire format.** Length-prefixed frames (`u32le` length, one type byte,
   body) with fixed-layout little-endian fields, in the style of exchange
   binary protocols (OUCH, ITCH, SBE). Prices, quantities and cash use the
   engine's integer units. Every string, group and frame is bounded. The
   codec is first-party (`packages/bnp-wire`, no dependencies) and compiles
   for `wasm32-unknown-unknown`. `postcard` is not adopted: a fixed layout is
   readable in other languages without a Rust schema and decodes with no
   allocation-driven variability.
2. **Transport.** TLS 1.3 only, terminated in the venue process with
   `rustls` (ring provider). No plaintext mode and no proxy termination for
   BNP.
3. **Certified identity.** The client certificate must chain to the operator
   CA and pass the configured CRLs; its SHA-256 fingerprint must be in the
   run's roster, which names the participant. Subjects and SANs carry no
   authority, so one CA can serve several runs without minting run-specific
   names, and rotating a team's certificate is a roster edit. v1 accepted the
   participant role only; ADR 0042 adds instructors and administrators.
4. **Same path as FIX.** The session's reader thread stamps *ciphertext* on
   arrival, so the team's TLS and network cost is inside `t_rx`. Orders go to
   their listing's venue, other requests to the hub, through the same
   admission sequencer; reports leave after commit and wait `L(s, p)`. Probes,
   pongs and heartbeats are session messages and are never delayed. There is
   no BNP-specific priority, batching or smoothing.
5. **Stateless identities.** A venue order ID is
   `namespace(run, participant) << 64 | client_order_id`; command IDs use
   separate namespaces for new orders, cancels and kill switches. Client order
   IDs are unique per participant per run (OUCH rule). Nothing about identity
   lives in session state, so a reconnect or restart cannot re-map an order.
6. **Resume by committed sequence.** Every private report carries its event
   sequence. The distributor retains the latest 16,384 committed batches; a
   `Hello` with a cursor inside the window replays exactly the missed reports
   (atomically with the live subscription), then `ReplayComplete`. Outside
   it, the session starts live with `resume = gap` and the client rebuilds
   from `OpenOrders` and `Account`, which state their committed position.
   Reports leave in sequence order: one whose path is shorter than an
   earlier report's waits for it, as on any single ordered connection.
   Without that, a near venue's report could overtake a far venue's, the
   client's cursor (its highest sequence) would pass the slower one, and a
   resume after a disconnect would skip it.
7. **One live session per participant per interface.** FIX and BNP keep
   separate claims.
8. **Native only.** BNP links `rustls` and is compiled out of the
   `wasm32` server build (`cfg(not(target_arch = "wasm32"))`); the WASIX
   package, retired by ADR 0044, never had a BNP listener.
9. **Reference client.** `packages/bunting-client` (connection, TLS, Hello,
   probe replies, heartbeats, cursor tracking, a feed book) and
   `apps/bunting-trader` (CLI). The client answers probes and sends orders
   immediately with `TCP_NODELAY`; it is a good client, not a compensated one.
10. **Per-participant kill switch (slice 32).** `KillSwitch` cancels every
    live order of the sender's participant and blocks only that
    participant's new orders until it sends `KillSwitchReset`. The engine
    keeps the set of kill-switched participants. Slice 25 shipped the
    engine's earlier run-wide switch, which let any one team halt the whole
    run; no real venue gives a participant that power. FIX v1 has no kill
    switch message.
11. **Authenticate before a session slot (slice 32).** Until a registered
    certificate completes TLS and sends `Hello`, a connection counts only
    against an unauthenticated budget (256 in total, 32 per address) bounded
    by `handshake_timeout_ms`; only then does it take one of
    `max_connections`. A session whose socket accepts nothing for three
    heartbeat intervals is closed as a slow consumer.
12. **Every public feed (slice 35).** A `Subscribe` with `FeedFlags::ORDERS`
    is an order-by-order feed and one naming venue 0 is the instrument's
    consolidated tape, both with ADR 0036's semantics and served by the same
    feed publisher, at the same times, as FIX's (`bnp_feeds.rs`). An order
    snapshot larger than a frame is sent in parts. `AccessDelay` publishes
    the session's measured delay, as the venue already did for operators.
13. **Own order reference (slice 43).** `OrderRested` and `OrderReduced`
    carry the order's public reference on the order-by-order feed (its book
    time priority), as OUCH's accepted message carries the reference ITCH
    publishes, so a team can find its own queue place. A new value means the
    order lost priority (iceberg refresh). It is read from the state right
    after the commit, like the public feed, and costs nothing to sessions
    without an order-by-order subscription. Two existing messages gained a
    field: BNP v1 is pre-release, and the reference client and server change
    together.

## Consequences

- Teams can trade from a native app without FIX tooling, at exactly FIX's
  speed through the venue.
- Operators run a CA and register certificate fingerprints in `bnp.roster`.
- Shared server pieces (outbound hold, public feeds, replies) are generic
  over the message type; FIX behavior is unchanged.
- Tenders stay FIX-only (order-by-order and consolidated feeds came in
  slice 35; instructor/admin control and the run stream in slice 41,
  ADR 0042).

## Rejected alternatives

- **`postcard` or another serde codec:** ties the format to Rust type
  layout and varint encoding; harder for C++/Python clients to implement.
- **Identity from the certificate subject:** couples identity to CA naming
  and lets any certificate the CA issues with a matching name act as the
  team.
- **Session-held order maps:** the trap ADR 0018 and slice 12 removed;
  identities must be derivable from the journal.
- **Resume from the full journal on every reconnect:** unbounded work on the
  session path; a bounded window plus state rebuild covers long gaps.
- **A BNP fast path or fairness smoothing:** contradicts ADR 0035.

## Validation

`apps/bunting-server/tests/bnp.rs`:

1. Only registered, unrevoked certificates from the operator CA get a session;
   a client that does not trust the venue's CA fails; a second session for the
   same participant is refused.
2. A BNP order fills against a FIX maker; reports and the public feed show
   the trade.
3. A reconnect with a cursor replays exactly the missed reports; a cursor
   outside the window gets `gap`, then `OpenOrders` rebuilds state.
4. With a 30 ms path, a BNP order's report arrives no earlier than the round
   trip, while `Ping` returns at once.
5. (Slice 32) A kill switch cancels and blocks only its own team until
   reset; private reports from a near and a far venue arrive in sequence
   order, so a resume never skips one; twelve idle unauthenticated sockets
   do not keep registered teams out and are closed at the handshake
   deadline. Unit tests cover the unauthenticated budget and the
   slow-consumer write timeout.
6. (Slice 35) An order-by-order feed shows queue places and trades naming
   the resting order; the tape shows each venue's quote and trades; the
   venue publishes the session's access delay.

Unit tests cover the codec (byte-exact layouts, direction, bounds, malformed
input), identity namespaces, event attribution and the distributor's
resume window.

## Operational impact

A `bnp` block in the server config names the bind address, server
certificate and key, client CA, CRLs, roster and limits; `bunting-trader
fingerprint` prints the value to register. CRLs were read at start-up only
in slice 25; since slice 29 they reload live (ADR 0041).

## Security impact

Identity is a certificate the client proves possession of, checked against an
operator roster; no shared secrets. All decoding is bounded and rejects
trailing bytes, unknown types and wrong-direction messages. Private reports
are filtered by participant namespace before they leave the venue. Test
certificates are generated at run time; no key material is committed.

## References

- [ADR 0031](0031-proposed-bunting-native-client-protocol.md),
  [ADR 0035](0035-latency.md), [ADR 0036](0036-public-market-data-feeds.md)
- [`docs/specs/bnp-v1.md`](../specs/bnp-v1.md)
- RFC 8446 (TLS 1.3); RFC 5280 (X.509 and CRLs); Nasdaq OUCH 5.0 and ITCH 5.0
  (layout style and client order ID rule, public specifications; no text
  copied)
