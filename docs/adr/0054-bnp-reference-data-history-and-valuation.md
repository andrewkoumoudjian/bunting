# ADR 0054: BNP reference data, history and valuation

- Status: **Accepted in part (2026-10-22)**, as follows.
  - **§4 Valuation is rejected.** That covers `Valuation`,
    `ValuationUpdate`, `ValuationHistory` and NLV history, FIX `AN`/`AP`
    figures and `U9` P&L history, and slice R3. Owner, 2026-10-22: "its
    the app that should determine realized and unrealized P&L … why is
    the venue calculating Pnl ???". The trading apps compute cost, and
    realized and unrealized P&L, from their own fills. They keep those
    books across a relaunch by resuming the private stream (see
    `bunting-trading-state` `LedgerRecord`). The engine ledger still
    values positions for scoring, unchanged and not published per
    participant.
  - Type bytes `0x36`, `0x78`, `0x5f`, `0x37` and `0x79` stay
    unassigned.
  - **Everything else is accepted:** version negotiation, reference
    data, history, trader/case information, transactions, interfaces,
    FIX parity, config, and slices R1, R2, R4 and R5 without NLV
    history.
  - Every behaviour stays **Target** until a slice is recorded in
    `docs/implementation-log/`. R1 is being built first (2026-10-22).
