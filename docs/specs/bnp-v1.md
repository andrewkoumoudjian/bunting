# Bunting Native Protocol, version 1

- Status: **Current** (slice 25). Decision record: [ADR 0040](../adr/0040-bunting-native-protocol-v1.md).
- Code: `packages/bnp-wire` (types and codec; the field order in
  `src/codec.rs` is this contract), `packages/bunting-application/src/bnp.rs`
  (identity and event mapping), `apps/bunting-server/src/bnp_host.rs` (venue
  host), `packages/bunting-client` and `apps/bunting-trader` (reference client).

BNP is Bunting's own participant protocol beside FIX (ADR 0031). Both are thin
adapters over the same admission sequencer, engine and committed-event
distributor: a BNP order waits for the same virtual distance as a FIX order,
and BNP has no speed advantage or compensation. The team's real network and
client stack delay counts as it is (ADR 0035).

## Transport and identity

- TCP, then TLS 1.3 terminated in the venue process (`rustls`, ring
  provider). No other TLS version, no plaintext, no proxy termination.
- Mutual authentication. The client certificate must chain to the operator CA
  configured as `bnp.client_ca` and must not be revoked by any configured CRL.
- **Identity is the leaf certificate's SHA-256 fingerprint** (lowercase hex of
  the DER encoding) looked up in the run's roster (`bnp.roster`). The subject
  and SANs carry no authority. An unregistered fingerprint receives `Logout`
  and the connection closes. A message can never name a participant: every
  command is attributed to the roster entry.
- v1 rosters accept the `participant` role only. Instructor and administrator
  control over BNP is Target.
- One live BNP session per participant. A second connection waits briefly for
  the first to close, then is refused with `Logout`. FIX and BNP sessions of
  the same participant are independent.
- `bunting-trader fingerprint --cert team.pem` prints the value an operator
  registers.

## Framing

```
frame   = length:u32le  type:u8  body
length  = 1 + len(body)            (bytes after the length field)
```

- All integers are little-endian, fixed width. `i128`/`u128` are 16 bytes.
- `length` is at most the session's `max_frame_bytes` (from `Welcome`, never
  above 65,536). A larger or empty frame closes the connection.
- `bool` is one byte, 0 or 1. `option<T>` is a `bool` then `T` when present.
- `string` is `u16` byte length (at most 1,024) then UTF-8 bytes.
- `group<T>` is `u16` count (at most 4,096) then the entries.
- `listing` is `venue_id:u128 instrument_id:u128` (32 bytes).
- `stamp` is `sequence:u64 logical_time_ns:u64`. `sequence` is the committed
  event sequence of the run, the private stream's cursor. Logical time is
  run time (ADR 0037): nanoseconds since the run began, as is a GTD
  order's `expires_at_ns`.
- Prices are `i64` ticks, quantities `i64` lots, cash `i128` minor units,
  exactly as the engine stores them.
- Trailing bytes after a known body, an unknown type, or a message sent in the
  wrong direction is a protocol error and closes the session.

## Client messages

| Type | Name | Body |
|---|---|---|
| 0x01 | Hello | `magic:"BNP1" version:u16 resume_after:option<u64> client_name:string` |
| 0x02 | Heartbeat | empty |
| 0x03 | ProbeReply | `probe_id:u64` |
| 0x04 | Ping | `nonce:u64` |
| 0x05 | Logout | `reason:string` |
| 0x10 | NewOrder | `client_order_id:u64 listing side:u8 quantity:i64 order_type:u8 price:i64 tif:u8 expires_at_ns:u64 post_only:bool anonymous:bool display_quantity:option<i64>` |
| 0x11 | CancelOrder | `client_order_id:u64` |
| 0x12 | KillSwitch | `request_id:u64` |
| 0x20 | Subscribe | `request_id:u32 listing flags:u8` |
| 0x21 | Unsubscribe | `request_id:u32` |
| 0x22 | SnapshotRequest | `request_id:u32 listing depth:u16` (0 = full book) |
| 0x30 | ListingsRequest | `request_id:u32` |
| 0x31 | OpenOrdersRequest | `request_id:u32` |
| 0x32 | AccountRequest | `request_id:u32` |

