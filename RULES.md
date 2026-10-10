# Competition rules

All teams trade on one shared market. Each instrument may be listed on
several venues; every venue runs its own continuous price-time book.

## Latency (ADR 0035)

Latency works as on a real network; there is no equalization and no alternative setting:

- **Your real connection counts.** When your order reaches the server
  depends on where you are, your network and connection method (wired or
  Wi-Fi, VPN or proxy hops), your TCP stack and your client's own speed,
  exactly as for a trading desk.
- **Every venue, team and the organizer is somewhere.** The organizer
  publishes a latency map: the virtual location of each team, each venue and
  the organizer's hub, and the distance between locations (for example a
  team in New York: New Jersey venues a fraction of a millisecond away,
  Toronto several milliseconds). Your order reaches a venue's book that much
  after it reaches the server; that venue's reports and market data leave
  the venue that much before they are sent to you.
- **Distance between teams counts too.** Anything passed from one team to
  another through the venue travels the distance between their locations:
  teams in the same city exchange information faster than teams far apart.
  (No team-to-team message type exists yet; this rule governs any that is
  added.)
- **Orders to one venue stay in order; orders to different venues do not.**
  An order sent to a near venue can arrive before one you sent earlier to a
  far venue.
- The venue measures your connection (TCP handshake/ACK timing and FIX
  TestRequest round trips) and shows it to the organizer as your access
  latency. It never changes ordering, so there is nothing to gain by
  looking slower.

What a good implementation earns: fewer network hops, a wired connection,
`TCP_NODELAY` (a client that leaves Nagle on can wait about 40 ms for the
server's delayed ACK before a small order leaves), reading the socket
continuously, pipelining orders, fast parsing and decisions, and sending
each order to the right venue.

## Choosing venues, and the risks that come with it

- **You choose the venue for every order** (FIX tag 207). There is no
  smart order router and no trade-through protection: each venue matches
  only its own book, and a buy can execute on one venue above an offer
  resting on another. Best execution is your job.
- **Information is late from far venues.** Your view of a distant venue is
  older than your view of a near one, and other teams may be closer to it
  than you are.
- **Resting orders can be picked off.** If the price moves on another venue,
  a team that sees that move first can trade against your stale quote here
  before your cancel arrives, because your cancel travels the same virtual
  path as your order. Quote sizes, prices and venues with that in mind.
- **What you can see:** your own execution reports on every venue;
  snapshots of any venue's book (FIX `V`, 263=0, tag 207; 264=0 is the
  full book); and each venue's **direct feed** (`V`, 263=1): a full-depth
  snapshot, then a `MarketDataIncrementalRefresh` (`X`) for every committed
  change to that listing, with anonymous trades (269=2) and price-level
  changes (269=0/1; 279=0 new, 1 change, 2 delete; tag 271 is the
  level's resulting quantity, zero when deleted). Every feed message
  leaves the venue when the change commits and reaches you over your
  virtual path from that venue, so a near team sees a venue's activity
  before a far one. Feeds never identify who traded or whose order a level
  holds, and hidden quantity stays hidden. Each entry carries a report
  sequence (tag 83) that increases by one per entry; 263=2 with the same
  262 ends a feed. Feeds and colocation are free, and there is no
  consolidated feed yet (ADR 0036).

## Published limits

The event profile publishes the latency map, `max_connections`,
`max_messages_per_interval` per `rate_limit_window_ms`, admission-queue,
outbound-hold, wire-byte, journal, and pending-message limits before a
round, and the scenario publishes each participant's risk limits, including
`max_live_orders` (orders resting on any listing at once; IOC, FOK and market
orders do not count). The engine enforces risk limits for the participant
across all of its connections. A rejected message names the limit it exceeded.

Execution reports for a team's orders, including fills on resting orders
caused by other teams or built-in agents, are delivered on that team's
connection after the venue commits them, over your virtual path from that
venue and then your real connection.

Resting orders survive a FIX disconnect. Reauthentication restores FIX sequence
and application state; it does not cancel or reprioritize book state. Operators
may halt the whole round for safety, and the settled result always comes from a
successful archive replay rather than the live display.

Credentials bind one connection to one roster participant. Sharing credentials,
attempting another participant's CompID, flooding, malformed framing, or
accessing private reports for another identity is prohibited and rejected.
One FIX session per team is live at a time; a reconnect replaces the closed
session.