- Date: 2026-10-20
- Evidence baseline: `main@e027402` (this ADR's branch point). Code
  claims below name the file they were read from at that commit.
- Depends on: ADR 0018 (one engine, one risk path), ADR 0028 (single
  ledger, full replay), ADR 0031 (FIX + BNP only, FIX/BNP parity),
  ADR 0035 (latency, no modes), ADR 0036 (public feeds), ADR 0037 (one
  run clock), ADR 0040 (BNP v1), ADR 0042 (operators and run stream),
  ADR 0043 (tenders and OTC), ADR 0053 §2–§3 (projections rebuildable
  from committed records; durable catch-up).
- Companion: [ADR 0055](0055-settlement-close-out-and-portfolio-limits.md)
  (settlement, close-out, tradable windows, portfolio limits). Several
  fields below only become non-empty when ADR 0055 lands; the wire
  shapes are decided here once so ADR 0055 needs no second wire change.
- Wire contract when implemented: `docs/specs/bnp-v1.md` gains a
  "BNP 1.1" section; the codec in `packages/bnp-wire/src/codec.rs`
  remains the contract, as today.

## Context

The desk's RIT-parity contract
([`docs/specs/rit-desk-window-parity.md`](../specs/rit-desk-window-parity.md),
"Protocol gaps") lists six things BNP v1 cannot carry: instrument
reference data, history before the desk connected, the venue's own
valuation, trader identity and limits, case information, and interface
status. The engine-side companion
([`docs/specs/rit-case-primitives.md`](../specs/rit-case-primitives.md)
§1, §2, §8, §9) adds a typed transaction journal and an intraday NLV
series. Until these land, the desk either lacks RIT columns (Portfolio
Type, Contract size, Cost, NLV, realized/unrealized P&L, VWAP, Volume)
or derives them client-side with a "Derived" label, and the derived
figures can disagree with the figures scoring uses.

Evidence labels follow `AGENTS.md`: **observed** (proved by recorded
source below), **inferred**, **Bunting-added**, **unresolved**.

### Observed in Bunting today

1. **BNP `Listings` carries only `listing` + `symbol`**
   (`packages/bnp-wire/src/lib.rs`, `ListingInfo`;
   `packages/bunting-application/src/bnp.rs`, `listings`). The engine
   already holds most of what the desk needs, in
   `packages/bunting-engine/src/lib.rs`:
   `InstrumentDefinition { instrument_id, symbol, currency, kind,
   multiplier, shortable, opening_mark }` and
   `ListingDefinition { key, symbol, price_bounds, fees, broker_ids,
   primary }`. `InstrumentKind`
   (`packages/bunting-engine/src/simulation.rs`) is `Equity`,
   `Currency`, `Bond { coupon_bps, maturity }`,
   `Option { underlying, strike, expiry, is_call }`,
   `Future { underlying, expiry, physical_delivery }`, `Commodity`,
   `Synthetic { components }`.
2. **Observed absent from the engine:** there is no tick-size field
   (prices are integer `PriceTicks`; one tick is one minor unit, see
   `packages/ledger/src/lib.rs` crate docs), no quoted-decimals field,
   no currency code or name (`CurrencyId` is a bare `u128` in
   `packages/market-types/src/lib.rs`), no display unit, no
   description, no tradeable flag and no per-instrument trading
   window. These are scenario-schema additions, not just wire
   additions (§2).
3. **BNP `Account` drops valuation the application already computes.**
   `bunting_application::competition::account` returns an
   `AccountView` with per-instrument `realized_pnl`,
   `unrealized_pnl`, `cost_basis`, `mark`, per-currency `fees`, and
   `net_liquidation_value` in the reporting currency.
   `account_balances` in `bnp.rs` maps that view down to cash
   `balance`/`reserved` and position quantity/open quantities only.
   The ledger (`packages/ledger/src/lib.rs`) is the single source:
   `Position { quantity, reserved, open_buy, open_sell, cost_basis,
   realized_pnl, traded }`, `CashBalance { balance, reserved, fees }`,
   `mark`, `position_value`, `unrealized_pnl`,
   `net_liquidation_value`, and `reporting_value` (FX through
   committed `FxRate` marks). Scoring uses exactly these:
   `RunState::net_liquidation_value` → `reporting_value` (engine
   `lib.rs`), frozen into `DaySettled` scores (`settle_day`) and into
   `IterationReport` by `score_iteration` (`simulation.rs`).
4. **Trade history exists inside the engine but is the wrong source
   for a wire history.** `MarketProjection` (`simulation.rs`) keeps
   per-instrument and per-listing `TradeRecord { logical_time, price,
   quantity }` plus `OhlcBar`s, but it is bounded to
   `MAX_TRADE_HISTORY = 4_096` entries with silent eviction, its bars
   are hard-coded to 1-second run-time buckets, and its records carry
   no committed sequence. ADR 0053 §2 requires history to be
   rebuildable from committed records, never read from mutable state
   as the authority.
5. **The ledger keeps no posting journal.** `Ledger` is balances,
   positions and marks only; `post_cash`, `settle`, `convert`
   mutate in place. The typed facts a transaction log needs exist as
   canonical events (fills, `FineApplied`, `TenderSettled`,
   `OtcSettled`, cashflow applications) in the journaled record
   (`packages/origin-store`, `apps/bunting-server/src/commit_journal.rs`),
   so a transaction log must be a projection, not a second ledger
   (ADR 0028 item 3).
6. **BNP framing forbids in-place body changes.** Trailing bytes,
   unknown message types and wrong-direction messages close the
   session (`docs/specs/bnp-v1.md`, "Framing"; `WireError` in
   `bnp-wire`). `Hello.version` / `Welcome.version` are `u16`,
   `PROTOCOL_VERSION = 1`, and `bnp_host.rs` logs out any other
   version. Group entries are capped at 4,096, strings at 1,024
   bytes, frames at 65,536 bytes. Request/response messages that
   exist (`Listings`, `OpenOrders`, `Account`, `Deals`) are hub jobs
   answered with a `run_sequence` snapshot position
   (`apps/bunting-server/src/bnp_host.rs`); the private stream is
   stamped, ordered, and replayed on resume within the distributor's
   16,384 retained batches (`apps/bunting-server/src/distributor.rs`).
7. **FIX already specifies most equivalents as Target.**
   `docs/specs/bunting-fix-competition-profile.md` names
   SecurityList `y` (tick, lot, bounds, currency), TradeCaptureReport
   `AD`/`AE` (bounded committed trades, time-and-sales, OHLC),
   RequestForPositions `AN` / PositionReport `AP` (NLV, realized/
   unrealized P&L, cost basis, fees, limits), UserRequest `BE` /
   UserResponse `BF` (participant metadata) and `U9` (transaction
   logs, P&L, rankings). ADR 0031 requires FIX/BNP parity for
   participant data, or a recorded reason (§9).
8. **Identity has no name anywhere.** BNP identity is the roster
   entry behind a certificate fingerprint (`BnpRosterEntry {
   certificate_sha256, participant_id, role }`,
   `apps/bunting-server/src/config.rs`); the FIX roster adds a
   username. `ActorIdentity` (`packages/bunting-api-contract`)
   carries `role` and an optional `team_id`, but no display name.
   `ParticipantDefinition` in the engine has limits and opening
   balances, no name. Fines exist as `ApplyFine` →
   `post_cash` debit + `FineApplied` event (`simulation.rs`); there
   is no fines-to-date total anywhere.

### Observed in the RIT reference (paraphrased, nothing copied)

The RIT DMA REST API 1.0.5 resource model (swagger, read 2026-10-20;
summarized in `rit-case-primitives.md` "Sources") is the RIT server's
own list of participant-visible resources. Relevant here, at the level
of resource and field names only:

- a securities resource carrying, per security, type, unit
  multiplier, quoted decimals, min/max price, currency, description,
  display unit, shortable and tradeable flags, start/stop period,
  max trade size, trading fee and limit-order rebate, plus the
  caller's position, NLV, realized and unrealized figures;
- securities history (OHLC bars) and time-and-sales, both paged with
  an `after` cursor and a `limit`;
- a case resource: name, period, total periods, tick, ticks per
  period, status, and whether trading limits are enforced;
- a trader resource (identity, NLV, total fines) and a trader NLV
  history (tick, NLV pairs);
- a limits resource: per limit group, name, gross and net limit,
  current gross and net, and accrued gross/net fines;
- a transactions resource: typed entries (26 transaction type names,
  including order, commission, rebate, tender, endowment, close-out,
  settlement, gross/net limit fines, case fine, dividend, coupon,
  interest, mark-to-market) with quantity, price, cash flow and
  resulting balance, paged with `after` and `limit`.

These shapes are evidence for *what a participant must be able to
see*; they are not a design Bunting copies. Where Bunting's model
differs (multi-venue listings, a run clock in nanoseconds instead of
period/tick, a private stream with resume cursors), this ADR says so.

## Decision

### 1. Version negotiation: BNP 1.1, an additive minor version

This ADR adds messages; it changes no existing message body. That is
a minor version, not BNP v2.

- From 1.1, the `u16` version in `Hello` and `Welcome` is
  `major × 256 + minor`. The legacy value `1` is special-cased and
  means 1.0; 1.1 is written `257`. `PROTOCOL_VERSION` keeps naming
  the server's own version in this encoding; the magic stays `BNP1`
  for every 1.x.
- The client states the highest 1.x it speaks. A 1.1 server answers
  `Welcome` with the negotiated version, the lower of the two
  minors. A client whose major is not 1 still receives `Logout`
  ("unsupported BNP version"), as today. A 1.1 client that meets a
  1.0 server is logged out the same way; the reference client
  (`packages/bunting-client`) retries once stating `1` and then
  behaves as a 1.0 client (its derived-figure labels stay, per the
  desk parity spec).
- A server must never send a 1.1 message type, or a 1.1-only enum
  value inside a 1.0 message, to a session negotiated at 1.0. Pushed
  1.1 messages (§4, §7, §8) are suppressed for such a session, not
  translated. A 1.0 client never sends 1.1 requests; a 1.1 request
  type arriving on a 1.0-negotiated session is a protocol error and
  closes it, like any unknown type today.
- Minor-version rule, binding on future ADRs: a 1.x minor may only
  add message types. Changing a body, reusing a type byte, or
  changing a field's meaning is a major version (BNP v2, new magic),
  decided by its own ADR. This is stricter than slice 46's in-place
  request-ID widening (ADR 0043), which was possible only because no
  external client existed; that exception does not repeat.

New type bytes (client `0x01..=0x3f`, server `0x40..=0x7f`, all
currently unassigned — observed in `msg_type`):

| Type | Name | Direction | § |
|---|---|---|---|
| 0x34 | ReferenceDataRequest | client | 2 |
| 0x76 | ReferenceData | server | 2 |
| 0x35 | HistoryRequest | client | 3 |
| 0x77 | History | server | 3 |
| 0x36 | ValuationRequest | client | 4 |
| 0x78 | Valuation | server | 4 |
| 0x5f | ValuationUpdate | server, pushed | 4 |
| 0x37 | ValuationHistoryRequest | client | 4 |
| 0x79 | ValuationHistory | server | 4 |
| 0x39 | TraderRequest | client | 5 |
| 0x7a | TraderInfo | server | 5 |
| 0x3a | CaseRequest | client | 6 |
| 0x7b | CaseInfo | server | 6 |
| 0x4b | CaseChanged | server, pushed | 6 |
| 0x3b | TransactionsRequest | client | 7 |
| 0x7c | Transactions | server | 7 |
| 0x4c | Transaction | server, pushed | 7 |

All request/response pairs follow the existing pattern: a `u32`
request ID, a hub or venue job as stated per item, a `run_sequence`
snapshot position in the answer, and `Reject` (request type +
reference) for a refused or oversized request. Field encodings reuse
`bnp-v1.md` exactly (`i64` ticks/lots, `i128` minor units, `u128`
identities, `option<T>`, bounded `string` and `group<T>`).

### 2. Instrument reference data: a new message, not a wider `Listings`

`Listings` stays byte-identical: it is the 1.0 symbol directory and
the desk's bootstrap. Reference data is a separate, paged request,
because (a) widening `Listings` would break 1.0 decoding, (b)
reference data is mostly per *instrument* while `Listings` is per
*listing*, and (c) with descriptions it cannot be guaranteed to fit
one 64 KiB frame (`MAX_INSTRUMENTS = 256`, `MAX_LISTINGS = 64`,
engine `lib.rs`).

| Type | Name | Body |
|---|---|---|
| 0x34 | ReferenceDataRequest | `request_id:u32 after_instrument:option<u128> limit:u16` |
| 0x76 | ReferenceData | `request_id:u32 run_sequence:u64 next_after:option<u128> currencies:group<currency> instruments:group<instrument> listings:group<listing>` |

- `currency` is `currency_id:u128 code:string name:string
  minor_decimals:u8`.
- `instrument` is `instrument_id:u128 symbol:string kind:u8
  underlying:option<u128> strike:option<i64> expiry_ns:option<u64>
  is_call:bool physical_delivery:bool coupon_bps:u32
  components:group<instrument_id:u128 weight:i64> multiplier:i64
  currency_id:u128 quoted_decimals:u8 shortable:bool
  tradeable:bool window_start_ns:option<u64>
  window_stop_ns:option<u64> display_unit:string
  description:string opening_mark:option<i64>
  max_order_quantity:option<i64>`.
- `listing` is `listing symbol:string min_price:i64 max_price:i64
  tick_size:i64 taker_fee_per_lot:i128 maker_fee_per_lot:i128
  broker_ids:bool primary:bool halted:bool in_window:bool`.

`kind` codes: 1 equity, 2 currency, 3 bond, 4 option, 5 future,
6 commodity, 7 synthetic. Kind parameters ride as the optional
fields above (present only for their kind; `coupon_bps` 0 and
`components` empty when not applicable), keeping one fixed layout.
`in_window` and `halted` are the listing's state at `run_sequence`;
after that, the client derives window state from
`window_start_ns`/`window_stop_ns` and the logical time in private
stamps, and halt state from `ListingHalt` as today. ADR 0055 §3 adds
the run-stream window edge so the desk need not poll.

Where each field comes from:

| Field | Source today | Change needed |
|---|---|---|
| kind + parameters, multiplier, currency, shortable, opening mark | `InstrumentDefinition`, `InstrumentKind` (observed) | none |
| price bounds, fees, broker IDs, primary | `ListingDefinition` (observed) | none |
| tick size | implicit 1 tick (observed) | new `tick_size` on `ListingDefinition` beside `price_bounds`, default 1, published per listing (tick regimes are a venue property). The slice that adds the field also enforces it: risk admission refuses a limit price not divisible by the tick, with an appended reject code. "Integer-only quotes" (RIT Electricity, observed) is then tick 100 at 2 quoted decimals, not a special mode |
| quoted decimals | none | new per-instrument display field, default 2. Display only: it never changes tick arithmetic |
| currency code, name, minor decimals | none (`CurrencyId` only) | new scenario `currencies` table (§10). Until a scenario declares one, code and name are empty and minor decimals 2; the desk shows the ID |
| tradeable flag, trading window | none | ADR 0055 §3 (`tradeable`, `trading_window` on `InstrumentDefinition`). Until then: `tradeable = true`, window absent |
| display unit, description | none | new optional instrument fields, default empty; scenario validation caps each at 256 bytes so a page stays in frame |
| per-instrument max order size | none (per-participant only) | ADR 0055 §5. Until then: absent |

Bounds: `limit` 1..=64 instruments per page (0 means 64); pages are
in instrument-ID order, `next_after` present iff more remain; the
`currencies` group appears only on the first page (empty after);
`listings` carries exactly the listings of the page's instruments.
Reference data is immutable for a run except `halted`/`in_window`,
so a client requests it once after `Listings` and caches it per
`run_id`. The job runs at the hub, like `ListingsRequest`.

### 3. History from run start: trades and bars from committed records

| Type | Name | Body |
|---|---|---|
| 0x35 | HistoryRequest | `request_id:u32 listing kind:u8 after_sequence:option<u64> after_time_ns:option<u64> limit:u16 bar_interval_ns:u64` |
| 0x77 | History | `request_id:u32 listing kind:u8 through_sequence:u64 earliest_sequence:u64 complete:bool next_after_sequence:option<u64> trades:group<trade> bars:group<bar>` |

- `kind` 1 time-and-sales, 2 OHLC bars. Exactly one group is
  non-empty, matching `kind`.
- A `trade` is `sequence:u64 logical_time_ns:u64 price:i64
  quantity:i64` — one committed trade on that listing, in sequence
  order. `sequence` is the committed event sequence of the trade's
  batch (the `stamp.sequence` space clients already use).
- A `bar` is `bucket_start_ns:u64 open:i64 high:i64 low:i64
  close:i64 volume:i64 price_quantity_sum:i128 trade_count:u32`.
  Buckets are aligned to run time 0 at the requested
  `bar_interval_ns`. `price_quantity_sum` is Σ(price_ticks × lots)
  in exact integer arithmetic, so the desk computes VWAP as
  `price_quantity_sum / volume` ticks with no venue-side rounding
  (Bunting-added exactness; RIT publishes a rounded VWAP, observed).
  Days with no trade produce no bar; there is no forward fill.

Rules:

- **Bounded and paged.** `limit` is 1..=1,024 entries (0 means 256).
  Trades page by `after_sequence`; bars page by `after_time_ns`
  (bucket start strictly after it). `next_after_sequence` is present
  for trades iff more trades exist at `through_sequence`. A request
  past the end answers an empty group, not an error. History
  requests count against the existing per-session rate limit; there
  is no separate history quota.
- **Bar intervals are whitelisted** to bound server work:
  1 s, 5 s, 15 s, 60 s, 300 s and 3,600 s of *run time*
  (`bar_interval_ns` must equal one of these for `kind` 2 and must
  be 0 for `kind` 1, else `Reject`). The desk's bar-size picker maps
  onto this set in 1.1; arbitrary intervals are a later minor if a
  case needs them.
- **Served from a journal-rebuildable projection, never from
  `MarketProjection`.** The venue builds a per-listing public-trade
  projection from committed batches (the same committed records the
  direct feeds project, ADR 0036 §2, ADR 0053 §2), retained from run
  start. The engine's in-state `MarketProjection` stays what it is
  (a bounded simulation projection) and is not read for history.
  Bars are computed from the trade projection for the requested
  interval, so both kinds agree by construction.
- **Retention bound, stated rather than silent.** Server config
  `bnp.max_history_trades_per_listing` (also applied to FIX, §9)
  caps retained trades per listing; the implementation slice
  proposes the default from a recorded workload, and this ADR
  requires the cap to be at least what a full classroom run
  produces (the slice must measure, not guess). When the cap is
  exceeded, the oldest trades drop and `earliest_sequence` advances;
  `complete` is then false for any page that starts before the
  retained floor, so a client can never mistake a truncated series
  for the whole run. The same flag covers bars: bars whose bucket
  contains dropped trades are not served as if exact — the answer
  starts at the first fully retained bucket and `complete` says so.
- **Latency: history can never lead the live feed.** A
  `HistoryRequest` is a market-data request addressed to the
  listing's venue (ADR 0035), exactly like `SnapshotRequest`: it
  travels `L(p, v)`, runs as a job on the sequencer thread at
  release, and its answer leaves `L(v, p)` after the job.
  `through_sequence` is the run's committed sequence when the job
  runs. Every trade in the answer was therefore already sent on
  that venue's direct feed to a subscriber at the same location no
  later than this answer leaves; history reveals only public,
  already-published facts, later than live publication. There is no
  extra "historical delay" and no exemption from the latency map.
- **Recovery.** After a venue restart the projection is rebuilt
  from the journal (checkpoint plus journaled inputs, through
  `RunRecovery`) before history is served; while it rebuilds,
  `HistoryRequest` is answered `Reject` ("history rebuilding"),
  never a partial series presented as complete. Rebuild
  correctness is ADR 0053 §2's invariant applied to this
  projection: `Project(checkpoint, committed ≤ k)` must reproduce
  the same trades. History is a snapshot read, so resume semantics
  do not apply; a reconnecting desk re-requests what its charts
  need and continues live feeds from their own snapshots.

### 4. Valuation: the scoring figures, on request and pushed

| Type | Name | Body |
|---|---|---|
| 0x36 | ValuationRequest | `request_id:u32` |
| 0x78 | Valuation | `request_id:u32 valuation` |
| 0x5f | ValuationUpdate | `stamp valuation` |
| 0x37 | ValuationHistoryRequest | `request_id:u32 after_time_ns:option<u64> limit:u16` |
| 0x79 | ValuationHistory | `request_id:u32 run_sequence:u64 complete:bool points:group<logical_time_ns:u64 sequence:u64 reporting_nlv:i128>` |

A `valuation` is `run_sequence:u64 reporting_currency:u128
reporting_nlv:i128 cash:group<cash> positions:group<position>`:

- `cash` is `currency_id:u128 balance:i128 reserved:i128 fees:i128
  nlv:i128` (this participant's NLV **in that currency**, from
  `Ledger::net_liquidation_value`).
- `position` is `instrument_id:u128 position:i64 open_buy:i64
  open_sell:i64 mark:option<i64> cost_basis:i128 realized_pnl:i128
  unrealized_pnl:i128 traded:i64`.

Rules:

- **One valuation path.** Every figure is read from the engine in a
  hub read closure (`OriginStore::read_run`, never `clone_run` on
  this path, per `AGENTS.md`) through the same functions scoring
  uses: `competition::account` / `Ledger` accessors /
  `RunState::net_liquidation_value` (§Context 3). No adapter
  recomputes P&L; the desk's "Derived" labels come off when this
  lands. `Valuation` is a strict superset of `Account`;
  `Account`/`AccountRequest` remain for 1.0 and for rebuild-after-gap
  exactly as today.
- **Cost per instrument.** `cost_basis` is the ledger's signed
  average-cost basis in minor units. Average cost *price* is
  `cost_basis / (position × multiplier)`, which is not in general
  an integer tick count, so the wire carries the exact basis and
  the client renders the quotient; the venue never publishes a
  rounded price as exact. `realized_pnl` excludes fees (ledger
  semantics, observed); fees are visible per currency in `fees`
  (negative when rebates exceed fees) and per fill in `Fill`, so
  the desk can show RIT's "realized including commissions" as a
  labelled sum of venue figures, not a derivation.
- **Marks are the engine's marks:** last trade on any listing,
  opening mark before any trade, primary listing's official close
  after `DaySettled` (ADR 0037 §4). A position in an instrument with
  no mark is carried at cost, as `position_value` does today, and
  `mark` is absent so the desk can show that it is at cost.
  RIT's Portfolio NLV column (value at best bid/ask, observed in the
  desk parity spec) is a *desk* figure from live books and stays
  labelled Derived; it is not the venue's NLV and this ADR does not
  make it one.
