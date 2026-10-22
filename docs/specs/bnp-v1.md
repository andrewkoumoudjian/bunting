# Bunting Native Protocol, version 1

- Status: **Current** (slice 25; order-by-order and tape feeds and access
  delay added in slice 35; operators and the run stream in slice 41,
  [ADR 0042](../adr/0042-bnp-operators-and-run-stream.md); own order
  reference in reports in slice 44; BNP 1.1 reference data, ADR 0054 R1,
  2026-10-22). Decision record: [ADR 0040](../adr/0040-bunting-native-protocol-v1.md).
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
  The venue re-reads the CA and CRL files every second (ADR 0041): new
  handshakes use the current files, and a live session whose certificate no
  longer verifies receives `Logout` ("certificate no longer trusted") and is
  closed. An unreadable file, or a listed CRL file with no CRL, keeps the
  previous trust.
- **Identity is the leaf certificate's SHA-256 fingerprint** (lowercase hex of
  the DER encoding) looked up in the run's roster (`bnp.roster`). The subject
  and SANs carry no authority. An unregistered fingerprint receives `Logout`
  and the connection closes. A message can never name a participant: every
  command is attributed to the roster entry.
- A roster entry's role is `participant`, `instructor` or `administrator`
  (ADR 0042). For an instructor or administrator, `participant_id` is the
  operator's actor ID (unique across the roster) and `Welcome.participant_id`
  carries it; the session names no participant, receives no participant's
  private reports, and its `NewOrder`, `CancelOrder` and kill-switch messages
  are refused with `Reject`.
- One live BNP session per participant. A second connection waits briefly for
  the first to close, then is refused with `Logout`. FIX and BNP sessions of
  the same participant are independent.
- `bunting-trader fingerprint team.pem` prints the value an operator
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
| 0x13 | KillSwitchReset | `request_id:u64` |
| 0x18 | TenderDecision | `request_id:u64 tender_id:u128 accept:bool` |
| 0x19 | OtcPropose | `request_id:u64 negotiation_id:u64 counterparty_id:u128 instrument_id:u128 side:u8 quantity:i64 price:i64 expires_at_ns:u64` |
| 0x1a | OtcCounter | `request_id:u64 negotiation_id:u128 quantity:i64 price:i64` |
| 0x1b | OtcDecision | `request_id:u64 negotiation_id:u128 decision:u8` |
| 0x20 | Subscribe | `request_id:u32 listing flags:u8` |
| 0x21 | Unsubscribe | `request_id:u32` |
| 0x22 | SnapshotRequest | `request_id:u32 listing depth:u16` (0 = full book) |
| 0x30 | ListingsRequest | `request_id:u32` |
| 0x31 | OpenOrdersRequest | `request_id:u32` |
| 0x32 | AccountRequest | `request_id:u32` |
| 0x33 | DealsRequest | `request_id:u32` |
| 0x38 | OperatorCommand | `request_id:u64 action:u8 …` (operator action, below) |

