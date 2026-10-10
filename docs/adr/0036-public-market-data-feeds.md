# ADR 0036: Public market data — per-venue direct feeds over the latency map

- Status: **Accepted** (2026-10-10). Per-venue trade and L2 feeds were
  implemented in slice 21 (see `docs/implementation-log/`). The four owner
  choices below are **provisional defaults**, recorded under the owner's
  direction to keep building and revisit them later. The consolidated feed
  and L3 feeds are **Target**.
- Date: 2026-10-10
- Depends on: ADR 0011 (committed-sequence streams), ADR 0022 (single
  venue), ADR 0029 (owned book), ADR 0035 (latency), slice 12 (committed-event
  distributor).
- Supersedes: nothing.

## Context

The owner asked (2026-10-10) for participants to see all trades and order
activity on every venue so that cross-venue arbitrage is possible, but
realistically: not as an instant global list of orders. Under ADR 0035 each
venue is somewhere on the latency map, so what a team sees, and when, must
depend on which feeds it takes and how far it is from each venue. The
options and the open questions are in
[`research/2026-10-10-cross-venue-market-data.md`](../research/2026-10-10-cross-venue-market-data.md).
Until slice 21, public data was request/response L2 snapshots only.

## Decision

1. **Direct venue feeds.** A FIX `MarketDataRequest` (`V`) with 263=1 for
   one listing (tags 48 + 207) is a subscription to that venue's direct
   feed: a full-depth snapshot (`W`) taken at the venue when the request
   arrives there (after `L(p, v)`), then one
   `MarketDataIncrementalRefresh` (`X`) per later commit that changed that
   listing's public view. 263=2 with the same 262 ends it.
2. **Derived from commits only.** Right after each commit, on the sequencer
   thread (the only committer), the venue computes the public view of every
   listing the commit touched: its trades in execution order, then each
   price level whose visible quantity changed, with the resulting
   quantity (zero = level deleted). A feed is therefore a function of the
   journal, never ahead of commit, and the same for every subscriber.
3. **Timing.** Every feed message leaves the venue when the commit is
   durable and reaches each subscriber `L(v, p)` later, `v` being the
   listing's venue, then crosses the subscriber's real connection. One
   feed's messages never overtake each other, even with path jitter.
4. **Exactly-once continuity.** The subscription is registered before its
   request is admitted; the snapshot's reply records how many committed
   batches preceded it, and the feed continues from the next one. Batches
   committed while the snapshot is in flight are buffered (bounded). Each
   entry carries a per-feed report sequence (tag 83) that increases by one,
   so a client can prove it missed nothing.
5. **Bounds.** At most 32 feeds per session; a feed's in-flight buffer and
   the session's outbound hold are bounded, and overflowing either
   disconnects the session rather than publishing a feed with a hole.

### Provisional owner defaults (revisit any time)

| Question | Default | Why this default |
|---|---|---|
| L2 only, or L3 order-by-order? | **L2 by price level, plus trades.** L3 later as an opt-in feed. | The most conservative: reveals no order-level queue information; L3 can be added without changing L2. |
| A consolidated (SIP-like) feed, and where? | **Yes, later**, from a processor at the hub's location on the map, with a processing delay. Not built yet. | It is the realistic latency-arbitrage lever, but it needs no decision to start with direct feeds. |
| Broker identifiers? | **None.** Feeds are fully anonymous. | Leaks the least; a per-venue broker-ID option can come later. |
| Data and colocation pricing? | **Free; no colocation purchase.** Locations come from the organizer's map. | Keeps scoring unchanged until the owner chooses a pricing rule. |

## Consequences

- Teams can see every venue's trades and depth, each at the age its
  distance implies, so stale-quote risk and cross-venue arbitrage are
  observable and realistic.
- Each commit now reads the visible depth of the listings it touched
  (O(levels) of those books); simulation events re-read every listing of
  the run.
- `RULES.md`, `PROTOCOL.md` and the FIX profile describe the feed.

## Rejected alternatives

- **Diffing the book at delivery time** (each session reading live state
  when it drains a batch): the state may already include later commits,
  so a far team would learn of them early.
- **Deriving depth from events per subscriber**: cancel and reduce events
  carry no price, so every subscriber would need order-level state (L3)
  to maintain L2.
- **One global stream of every venue's orders**: the owner explicitly
  rejected an instant global view.

## Validation

`bunting-application` (`diff_levels`), `simfix-mapping` (263=0/1/2, the
multi-entry `X` layout and its absence of identity tags),
`apps/bunting-server/src/public_feed.rs` (a feed resumes exactly after its
snapshot, venue and side filtering, limits, a full buffer is an error,
send times never go backwards), and end-to-end
`apps/bunting-server/tests/public_feeds.rs` (two venues: venue 1's feed
reaches the team 40 ms away no sooner than its path allows and after the
near team; trades and level changes carry no identity; snapshot plus
increments equal a fresh snapshot after racing activity; unsubscribe stops
the feed).

## Operational impact

No new configuration. Organizers place venues and teams on the existing
latency map; the feed follows it.

## Security impact

The public projection is an allowlist (price, quantity, side, listing):
no participant, order, command or account identity, and hidden quantity
never appears. A slow subscriber cannot stall the committer or other
sessions; it is disconnected at its bounds.

## References

- [ADR 0035](0035-latency.md), [ADR 0011](0011-streaming-market-data.md)
- [Cross-venue market data exploration](../research/2026-10-10-cross-venue-market-data.md)
- `docs/specs/bunting-fix-competition-profile.md` (public market data)
