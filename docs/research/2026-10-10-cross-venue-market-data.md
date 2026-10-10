# Cross-venue market data: seeing every trade and order, realistically

Status: **Exploration** — owner request 2026-10-10. Options A and B (per-venue
trade and L2 feeds) were implemented in slice 21 and option D (consolidated
tape) in slice 22 and option C (order by order) in slice 23 under [ADR 0036](../adr/0036-public-market-data-feeds.md),
which also records the owner's answers to the questions below (both L2 and
L3; consolidated and direct feeds; the most realistic broker-identifier
rule, which is per venue; data and colocation free); `RULES.md` states what
participants can see today.

## The request

Participants should be able to see all trades and order activity on every
venue so that venue arbitrage is possible — but realistically, not as an
instant global list of orders. Combined with ADR 0035 (real connectivity
counts; each venue is a virtual distance from each team), what a team sees
must depend on which feeds it uses and how far it is from each venue.

## What exists now (observed in this repository)

- Every committed command produces canonical events; order and trade events
  carry their `listing_key` (venue + instrument): `OrderReceived`,
  `OrderRested`, `OrderCanceled`, `TradeExecuted`
  (`packages/market-events`).
- `bunting-application::project_public_event` already projects an
  allowlisted, anonymous `PublicTrade` (venue, instrument, price, quantity,
  time) with no participant, order or command identity.
- The committed-event distributor fans every committed batch out to every
  session (slice 12); sessions currently map only their own participant's
  reports.
- Outbound delivery already applies the venue-to-team virtual path `L(v, p)`
  (slice 16) from the latency map, which also defines team-to-team paths
  (slice 17), so any new feed or team-to-team channel inherits realistic
  timing for free.
- Public data today: request/response L2 snapshots per venue (FIX `V`, tag
  207). No streaming public trades or book updates.

## How real markets publish (background, not repository evidence)

- **Direct venue feeds.** Each exchange publishes its own feed from its own
  data centre: order-by-order (often called L3: add, modify, delete, execute
  with anonymous order references, e.g. Nasdaq TotalView-ITCH, Cboe PITCH)
  and/or aggregated depth (L2) plus trades. Subscribers far from the venue
  receive it later.
- **Consolidated feeds.** A securities information processor aggregates every
  venue's best bid/offer and trades into one feed (the US SIP's NBBO and
  consolidated tape; Canada has an information processor too). It adds an
  aggregation hop at the processor's location, so it is slower than direct
  feeds near the venues — the classic source of latency arbitrage against
  traders who only watch the consolidated view.
- **Anonymity.** Feeds identify orders, not owners. Some markets publish
  broker identifiers (Toronto has historically shown broker numbers unless
  the order is anonymous), which lets traders infer who is active.
- **Hidden liquidity.** Iceberg reserves and hidden orders are not shown;
  only their executions are.
- **Cost.** Direct feeds and colocation cost money; consolidated data is
  cheaper. Firms choose.

## Options for Bunting (Bunting-added proposals)

| Option | What a team sees | Delivery timing (ADR 0035) | Realism | Cost to build |
|---|---|---|---|---|
| A. Per-venue trades | Anonymous trades of one venue | `L(v, p)` + real delay | High | Low: projection already exists |
| B. Per-venue L2 incremental | Book changes by price level + trades | `L(v, p)` + real delay | High | Medium: incremental depth, sequence numbers, gap recovery |
| C. Per-venue L3 order-by-order | Every add/cancel/execute with anonymous per-venue order references (never the owner's IDs) | `L(v, p)` + real delay | Highest; enables queue-position inference | Medium-high: anonymous ID mapping, hidden-quantity rules |
| D. Consolidated feed | Best bid/offer per venue and consolidated trades | `L(v, processor)` + processing + `L(processor, p)` | High; the latency-arbitrage lever | Medium: a processor location in the latency table |
| E. Broker identifiers | Per-venue option to show a team code on orders/trades | as the feed | Realistic for some venues | Low once C exists |
| F. Data and colocation pricing | Feeds and lower virtual latency cost score/cash | — | Realistic firm trade-offs | Medium: scoring rules |

**Recommendation.** Build A and B first as FIX `MarketDataIncrementalRefresh`
(35=X) subscriptions per venue (and BNP streams under ADR 0031), delivered
over `L(v, p)`; then D, so teams who rely only on the consolidated view are
exposed to faster traders using direct feeds; then C for queue-position
strategies. E and F are scenario options to decide with the owner.

Design constraints whichever option is chosen:

- Derived only from committed events, so every feed is reproducible from the
  journal and never ahead of commit.
- Bounded per-subscriber queues with explicit gaps and snapshot recovery
  (sequence numbers per venue feed); a slow consumer must not stall others.
- Order references in public feeds are per-venue, anonymous and unrelated
  to participants' own IDs, so no feed leaks ownership.
- Hidden and iceberg reserves stay hidden; only executions reveal them.
- The consolidated processor's location and processing delay are part of the
  published latency table.

## Questions for the owner

1. L2 only, or L3 order-by-order on direct feeds?
2. Should a consolidated (SIP-like) feed exist, and where is its processor?
3. Should data feeds and lower virtual latency (colocation) cost score or
   cash?
4. Should any venue publish broker identifiers?

## To do (tracked in the exploration note §8)

- [x] Per-venue public trade stream (option A) over the virtual path
      (slice 21).
- [x] Per-venue L2 incremental feed with sequence numbers and snapshot
      recovery (option B) (slice 21; recovery is a new subscription).
- [x] Consolidated feed from a processor at the hub (option D) (slice 22).
- [x] Owner decisions (2026-10-10, ADR 0036): L2 and L3; consolidated and
      direct feeds; broker identifiers per venue (the realistic choice);
      data and colocation free.
- [x] L3 order-by-order feed with anonymous public references (option C)
      (slice 23).
- [ ] Per-venue broker identifiers on L3 feeds and trades, unless an
      order is anonymous (option E).
- [ ] Location coordinates helper: derive the latency map's links from
      location coordinates (great-circle distance, fibre or microwave route
      factor, venue gateway delay) instead of hand-written microseconds.
- [ ] Team-to-team messages over FIX/BNP (OTC negotiation, a participant
      messaging or data-sharing channel), admitted with the counterparty as
      destination so they travel `L(a, b)`; owner to decide which exist.