Enumerations: `side` 1 buy, 2 sell. `order_type` 1 limit (uses `price`),
2 market (`price` must be 0). `tif` 0 GTC, 1 IOC, 2 FOK, 3 DAY, 4 GTD (uses
`expires_at_ns`, otherwise 0). `anonymous` hides the participant's broker
identifier on venues that publish them (ADR 0036, as FIX tag 10021). Feed `flags` bit 0 bids, bit 1 offers, bit 2
trades, bit 3 order by order (venue feeds only).

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
| 0x48 | AccessDelay | `source:u8 min_rtt_us:option<u64> one_way_us:option<u64> probe_samples:u64` |
| 0x49 | OperatorDone | `request_id:u64 run_sequence:u64` |
| 0x4a | DealDone | `request_id:u64 run_sequence:u64` |
| 0x50 | OrderAccepted | `stamp client_order_id:u64 order_id:u128 listing side:u8 quantity:i64 price:option<i64>` |
| 0x51 | OrderRejected | `stamp client_order_id:u64 reason:u8` |
| 0x52 | OrderRested | `stamp client_order_id:u64 order_id:u128 price:i64 remaining:i64 reference:option<u64>` |
| 0x53 | Fill | `stamp client_order_id:u64 order_id:u128 listing side:u8 price:i64 quantity:i64 fee:i128 liquidity:u8` |
| 0x54 | OrderDone | `stamp client_order_id:u64 order_id:u128` |
| 0x55 | OrderCanceled | `stamp client_order_id:u64 order_id:u128 remaining:i64 reason:u8` |
| 0x56 | CancelRejected | `stamp client_order_id:u64 reason:u8` |
| 0x57 | PositionChanged | `stamp instrument_id:u128 delta:i64` |
| 0x58 | BalanceChanged | `stamp delta:i128` |
| 0x59 | KillSwitchActivated | `stamp` |
| 0x5a | OrderReduced | `stamp client_order_id:u64 order_id:u128 remaining:i64 reference:option<u64>` |
| 0x5b | KillSwitchReset | `stamp` |
| 0x5c | RunStatus | `stamp status:string` (`active`, `paused`, `terminated`, …) |
| 0x5d | ListingHalt | `stamp instrument_id:u128 halted:bool` |
| 0x5e | News | `stamp news_id:u128 headline:string body:string` |
| 0x60 | MarketSnapshot | `request_id:u32 listing next_report:u64 bids:group<price:i64 quantity:i64> asks:group<…>` |
| 0x61 | MarketUpdate | `request_id:u32 listing first_report:u64 entries:group<kind:u8 price:i64 quantity:i64>` |
| 0x62 | OrderSnapshot | `request_id:u32 listing next_report:u64 complete:bool orders:group<side:u8 reference:u64 price:i64 quantity:i64 broker:option<u128> queue_position:u32>` |
| 0x63 | OrderUpdate | `request_id:u32 listing first_report:u64 entries:group<order entry>` |
| 0x64 | TapeSnapshot | `request_id:u32 instrument_id:u128 last_report:u64 quotes:group<venue_id:u128 side:u8 price:i64 quantity:i64>` |
| 0x65 | TapeUpdate | `request_id:u32 instrument_id:u128 first_report:u64 entries:group<venue_id:u128 kind:u8 price:i64 quantity:i64>` |
| 0x70 | Listings | `request_id:u32 listings:group<listing symbol:string>` |
| 0x71 | OpenOrders | `request_id:u32 run_sequence:u64 orders:group<client_order_id:u64 order_id:u128 listing side:u8 price:i64 original:i64 remaining:i64>` |
| 0x72 | Account | `request_id:u32 run_sequence:u64 cash:group<currency_id:u128 balance:i128 reserved:i128> positions:group<instrument_id:u128 position:i64 open_buy:i64 open_sell:i64>` |
| 0x73 | Deals | `request_id:u32 run_sequence:u64 tenders:group<tender> negotiations:group<negotiation>` |
| 0x74 | Tender | `stamp tender` |
| 0x75 | Negotiation | `stamp negotiation` |

A `tender` is `tender_id:u128 participant_id:u128 instrument_id:u128 side:u8
quantity:i64 price:i64 expires_at_ns:u64 status:u8` (`side` is the
participant's side if it accepts; status 1 open, 2 accepted, 3 declined,
4 expired). A `negotiation` is `negotiation_id:u128 proposer_id:u128
counterparty_id:u128 instrument_id:u128 side:u8 quantity:i64 price:i64
expires_at_ns:u64 status:u8 awaiting:u128` (`side` is the proposer's;
status 1 proposed, 2 countered, 3 accepted, 4 rejected, 5 broken, 6 expired;
`awaiting` the participant whose answer is due). `OtcDecision.decision`
1 accept, 2 reject, 3 break.