Enumerations: `side` 1 buy, 2 sell. `order_type` 1 limit (uses `price`),
2 market (`price` must be 0). `tif` 0 GTC, 1 IOC, 2 FOK, 3 DAY, 4 GTD (uses
`expires_at_ns`, otherwise 0). `anonymous` hides the participant's broker
identifier on venues that publish them (ADR 0036, as FIX tag 10021). Feed `flags` bit 0 bids, bit 1 offers, bit 2
trades.

## Server messages

| Type | Name | Body |
|---|---|---|
| 0x41 | Welcome | `version:u16 run_id:u128 participant_id:u128 role:u8 heartbeat_ms:u32 max_frame_bytes:u32 resume:u8 run_sequence:u64` |
| 0x42 | Heartbeat | empty |
| 0x43 | Probe | `probe_id:u64` |
| 0x44 | Pong | `nonce:u64 server_time_us:u64` |
| 0x45 | Reject | `request_type:u8 reference:u64 reason:string` |
| 0x46 | ReplayComplete | `through_sequence:u64` |
| 0x47 | Logout | `reason:string` |
| 0x50 | OrderAccepted | `stamp client_order_id:u64 order_id:u128 listing side:u8 quantity:i64 price:option<i64>` |
| 0x51 | OrderRejected | `stamp client_order_id:u64 reason:u8` |
| 0x52 | OrderRested | `stamp client_order_id:u64 order_id:u128 price:i64 remaining:i64` |
| 0x53 | Fill | `stamp client_order_id:u64 order_id:u128 listing side:u8 price:i64 quantity:i64 fee:i128 liquidity:u8` |
| 0x54 | OrderDone | `stamp client_order_id:u64 order_id:u128` |
| 0x55 | OrderCanceled | `stamp client_order_id:u64 order_id:u128 remaining:i64 reason:u8` |
| 0x56 | CancelRejected | `stamp client_order_id:u64 reason:u8` |
| 0x57 | PositionChanged | `stamp instrument_id:u128 delta:i64` |
| 0x58 | BalanceChanged | `stamp delta:i128` |
| 0x59 | KillSwitchActivated | `stamp` |
| 0x5a | OrderReduced | `stamp client_order_id:u64 order_id:u128 remaining:i64` |
| 0x60 | MarketSnapshot | `request_id:u32 listing next_report:u64 bids:group<price:i64 quantity:i64> asks:group<…>` |
| 0x61 | MarketUpdate | `request_id:u32 listing first_report:u64 entries:group<kind:u8 price:i64 quantity:i64>` |
| 0x70 | Listings | `request_id:u32 listings:group<listing symbol:string>` |
| 0x71 | OpenOrders | `request_id:u32 run_sequence:u64 orders:group<client_order_id:u64 order_id:u128 listing side:u8 price:i64 original:i64 remaining:i64>` |
| 0x72 | Account | `request_id:u32 run_sequence:u64 cash:group<currency_id:u128 balance:i128 reserved:i128> positions:group<instrument_id:u128 position:i64 open_buy:i64 open_sell:i64>` |

Enumerations: `role` 1 participant (2 team, 3 instructor, 4 administrator,
5 built-in agent are reserved). `resume` 0 live, 1 replaying, 2 gap.
`liquidity` 1 maker, 2 taker. Entry `kind` 0 trade, 1 bid, 2 ask (a level's
absolute visible quantity after the change, 0 when it is gone). Reject
`reason` codes 1–23 mirror the engine's `RejectCode` in order
(`packages/bnp-wire/src/lib.rs`, `RejectReason`); 24 is an unknown listing,
25 a closed session and 26 a market, IOC or FOK order in a call phase
(ADR 0037).
Cancel `reason` 1 requested, 2 kill switch, 3 market remainder, 4 mass cancel,
5 expired, 6 halt. The engine currently emits no `PositionChanged` or
`BalanceChanged`; clients read positions and cash with `AccountRequest`.