- **Pushed at a bounded rate, stamped but coalescible.**
  `ValuationUpdate` is sent on the private stream when a committed
  batch changed this participant's cash, positions, or a mark of an
  instrument they hold, at most once per
  `bnp.valuation_push_ms` of venue time (new config, 100..=60,000;
  0 disables pushes, leaving request-only). It carries the stamp of
  the latest batch it reflects so the client can order it against
  fills, with one recorded exception to the private-stream
  contract: it is a snapshot projection, so an unsent update may be
  superseded by a newer one for a slow session, and on resume at
  most one (the latest at or before the replay end) is sent inside
  the replay. No cursor can stall on a coalesced update:
  `ReplayComplete.through_sequence` still bounds the replay, and a
  `gap` resume rebuilds with `ValuationRequest`. Pushes cross
  `L(s, p)` from where the batch applied, like every private
  report; the rate bound never reorders them against stamped
  reports. Operators receive no participant's valuation.
- **NLV history is a sampled projection** (RIT `/trader/history`,
  observed as (tick, NLV) pairs). The distributor samples each
  participant's reporting NLV at most once per 1 second of *run
  time* (bucketed by the batches' logical times), plus a mandatory
  sample at every `DaySettled` and `ScoreIteration`, into the same
  journal-rebuildable projection family as §3. It is bounded by
  `bnp.max_valuation_points_per_participant`; overflow drops the
  oldest points and `complete` reports it. Samples are private:
  only the participant (and operators already able to score the
  run) can request them, and only their own series. Because samples
  are recomputed by replay, a restart reproduces the same points;
  the sampling rule is part of the projection version (ADR 0053 §2)
  and changes to it are versioned, not silent.

### 5. Trader identity, limits and fines

| Type | Name | Body |
|---|---|---|
| 0x39 | TraderRequest | `request_id:u32` |
| 0x7a | TraderInfo | `request_id:u32 run_sequence:u64 participant_id:u128 display_name:string team_id:option<u128> team_name:string role:u8 case_role:string interfaces:u8 this_session:u8 max_order_quantity:i64 max_open_order_quantity:i64 max_absolute_position:i64 max_live_orders:u32 live_orders:u32 cash_constrained:bool limit_groups:group<limit_group> fines:group<currency_id:u128 amount:i128>` |

- A `limit_group` is `group_id:u32 name:string
  gross_limit:option<i64> net_limit:option<i64> gross_used:i64
  net_used:i64 enforcement:u8` (1 refuse at admission, 2 fine).
  `gross_used`/`net_used` are on settled positions at
  `run_sequence`, weighted as ADR 0055 §5 defines — the basis fines
  are assessed on. (Admission uses a worst-case basis including
  open orders; the desk can show that from its own live orders. The
  two bases are named, never conflated.)

Rules:

- **Names live in the roster, not the engine.** Identity in BNP is
  already a roster concern (ADR 0040), so `display_name`, `team_id`
  and `team_name` come from new optional fields on `BnpRosterEntry`
  and, for parity, the FIX roster (§9). They are presentation only:
  a message can still never name a participant, and no engine rule
  reads a name. Team *scoring* aggregation is engine business for
  a later case-structure ADR (`rit-case-primitives.md` §8); when it
  lands, the roster's `team_id` must be reconciled with the
  scenario's teams rather than quietly becoming authority — that
  reconciliation is recorded as Unresolved below. Defaults:
  empty name (desk shows the participant ID), no team.
- **`role` repeats `Welcome.role`; `case_role` is reserved** for
  case roles (Producer/Distributor/Trader — `rit-case-primitives.md`
  §8, Missing today) and is empty until that ADR. Publishing the
  field now avoids a wire change then.
- **Limits are the engine's, read where they are enforced.** The
  scalar fields are the participant's `RiskLimits`
  (`packages/risk-engine/src/lib.rs`) from their
  `ParticipantDefinition`, and `live_orders` is the engine's
  committed live-order count, the same value
  `admit_live_order_count` checks. `limit_groups` is empty until
  ADR 0055 §5 lands, then lists every group with current use.
  Nothing here is computed in the adapter from session state (the
  adapter-held-authority trap in `AGENTS.md` stays fixed).
- **Fines to date** sum every `FineApplied` for the participant
  plus, from ADR 0055, `GROSS_LIMIT_FINE`, `NET_LIMIT_FINE`,
  close-out fines and `CASE_FINE` postings, grouped by currency in
  exact minor units (no FX conversion of a displayed total). The
  sums are part of the §7 transaction projection, so they survive
  restart by replay and always match the transaction log's fine
  rows. RIT's trader resource exposes one total (observed);
  per-currency groups are Bunting-added exactness.
- Only the participant's own `TraderInfo` is ever answered on a
  participant session. Operator sessions receive their own identity
  fields with empty limits/fines, as `AccountRequest` already
  refuses account data to operators.

### 6. Case information, mapped onto the one run clock

| Type | Name | Body |
|---|---|---|
| 0x3a | CaseRequest | `request_id:u32` |
| 0x7b | CaseInfo | `request_id:u32 case` |
| 0x4b | CaseChanged | `stamp case` |

A `case` is `run_sequence:u64 case_name:string scenario_id:u128
scenario_version:u128 iteration_id:u128 status:u8
logical_time_ns:u64 step_ns:u64 tick:u64 period:u32
total_periods:u32 ticks_per_period:u64
period_start_ns:option<u64> period_end_ns:option<u64>
ticks_remaining:option<u64> enforce_limits:bool`.
`status` codes mirror `RunLifecycle` (observed in `simulation.rs`):
0 stopped, 1 active, 2 paused, 3 terminated.

The RIT period/tick vocabulary is an *alias over Bunting's existing
time*, never a second clock (ADR 0037; the clocks trap in
`AGENTS.md`):

- `tick = logical_time_ns / step_ns`, using the run clock's own
  `step_ns` (`LogicalClock`, observed; default 1 ms). News and the
  desk's Tick columns use the same division of stamp times.
- With a trading calendar (ADR 0037 §2, `calendar.rs`): `period`
  is the current trading day, 1-based among non-holiday days;
  `total_periods` is the count of trading days
  (`days` minus `holidays`); `ticks_per_period = day_ns / step_ns`;
  `period_start_ns` is the day's start (`day × day_ns`) and
  `period_end_ns` the next day start, so time remaining is
  `period_end_ns − logical_time_ns` and `ticks_remaining` that
  difference in steps. On a holiday or after the last day, period
  fields name the most recent trading day and remaining is absent.
- Without a calendar: `period = 1`, `total_periods = 0` (unbounded),
  `ticks_per_period = 0`, period bounds and remaining absent.
  `tick` still advances. No synthetic calendar is invented.
- `iteration_id` is the run's `IterationId` (reset by
  `RunState::reset_iteration`, observed) — the Bunting sub-heat
  boundary. Period is the *day*, iteration the *sub-heat*; both are
  published so the later case-structure ADR
  (`rit-case-primitives.md` §8) can re-point the desk's labels
  without a wire change. This mapping is **Bunting-added**; whether
  RIT's "period" should ultimately read as sub-heat is an owner
  question (Unresolved).