Enumerations: `role` 1 participant, 3 instructor, 4 administrator (2 team
and 5 built-in agent are reserved). `resume` 0 live, 1 replaying, 2 gap.
`liquidity` 1 maker, 2 taker. Entry `kind` 0 trade, 1 bid, 2 ask (a level's
absolute visible quantity after the change, 0 when it is gone). Reject
`reason` codes 1–23 mirror the engine's `RejectCode` in order
(`packages/bnp-wire/src/lib.rs`, `RejectReason`); 24 is an unknown listing,
25 a closed session, 26 a market, IOC or FOK order in a call phase and (1.1) 27 a limit price off the tick grid
(ADR 0037).
An order entry starts with `kind:u8`: 0 is a trade,
`price:i64 quantity:i64 resting_reference:option<u64> buyer:option<u128> seller:option<u128>`;
1 add, 2 change, 3 delete are an order,
`side:u8 reference:u64 price:i64 quantity:i64 broker:option<u128>` (quantity
displayed after the change, 0 when deleted). `AccessDelay.source` 0 none,
1 kernel, 2 probes, 3 both.
Cancel `reason` 1 requested, 2 kill switch, 3 market remainder, 4 mass cancel,
5 expired, 6 halt. The engine currently emits no `PositionChanged` or
`BalanceChanged`; clients read positions and cash with `AccountRequest`.

`Reject` answers a request the venue did not admit or could not commit (rate
limit, bad feed, commit failure). `request_type` is the request's type byte
and `reference` its client order ID or request ID.

Operator `action` codes and the fields that follow them: 1 start, 2 pause,
3 resume, 9 score (none); 4 advance `steps:u32`; 5 terminate
`reason:string`; 6 halt `instrument_id:u128 halted:bool reason:string`
(`halted` false reopens); 7 mass cancel `participant_id:option<u128>
instrument_id:option<u128>`; 8 publish news `news_id:u128 audience:u8 …
headline:string body:string`, where audience 0 is public, 1
`participant_id:u128`, 2 `team_id:u128`, 3 `role:string`; 10 fine
`participant_id:u128 currency_id:u128 amount:i128 reason:string` (minor
units); 11 open tender `tender_id:u128 participant_id:u128
instrument_id:u128 side:u8 quantity:i64 price:i64 expires_at_ns:u64`.

## Session

1. Client connects, completes TLS 1.3 and sends `Hello` within
   `handshake_timeout_ms`; every read waits only until that deadline. Frames
   pipelined after `Hello` are processed after `Welcome`. Until `Hello`
   arrives from a registered certificate the connection holds no session
   slot: it counts against an unauthenticated budget of 256 connections, at
   most 32 per address, and connections beyond it are closed before TLS.
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
  (never zero). New orders, cancels, kill switches and kill-switch resets
  use separate command namespaces, so no two sessions or interfaces can collide.
- `CancelOrder` names the client order ID. A cancel for an unknown or another
  participant's order yields `CancelRejected`.
- `KillSwitch` is per participant: the venue cancels every live order of the
  sender's participant on every venue (`OrderCanceled` reason 2), answers
  `KillSwitchActivated`, and rejects that participant's new orders until it
  sends `KillSwitchReset` (answered `KillSwitchReset`). Other participants
  are not affected. It is Bunting-added, like a broker's or venue's
  participant kill switch; FIX v1 has no equivalent message.
- `OrderRested` and `OrderReduced` carry the order's public `reference` on
  the venue's order-by-order feed while it rests and displays quantity (as
  OUCH's accepted message carries the order reference number ITCH shows), read
  right after the commit. A different reference in a later `OrderReduced`
  means the order lost time priority: an iceberg slice was refreshed at the
  back of its level. With an order-by-order subscription the client finds its
  own place in the queue from it. Reports replayed on resume carry the
  reference they had when first sent; after a venue restart there are none to
  replay (slice 44).
- `OrderAccepted` and `OrderDone` are sent only for orders entered over BNP;
  orders the same participant entered over FIX reach BNP as their fills,
  cancels and rests.
- Every order travels to its listing's venue and waits `L(team, venue)` there
  before the sequencer can commit it; kill switches and the request messages
  (`Listings`, `OpenOrders`, `Account`) and kill-switch resets are applied at
  the hub. Reports leave
  the venue after commit and arrive after `L(venue, team)`.

## Operator commands and the run stream (ADR 0042)

- `OperatorCommand` from an instructor or administrator becomes the engine's
  simulation command, authorized exactly as FIX's `UA` operator messages, and
  is applied at the hub through the same admission sequencer (it waits
  `L(operator, hub)`). The answer is `OperatorDone` with the committed run
  sequence, or `Reject` (`request_type` 0x38, `reference` the request ID)
  when it is refused or the engine does not commit it. A participant's
  `OperatorCommand` is refused.