`Reject` answers a request the venue did not admit or could not commit (rate
limit, bad feed, commit failure). `request_type` is the request's type byte
and `reference` its client order ID or request ID.

## Session

1. Client connects, completes TLS 1.3 and sends `Hello` within
   `handshake_timeout_ms`. Frames pipelined after `Hello` are processed after
   `Welcome`.
2. Server answers `Welcome`, then sends `Probe`s. Clients answer each `Probe`
   with `ProbeReply` at once; the server uses the round trips only to report
   measured delay. Measured delay never orders commands.
3. Either side sends `Heartbeat` after `heartbeat_ms` of silence. A client
   silent for three heartbeat intervals is disconnected.
4. `Ping` returns `Pong` immediately with the venue clock in microseconds.
   Session messages (`Welcome`, `Probe`, `Pong`, `Heartbeat`, `Logout`) are
   never delayed by the latency map.
5. `Logout` from the client is answered with `Logout` and the connection
   closes.

## Orders and identities

- `client_order_id` must be non-zero and unique per participant for the run,
  as in OUCH. Re-sending a `NewOrder` with a used ID commits nothing new; the
  venue answers `Reject` (the original order's reports were already sent, and
  a resuming client recovers them from the private stream).
- The venue order ID is `namespace(run, participant) << 64 | client_order_id`,
  where the namespace is the first 8 bytes of
  `SHA-256("bunting.bnp-namespace.v1\0" ‖ domain ‖ run ‖ participant)`
  (never zero). New orders, cancels and kill switches use separate command
  namespaces, so no two sessions or interfaces can collide.
- `CancelOrder` names the client order ID. A cancel for an unknown or another
  participant's order yields `CancelRejected`.
- Every order travels to its listing's venue and waits `L(team, venue)` there
  before the sequencer can commit it; kill switches and the request messages
  (`Listings`, `OpenOrders`, `Account`) are applied at the hub. Reports leave
  the venue after commit and arrive after `L(venue, team)`.

## Private stream and resume

- Every private report carries the committed event `sequence`. A client's
  cursor is the highest sequence it processed.
- `Hello.resume_after = Some(cursor)` asks for everything after the cursor.
  When the venue's retention window (the latest 16,384 committed batches)
  covers it, `Welcome.resume = replaying`, the missed reports follow in order
  (each already past its path delay), then `ReplayComplete`. Otherwise
  `resume = gap`: the stream starts live and the client rebuilds with
  `OpenOrdersRequest` and `AccountRequest`, whose `run_sequence` says which
  committed position they reflect.
- After a venue restart the window is empty; only a cursor equal to the run's
  current sequence resumes without a gap.

## Public feeds

- `Subscribe` starts a direct, price-level feed of one listing: a
  `MarketSnapshot` taken at the venue with `next_report = 1`, then
  `MarketUpdate`s whose `first_report` numbers each entry consecutively.
  A missing number is a gap; the client unsubscribes and subscribes again.
- `SnapshotRequest` is a one-shot snapshot with `next_report = 0`.
- Feed messages are anonymous: no order, participant or command identity.
- At most 32 subscriptions per session. Order-by-order (L3) and consolidated
  tape feeds are FIX-only in v1.

## Limits

Server configuration (`bnp` block, `apps/bunting-server/src/config.rs`):
`heartbeat_ms` 100–60,000; `max_frame_bytes` 1,024–65,536;
`max_messages_per_interval` per `rate_limit_window_ms` (1–60,000), a
per-session rate limit: excess requests get `Reject`, session messages are
never limited, and nothing is reordered;
`handshake_timeout_ms` 100–60,000; `max_connections` at most the roster size.
A slow reader that falls 4,096 committed batches behind is disconnected and
resumes with its cursor.
