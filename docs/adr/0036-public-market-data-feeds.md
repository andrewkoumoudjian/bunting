# ADR 0036: Public market data — direct venue feeds and a consolidated tape over the latency map

- Status: **Accepted** (2026-10-10). Per-venue trade and L2 feeds were
  implemented in slice 21 and the consolidated tape in slice 22 (see
  `docs/implementation-log/`). The owner answered the four open questions
  on 2026-10-10 (below). Order-by-order (L3) feeds, broker identifiers and
  data/colocation pricing are **Target**.
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
6. **Consolidated tape (slice 22).** Like a securities information
   processor, one processor sits at the hub's location on the map. Right
   after each commit, each touched listing's trades and new best bid and
   offer start towards it and reach it `L(v, hub)` plus
   `fix.admission.consolidated_processing_us` later (default 500 µs). The
   processor applies changes in hub-arrival order and publishes, per
   instrument, every venue's trades and every change of a venue's best bid
   or offer (279=0 new, 1 replaced, 2 gone), each entry naming its venue
   (207) and numbered by one per-instrument report sequence (83) that is
   the same for every subscriber. A `V` naming exchange 0 (207=0) is a
   request to the processor: it travels `L(p, hub)`, its snapshot (`W`)
   lists each venue's best bid and offer as the processor sees them then,
   with 83 = the last report included, and a subscription continues with
   the next report; records reach the subscriber `L(hub, p)` after the
   processor applied them. A consolidated feed always carries quotes and
   trades, so its report sequence has no gaps. If more than 65,536 changes
   are in flight to the processor, the tape goes down (every subscriber is
   disconnected and new requests are refused) rather than publish a hole;
   direct feeds are unaffected.

### Owner decisions (2026-10-10)

The owner answered the open questions, replacing slice 21's provisional
defaults:

| Question | Owner decision | Status |
|---|---|---|
| L2 only, or L3 order-by-order? | **Both**: price-level and order-by-order direct feeds. | L2 implemented (slice 21); L3 Target, with anonymous per-venue order references, never the owner's IDs. |
| A consolidated (SIP-like) feed, and where? | **Yes**, or per-venue feeds only, whichever is closer to reality. Real markets run both, so Bunting has both: direct feeds per venue and one consolidated tape from a processor at the hub. | Implemented (slice 22). |
| Data and colocation pricing? | **The most realistic.** Real firms pay for direct feeds and colocation; consolidated data is cheaper. | Target: the fee rule (charged through the single ledger, published with the event profile) will be set in its own ADR before it is built. Until then data and colocation are free and locations come from the organizer's map. |
| Broker identifiers? | **Yes.** | Target: shown on order-by-order feeds and trades unless an order is marked anonymous (as on Toronto venues); not yet published. |

## Consequences

- Teams can see every venue's trades and depth, each at the age its
  distance implies, so stale-quote risk and cross-venue arbitrage are
  observable and realistic.
- Each commit now reads the visible depth of the listings it touched
  (O(levels) of those books); simulation events re-read every listing of
  the run.
- Teams relying only on the consolidated tape see every venue's quotes
  later than a team with direct feeds near those venues, which is the
  classic latency-arbitrage exposure.
- Each commit's public changes are also queued for the processor; its
  state is bounded by the in-flight limit and one quote pair per listing.
- `RULES.md`, `PROTOCOL.md` and the FIX profile describe the feeds.

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
send times never go backwards), `apps/bunting-server/src/consolidated.rs`
(hub-arrival order and one report sequence, deletes and unchanged quotes,
primed quotes, the in-flight bound takes the tape down), and end-to-end
`apps/bunting-server/tests/public_feeds.rs` (two venues: venue 1's feed
reaches the team 40 ms away no sooner than its path allows and after the
near team; trades and level changes carry no identity; snapshot plus
increments equal a fresh snapshot after racing activity; unsubscribe stops
the feed; the tape reaches a team beside venue 2 only after venue 2 ->
hub -> team and later than venue 2's direct feed, names each entry's
venue, has no report gaps, and snapshot plus records equal a fresh tape
snapshot).

## Operational impact

Organizers place venues, teams and the hub on the existing latency map;
the feeds follow it. `fix.admission.consolidated_processing_us` (default
500) sets the processor's delay and is published with the map.

## Security impact

The public projection is an allowlist (price, quantity, side, listing):
no participant, order, command or account identity, and hidden quantity
never appears. A slow subscriber cannot stall the committer or other
sessions; it is disconnected at its bounds. The consolidated processor
never blocks the committer: changes are queued under a bound, and
exceeding it takes only the tape down.

## References

- [ADR 0035](0035-latency.md), [ADR 0011](0011-streaming-market-data.md)
- [Cross-venue market data exploration](../research/2026-10-10-cross-venue-market-data.md)
- `docs/specs/bunting-fix-competition-profile.md` (public market data)