- `case_name` comes from a new optional scenario `name` (§10),
  falling back to empty. Publishing the case *brief* itself (desk
  "Case Files" gap) is not in 1.1: a brief does not fit the 1,024-
  byte string bound, and chunked document delivery needs its own
  bounds and integrity story — Unresolved.
- `enforce_limits` is true iff any ADR 0055 limit group has
  refuse-enforcement (RIT's case flag, observed); false until then.
- `CaseChanged` joins the run stream (ADR 0042): stamped, ordered
  with private reports, crossing `L(hub, team)`, replayed on
  resume. It is sent when the distributor maps `LifecycleChanged`,
  `SessionPhaseChanged`, `DayClosed` or `DaySettled` — the events
  that can change any case field. Between changes, the desk
  advances tick and time-remaining from stamp logical times; there
  is no per-tick push. `RunStatus` remains for 1.0 sessions.

### 7. A typed transaction journal

| Type | Name | Body |
|---|---|---|
| 0x3b | TransactionsRequest | `request_id:u32 after_index:option<u64> limit:u16` |
| 0x7c | Transactions | `request_id:u32 run_sequence:u64 earliest_index:u64 complete:bool next_after:option<u64> entries:group<transaction>` |
| 0x4c | Transaction | `stamp transaction` |

A `transaction` is `index:u64 sequence:u64 logical_time_ns:u64
type:u8 instrument_id:option<u128> listing:option<listing>
quantity:i64 price:option<i64> currency_id:u128 amount:i128
balance_after:i128 order_id:option<u128> reference:u128`.
`index` is a per-participant monotone counter starting at 1 (the
paging cursor, like RIT's `after`, observed); `amount` is the
signed cash flow in `currency_id`; `balance_after` is the
participant's cash balance in that currency after the entry (the
Desk Transaction Log's resulting-balance column); `reference`
carries the tender, negotiation, scheduled-action or settlement
instrument identity where one applies, else 0.

Type codes (Bunting-owned; names aligned with the RIT transaction
vocabulary only where the Bunting fact is the same fact):

| Code | Type | Bunting fact projected |
|---|---|---|
| 1 | Order | one exchange fill's notional leg (buyer −, seller +) |
| 2 | Commission | a fill fee > 0 |
| 3 | Rebate | a fill fee < 0 (amount positive credit) |
| 4 | Tender | `TenderSettled` (no commission leg, as today) |
| 5 | Otc | `OtcSettled` for each party |
| 6 | Dividend / 7 Coupon / 8 Interest / 9 Fee / 10 Adjustment | scheduled cashflows by `CashflowKind` (observed in `simulation.rs`) |
| 11 | Settlement | derivative cash settlement (ADR 0055 §1–§2); also `CashflowKind::Settlement` cashflows |
| 12 | CloseOut | close-out flattening (ADR 0055 §4) |
| 13 | CloseOutFine | leftover-position fine (ADR 0055 §4) — Bunting-added split; which RIT type carries it is unresolved |
| 14 | GrossLimitFine / 15 NetLimitFine | limit-fine assessments (ADR 0055 §5) |
| 16 | CaseFine | operator `ApplyFine`, and case fines from the later fines framework |
| 17 | Endowment | scheduled position grants (later ADR); reserved now |
| 18 | OpeningBalance | genesis cash and seeded-position cost basis — Bunting-added, so the log sums from run start |

Rules:

- **A projection of committed events, in commit order** (ADR 0053
  §2). One exchange fill yields an Order entry and, when the fee is
  non-zero, a Commission or Rebate entry, in that order, for each
  party (house legs produce no participant entry). Entries are
  derived only from journaled records, so replay reproduces the
  same indices and running balances; the projection is versioned
  with the others in this ADR. Retention is bounded by
  `bnp.max_transactions_per_participant`; `earliest_index` and
  `complete` expose truncation exactly as in §3.
- **Conservation is testable:** per currency,
  `OpeningBalance + Σ amount = current cash balance`, at every
  `run_sequence`. A projection that cannot satisfy this is a bug,
  not a rounding difference (all values are integers).
- **RIT's MARK_TO_MARKET type has no Bunting entry.** Bunting marks
  move no cash; valuation (§4) carries unrealized figures. The desk
  log is credits and debits (desk parity spec), so no zero-cash
  rows are invented to mimic the name.
- **Live delivery:** each new entry is also pushed as `Transaction`
  on the private stream — stamped, ordered, *not* coalescible (it
  is a log), and replayed on resume within the retained window.
  Beyond the window, `TransactionsRequest` backfills from the
  projection by index; the client deduplicates on `index`. This is
  deliberately more durable than the report window: the projection
  is rebuildable to run start (subject to its retention bound), so
  a desk that reconnects after a `gap` can still rebuild its log.
- `limit` is 1..=512 (0 means 256); the entry size keeps a full
  page inside one frame. Only the participant's own entries exist
  on their session; operators get none (their audit path is the
  journal and the archive, not a participant log).

### 8. Interface permissions, as data only

`TraderInfo.interfaces` is a bitmask of the session kinds this
participant may use: bit 0 BNP from a manual application (the desk,
the TUI), bit 1 BNP from an algorithmic client, bit 2 FIX.
`TraderInfo.this_session` names the current session's kind with the
same numbering (1 manual BNP, 2 algorithmic BNP, 3 FIX — on BNP
only 1 or 2 can occur).

- The kind of a BNP session is declared on its roster entry (new
  optional `kind: manual | algorithmic`, default `algorithmic` for
  existing rosters so today's tooling is unchanged); the same
  participant may hold several roster entries. `interfaces` is the
  union over the participant's roster entries on both transports,
  read from server config at the hub. FIX answers the same union
  (§9) so a FIX algo can see that manual trading exists or not.
- This is **data the client can show** — the desk's API/manual
  indicators (desk parity spec) — and nothing more. No order is
  refused because of it in 1.1. Enforcement (refusing orders from a
  disallowed kind, per case: RIT ALGO cases ban manual trading,
  Electricity disables API orders, observed) belongs to ADR 0055
  or the case-structure ADR, which must also decide whether kind
  is per certificate, per participant, or per case role; 1.1 does
  not prejudge it beyond carrying the bits.

### 9. FIX parity (ADR 0031)

Each item lands on FIX in the same slice as BNP, projecting the
same engine reads and the same server projections, through the
messages the FIX profile already names. FIX values are canonical
decimal strings for wide integers (profile rule); every output
keeps `BuntingCommittedSequence (10010)` and its audience tag.

| Item | FIX surface | Notes |
|---|---|---|
| Reference data | SecurityListRequest `x` / SecurityList `y` | Standard fields first: Symbol (55), SecurityExchange (207), SecurityType (167), Currency (15), ContractMultiplier (231), MinPriceIncrement (969, as integer ticks). Bunting tags only where no standard field fits: quoted decimals, tradeable flag, trading window, display unit, per-instrument max order size. The profile's dictionary gains these tags in the same slice; no JSON payload |
| History | TradeCaptureReportRequest `AD` / TradeCaptureReport `AE` | Same §3 projection, same bounds. `AD` carries the listing (48 + 207), kind (trades or bars), `after` cursor and limit in Bunting tags, since `AD`'s standard date fields cannot name run-time or sequence cursors. Bars ride `AE` with the same interval whitelist. Same venue-addressed latency path as a `V` snapshot |
| Valuation | RequestForPositions `AN` / PositionReport `AP` | The profile already requires `AP` to carry NLV, realized/unrealized P&L, cost basis, fees/rebates and marks with their policy version. Same `competition::account` read as BNP §4 |
| Valuation push | unsolicited `AP` — **deferred one slice, reason recorded** | FIX participants are algorithmic and can poll `AN`; the desk (the push consumer) is BNP. Recorded reason per ADR 0031: request/response parity lands first; unsolicited `AP` at the same cadence follows in the next FIX slice, and must respect the FIX session WAL (ADR 0045) before send. If that slice slips, the reason is re-recorded, not forgotten |
| NLV history | `U9` (P&L history resource) | Same sampled projection and bounds as §4 |
| Trader identity, limits, fines, interfaces | UserRequest `BE` / UserResponse `BF`, limits also in `AP` | `BF` carries display name, team, role, interface bits and fines; limit groups ride `AP` as the profile's "limits, warnings, penalties and risk groups" |
| Case information | `BF` (case resource) + lifecycle in `y` | Same period/tick alias formulas. No FIX `CaseChanged` push: FIX sessions already poll `x`/`BE` (observed, ADR 0042 context); recorded as the parity exception for pushes, same condition as valuation push |
| Transactions | `U9` (transaction-log resource) | Same §7 projection, types, indices and paging; types are integer codes in a Bunting tag, not free text |

The FIX/BNP parity test (ADR 0031 validation 4) is extended: the
same run read over both interfaces must yield identical reference
fields, history entries, valuation figures, limits, case fields
and transaction rows (modulo encoding), for the same
`run_sequence`.

### 10. Scenario and server-config changes

Scenario schema (coordinated with ADR 0055 §7, which performs the
version bump if it lands first; whichever slice first touches the
schema does the bump and the other rides it):

- New optional scenario `name` (case name, ≤ 128 bytes).
- New optional scenario `currencies` table:
  `{ currency_id, code, name, minor_decimals }`, validated unique
  per ID; code ≤ 8 bytes, name ≤ 64 bytes. Currencies used by
  instruments but absent from the table publish empty code/name
  (no invented ISO mapping from bare IDs).
- New per-instrument optional `quoted_decimals` (default 2),
  `display_unit`, `description` (each ≤ 256 bytes).
- New per-listing optional `tick_size` (default 1, must be ≥ 1 and
  divide the price-bounds span sensibly: validation requires
  `min % tick_size == 0`; admission enforces divisibility from the
  slice that adds it).
- All new fields are additive with defaults, so a scenario that
  predates them loads with today's behaviour; the loader rules are
  ADR 0055 §7's.

Server config (`bnp` block, `apps/bunting-server/src/config.rs`):
`valuation_push_ms`, `max_history_trades_per_listing`,
`max_transactions_per_participant`,
`max_valuation_points_per_participant`, each validated positive
(push may be 0 = off) with defaults set by the implementation slice
from recorded workloads. Roster entries (BNP and FIX) gain optional
`display_name`, `team_id`, `team_name`, and BNP entries gain
`kind`. Config that predates them loads unchanged.

### 11. Recovery and resume, consolidated

- Request/response messages (§2–§7) are stateless snapshots at a
  `run_sequence`; disconnect loses nothing and resume does not
  replay them. A client that resumes with `gap` rebuilds with
  `ReferenceDataRequest` (cacheable), `OpenOrdersRequest`,
  `ValuationRequest`, `CaseRequest`, `TraderRequest` and paged
  `TransactionsRequest`/`HistoryRequest` — supersets of today's
  `AccountRequest` rebuild, which keeps working for 1.0.
- Pushed messages split in two classes, named here so the codec
  docs can state them once: **log-class** (`Transaction`,
  `CaseChanged`) are stamped, ordered, never coalesced, replayed
  in the retained window like fills; **snapshot-class**
  (`ValuationUpdate`) is stamped for ordering but may be coalesced
  and replays at most once (§4). No other message changes class.
- After a venue restart, §3/§4/§7 projections rebuild from the
  journal before serving (ADR 0053 §2–§3); engine-read messages
  (§2, §4 request, §5, §6) serve as soon as the run is recovered,
  like `AccountRequest` today. Nothing in 1.1 is served from
  process-local caches that a restart silently empties.

### 12. Proposed slices

Slice numbers are assigned when they land; the labels are for
ordering. Each slice is engine/read-model first, BNP and FIX in the
same slice, `bunting-trading-state` model support and desk columns
labelled until their data arrives (desk parity spec acceptance).

1. **R1 — Version + reference data.** 1.1 negotiation, the suppress-
   for-1.0 rule, scenario/config fields of §10 (with ADR 0055 §7's
   bump if it has not landed), tick-size enforcement,
   `ReferenceData`, FIX `y` fields. Desk Portfolio Type/Contract
   size columns unblocked.
2. **R2 — Public history.** The journal-rebuildable trade
   projection, `History`, FIX `AD`/`AE`, restart rebuild. Desk
   charts, VWAP and Volume from run start.
3. **R3 — Valuation.** `Valuation` + `ValuationUpdate`, FIX `AN`/`AP`
   figures. Desk Portfolio Cost/NLV/realized/unrealized become
   venue figures; "Derived" labels removed for them.
4. **R4 — Trader, case, interfaces.** Roster name/team/kind,
   `TraderInfo` (groups empty until ADR 0055), `CaseInfo` +
   `CaseChanged`, FIX `BE`/`BF`. Desk status bar and Trader Info.
5. **R5 — Transactions and NLV history.** The transaction
   projection (all types this ADR can already produce; ADR 0055
   types light up when it lands), live `Transaction`, fines totals
   in `TraderInfo`, `ValuationHistory`, FIX `U9`. Desk Transaction
   Log and P&L chart.

R1–R5 are independent of ADR 0055's engine work except where fields
are reserved; the joint ordering constraint is only §10's schema
bump, taken by whichever lands first.

## Consequences

- The desk's six protocol gaps close with venue figures: the NLV
  and P&L a participant sees are bit-identical to the ones
  `DaySettled` and `ScoreIteration` freeze, because they are the
  same function calls on the same ledger.
- BNP gains a durable minor-version discipline. 1.0 algorithmic
  clients keep working against a 1.1 venue and simply never see
  1.1 traffic; the cost is a negotiation branch in `bnp_host` and
  a retry in the reference client.
- Three new server projections (public trades, transactions, NLV
  samples) join the distributor. They are memory the venue did not
  spend before; each is bounded by config with the bound visible
  to clients (`complete`, `earliest_*`), and each is discardable
  and rebuildable — none is authority (ADR 0053 §2). Their rebuild
  cost after restart is journal length; that cost is already paid
  for run recovery, but the projections must ride the same replay,
  not a second journal pass, and the R2 slice must show that.
- `ValuationUpdate` is the first coalescible stamped message. The
  exception is recorded in §11 and must be stated in `bnp-v1.md`;
  clients that treat every stamp as exactly-once (correct for
  everything else) must special-case this one type.
- FIX reaches the profile's stated `y`/`AD`/`AE`/`AN`/`AP`/`BE`/`BF`/
  `U9` surface for these items, with two recorded push deferrals
  (§9) instead of silent asymmetry.
- `packages/bunting-trading-state` renders all of this: its model
  today keeps `ListingInfo`, `AccountView`, fills and prints since
  connect (observed in `model.rs`); it gains reference data,
  venue valuation replacing derived figures, case state, the
  transaction log and history-seeded series, with the TUI and desk
  picking them up through the same model (ADR 0047, ADR 0050).

## Rejected alternatives

- **Extend `Listings` in place.** Breaks 1.0 decoders (trailing
  bytes close the session) and forces per-listing repetition of
  per-instrument facts into a message with no paging.
- **BNP v2 for additive messages.** A major version for additions
  strands conforming 1.0 clients for no semantic reason and spends
  the one clean break the protocol has. Reserved for body changes
  (§1).
- **Carry history from the engine's `MarketProjection`.** Bounded
  to 4,096 trades with silent eviction, 1-second bars only, no
  sequences, and mutable run state — exactly what ADR 0053 §2
  forbids as a publication source.
- **Serve history hub-side "because it is old data".** Would let a
  team near the hub read a distant venue's trades before that
  venue's direct feed could reach a team beside the venue —
  a latency-path shortcut ADR 0035 forbids.
- **Client-derived valuation permanently, labelled.** The desk
  would keep disagreeing with scoring at every FX conversion, fee
  and mark-source edge; the label documents the disagreement
  without removing it.
- **A posting journal inside `Ledger`.** Duplicates the journaled
  event record as a second economic history that must be kept
  consistent with it (ADR 0028 item 3: one authority). The
  projection derives everything from committed records instead.
- **A generic key/value or JSON extension message** (for case info,
  trader info, permissions). Untyped, against BNP's fixed-layout
  decision (ADR 0040) and the FIX profile's no-generic-escape rule.
- **Period/tick as engine state.** A second clock beside
  `simulation.clock.now`, reintroducing the trap ADR 0037 removed.
  The alias is computed at the read boundary only.

## Validation

Per slice, recorded in `docs/implementation-log/` with the commands
that ran (this ADR itself runs no code):

1. **Codec:** round-trip and malformed-frame tests for every new
   message in `packages/bnp-wire` (truncation, trailing bytes,
   oversized groups/strings, unknown kind/status/type codes),
   plus golden frames so a layout drift fails loudly.
2. **Negotiation:** a 1.0 client against a 1.1 server receives no
   1.1 type byte across a full session including pushes; a 1.1
   client against a 1.0 server is logged out and the reference
   client's single retry succeeds as 1.0; a 1.1 request on a
   1.0-negotiated session closes it.
3. **History correctness:** trades and every bar interval equal an
   independent recomputation from the journal; paging by
   `after_sequence`/`after_time_ns` tiles the series with no
   overlap or hole; retention overflow flips `complete` and
   advances `earliest_sequence`; after `kill -9` and restart the
   rebuilt series is identical.
4. **No look-ahead:** under a latency map with distinct team/venue
   locations, no `History` answer contains a trade that had not
   already been sent on the direct feed to a same-location
   subscriber when the answer left the venue (asserted on
   send times in the server integration harness).
5. **Valuation = scoring:** for randomized seeded runs,
   `Valuation.reporting_nlv` at a `DaySettled`/`ScoreIteration`
   `run_sequence` equals the frozen score for every participant,
   including FX-converted and at-cost positions; push coalescing
   never delivers a stamp out of order against fills.
6. **Transactions conservation:** §7's per-currency sum identity
   holds at every index for seeded runs mixing fills, fees,
   tenders, OTC, cashflows and fines; indices are stable across
   restart/replay; live entries and backfilled pages agree.
7. **Audience:** no participant's valuation, history of another's
   NLV, transaction or trader info ever appears on another's
   session (BNP and FIX); operators receive none of them.
8. **FIX parity:** §9's extended parity test, both interfaces read
   at the same `run_sequence`.
9. **Bounds:** page limits, bar-interval whitelist, rate-limit
   interaction and projection caps each proven by a refusal test,
   not by inspection.
10. **Repository gates** from `AGENTS.md` for every slice
    (fmt, clippy, workspace tests, wasm32 check — `bnp-wire` and
    the engine stay host-neutral), plus `tools/check_desk.sh` once
    the desk consumes the new model fields.

## Unresolved (open questions for the owner)

1. **Period semantics:** is the desk's "period" the trading day
   (this ADR) or the sub-heat/iteration (§6 alias can be re-pointed
   by the case-structure ADR, but a case author reading the desk
   during a 5 × 600 s case will see one or the other — owner
   chooses the label).
2. **Retention defaults:** the three projection caps need owner-
   accepted defaults from a recorded classroom-scale workload
   (trades per listing, transactions and NLV points per
   participant). R2/R5 must bring measurements; this ADR refuses
   to invent the numbers.
3. **Valuation push default:** on at 250 ms, on at 1 s, or off
   (request-only) as the shipped default for competitions — a
   fairness/feel choice, not a correctness one.
4. **Case brief delivery:** whether briefs travel over BNP at all
   (chunked, hashed, bounded) or stay out-of-band with only the
   case name on the wire (§6).
5. **Team identity authority:** roster `team_id` (presentation,
   §5) versus scenario-declared teams when team scoring lands —
   which one names a team on the wire must be settled there, not
   drifted into.
6. **RIT close-out fine type:** whether RIT books leftover fines
   as `CASE_FINE` or a distinct type is not proved by the swagger
   field names alone; Bunting's `CloseOutFine` split (§7) stands
   until a case source settles it.
7. **Currency table ownership:** scenario-declared (this ADR) vs a
   server-level registry shared by runs — matters once one venue
   hosts several scenarios with the same `CurrencyId` meaning
   different currencies.

## References

- [`docs/specs/bnp-v1.md`](../specs/bnp-v1.md) — current wire
  contract; framing, bounds, resume, latency path.
- [`docs/specs/rit-desk-window-parity.md`](../specs/rit-desk-window-parity.md)
  §"Protocol gaps" — the gap list this ADR answers.
- [`docs/specs/rit-case-primitives.md`](../specs/rit-case-primitives.md)
  §1, §2, §6, §8, §9 — engine primitives and the RIT evidence
  summaries (copyright Rotman FRTL, paraphrased there and here;
  nothing copied).
- RIT DMA REST API 1.0.5 swagger
  (`https://rit.306w.ca/RIT-DMA-API/1.0.5/swagger.json`, read
  2026-10-20) — resource and field-name model only.
- Code at `e027402`: `packages/bnp-wire/src/lib.rs` and
  `src/codec.rs`, `packages/bunting-engine/src/lib.rs` and
  `src/simulation.rs` and `src/calendar.rs`,
  `packages/ledger/src/lib.rs`, `packages/risk-engine/src/lib.rs`,
  `packages/bunting-application/src/bnp.rs` and
  `src/competition.rs`, `apps/bunting-server/src/bnp_host.rs` and
  `src/distributor.rs` and `src/config.rs`,
  `packages/bunting-trading-state/src/model.rs`.
- [ADR 0018](0018-unified-bunting-engine.md),
  [0028](0028-proposed-headless-run-authority.md),
  [0031](0031-proposed-bunting-native-client-protocol.md),
  [0035](0035-latency.md),
  [0036](0036-public-market-data-feeds.md),
  [0037](0037-run-clock-calendar-and-auctions.md),
  [0040](0040-bunting-native-protocol-v1.md),
  [0042](0042-bnp-operators-and-run-stream.md),
  [0043](0043-bnp-tenders-and-otc.md),
  [0050](0050-desktop-trading-app-on-gpui.md),
  [0051](0051-desk-workspace-canvas-of-floating-windows.md),
  [0053](0053-replayable-exchange-publication-and-runtime.md),
  [0055](0055-settlement-close-out-and-portfolio-limits.md).
