# ADR 0040: Bunting Native Protocol v1

- Status: **Accepted** (2026-10-10), implemented in slice 25. Refines
  [ADR 0031](0031-proposed-bunting-native-client-protocol.md) in part: the
  codec candidate (`postcard`) is replaced by a fixed binary layout, and
  identity comes from the registered certificate fingerprint rather than the
  certificate subject. ADR 0031's interface decision (FIX plus BNP only)
  stands.
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
   names, and rotating a team's certificate is a roster edit. v1 accepts the
   participant role only.
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
7. **One live session per participant per interface.** FIX and BNP keep
   separate claims.
8. **Native only.** BNP links `rustls` and is compiled out of the
   `wasm32` server build (`cfg(not(target_arch = "wasm32"))`); the WASIX
   package has no BNP listener.
9. **Reference client.** `packages/bunting-client` (connection, TLS, Hello,
   probe replies, heartbeats, cursor tracking, a feed book) and
   `apps/bunting-trader` (CLI). The client answers probes and sends orders
   immediately with `TCP_NODELAY`; it is a good client, not a compensated one.

## Consequences

- Teams can trade from a native app without FIX tooling, at exactly FIX's
  speed through the venue.
- Operators run a CA and register certificate fingerprints in `bnp.roster`.
- Shared server pieces (outbound hold, public feeds, replies) are generic
  over the message type; FIX behavior is unchanged.
- Order-by-order and consolidated feeds, instructor/admin control, news and
  tenders stay FIX-only or Target in v1.

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

Unit tests cover the codec (byte-exact layouts, direction, bounds, malformed
input), identity namespaces, event attribution and the distributor's
resume window.

## Operational impact

A `bnp` block in the server config names the bind address, server
certificate and key, client CA, CRLs, roster and limits; `bunting-trader
fingerprint` prints the value to register. CRLs are read at start-up; a
revocation needs a restart in v1.

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