- Every session, participants and operators alike, receives the run's
  public facts in its private stream as they commit, stamped with their
  event sequence: `RunStatus` for lifecycle changes, `ListingHalt` when an
  instrument is halted or reopened on every venue, and `News` for items
  whose audience includes the session's actor (public, its participant,
  team or role). They are ordered with private reports, cross
  `L(hub, team)` like them, and are replayed on resume.
- Cashflow and facility commands have no BNP layout.

## Tenders and OTC negotiations (ADR 0043)

- A team answers a tender with `TenderDecision` and negotiates with
  `OtcPropose`, `OtcCounter` and `OtcDecision`. Each is applied at the hub
  through the admission sequencer (it waits `L(team, hub)`) and answered
  `DealDone` with the committed run sequence, or `Reject` (`request_type`
  the message type, `reference` the request ID). Request IDs are unique per
  participant for the run; a reused one commits nothing new. Operators may
  not send them.
- `OtcPropose.negotiation_id` is the client's own non-zero ID; the venue's
  negotiation ID is `namespace(run, participant) << 64 | negotiation_id` in
  the `negotiation` domain, and every later message and update uses that
  u128 ID.
- Every committed change to a tender or negotiation, including expiry at
  its deadline, is sent as `Tender` or `Negotiation` with its state at that
  commit to the tender's participant or the negotiation's two parties and
  to every operator session, stamped in the private stream and replayed on
  resume. `DealsRequest` lists every visible tender and negotiation.
- FIX carries tender decisions but no OTC messages.

## BNP 1.1 (ADR 0054, slice R1)

1.1 adds message types only; every 1.0 body is byte-identical.

- **Version encoding:** `Hello.version` and `Welcome.version` are
  `major × 256 + minor`. The legacy `1` means 1.0 and 1.1 is `257`.
- **Negotiation:**
  - the client offers the highest 1.x it speaks;
  - the venue answers with the lower minor;
  - another major is logged out with "unsupported BNP version …";
  - the reference client (`bunting-client`) retries once at `1`
    after that, so it still joins a 1.0 venue.
- **A session negotiated at 1.0:**
  - never receives a 1.1 message type or a 1.1-only enum value;
  - receives reject reason 27 as 11 (`PriceOutOfBounds`);
  - is logged out if it sends a 1.1 request, as for any unknown
    type.

| Type | Name | Direction | Body |
|---|---|---|---|
| 0x34 | ReferenceDataRequest | client | `request_id:u32 after_instrument:option<u128> limit:u16` |
| 0x76 | ReferenceData | server | `request_id:u32 run_sequence:u64 next_after:option<u128> currencies:group<currency> instruments:group<instrument> listings:group<listing>` |

- `currency`: `currency_id:u128 code:string name:string minor_decimals:u8`.
- `instrument`: `instrument_id:u128 symbol:string kind:u8
  underlying:option<u128> strike:option<i64> expiry_ns:option<u64>
  is_call:bool physical_delivery:bool coupon_bps:u32
  components:group<instrument_id:u128 weight:i64> multiplier:i64
  currency_id:u128 quoted_decimals:u8 shortable:bool tradeable:bool
  window_start_ns:option<u64> window_stop_ns:option<u64>
  display_unit:string description:string opening_mark:option<i64>
  max_order_quantity:option<i64>`.
- **`kind` codes:** 1 equity, 2 currency, 3 bond (`coupon_bps`, maturity
  in `expiry_ns`), 4 option, 5 future, 6 commodity, 7 synthetic.
- `listing`: `listing symbol:string min_price:i64 max_price:i64
  tick_size:i64 taker_fee_per_lot:i128 maker_fee_per_lot:i128
  broker_ids:bool primary:bool halted:bool in_window:bool`.
- **Paging:**
  - the request asks for instruments after `after_instrument`, in ID
    order; `limit` is 1..=64, and 0 means 64;
  - `next_after` is present iff more remain;
  - currencies come only on the first page;
  - `listings` are this page's instruments' listings.
- **Where it runs:** the job runs at the hub, like `ListingsRequest`.
- **Display:** `quoted_decimals` is display only: a price is shown as
  `ticks / 10^quoted_decimals`. Money is shown at the currency's
  `minor_decimals`.
- **Not yet published:**
  - currency `code` and `name` stay empty until a scenario declares a
    currency table (ADR 0054 §10);
  - `tradeable` is true, the window absent and `in_window` true until
    ADR 0055.
- **Ticks:** a limit price that is not a multiple of the listing's
  `tick_size` is rejected with reason 27, `PriceNotOnTick`.
- **FIX:** the SecurityList (`y`) that answers `x` carries the same
  fields in a `NoRelatedSym` (146) group: 55, 48, 207, 167, 15, 231,
  969 (ticks) and BuntingQuotedDecimals (10023).

### Enrolment by invite (ADR 0056)

| Type | Name | Direction | Body |
|---|---|---|---|
| 0x06 | Enroll | client | `secret:string csr_pem:string` |
| 0x4e | Enrolled | server | `participant_id:u128 certificate_pem:string ca_pem:string` |

- **Which sessions:** a venue with `bnp.enrolment` also accepts a TLS
  client without a certificate. That session's first and only message
  must be `Enroll`; anything else is logged out.
- **Success:** `Enrolled`, then `Logout` ("enrolled; reconnect with your
  certificate").
- **Refusal:** `Logout` ("invite refused"), whatever the reason (unknown,
  used or expired invite, participant already enrolled, bad request).
- **On a certified session,** `Enroll` is logged out.
- **The invite** is `bnpi1:<host:port>:<ca-sha256-hex>:<secret-hex>`. The
  client accepts the venue's chain only if it ends in the pinned CA,
  before sending the secret (`bunting_client::enroll`).

## Private stream and resume

- Every private report carries the committed event `sequence`, and the
  venue sends them in sequence order: a report whose path is shorter than an
  earlier one's waits for it, as on one ordered connection. A client's
  cursor is therefore the highest sequence it processed, and resuming after
  it never skips a report.
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
- With flag bit 3 (`ORDERS`) the feed is order by order (ADR 0036 §7): the
  snapshot (`OrderSnapshot`) lists every displayed order, bids then asks in
  matching priority, with its anonymous public reference, its 1-based
  `queue_position` on its side and, on venues that publish them, its
  owner's broker ID. A book too large for one frame arrives in consecutive
  parts; the last has `complete` set and updates follow only after it.
  Each commit then sends an `OrderUpdate`: trades first, each naming the
  resting order it executed against (with broker IDs where published), then
  orders deleted, changed (keeping reference and place) and added. A public
  reference is the order's time priority in its book, unrelated to any
  order ID, and new whenever the order loses priority (an iceberg refresh
  is a delete and an add). Only displayed quantity is shown.
- A `Subscribe` whose listing has `venue_id` 0 is the instrument's
  consolidated tape (ADR 0036 §6). It travels to the processor at the hub;
  `TapeSnapshot` lists each venue's best bid and offer as the processor
  sees them, with `last_report` the last tape report included. `TapeUpdate`s
  continue with the next report: every venue's trades and every change of a
  venue's best bid or offer (quantity 0: the venue has none on that side),
  each naming its venue, numbered by one per-instrument sequence shared by
  every subscriber (FIX included). It leaves the hub and crosses `L(hub,
  team)`, so near a venue it is slower than that venue's direct feed. The
  tape always carries quotes and trades; `ORDERS` is refused.
- At most 32 subscriptions per session, all kinds together.
- `AccessDelay` is a session message (never delayed) sent when the venue's
  measurement of this connection changes: the smallest round trip seen
  (kernel TCP and application probes) and half of it. It is published only
  and never orders commands (ADR 0035).

## Limits

Server configuration (`bnp` block, `apps/bunting-server/src/config.rs`):
`heartbeat_ms` 100–60,000; `max_frame_bytes` 1,024–65,536;
`max_messages_per_interval` per `rate_limit_window_ms` (1–60,000), a
per-session rate limit: excess requests get `Reject`, session messages are
never limited, and nothing is reordered;
`handshake_timeout_ms` 100–60,000; `max_connections` at most the roster size.
A slow reader that falls 4,096 committed batches behind, or whose socket
accepts no bytes for three heartbeat intervals, is disconnected and resumes
with its cursor.
