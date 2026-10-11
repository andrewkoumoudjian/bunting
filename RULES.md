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
- **Think of one stock on several exchanges**, the way one stock trades on
  the TSX, Cboe (formerly BATS) and NYSE at once. Each venue has its own
  book and sits at its own distance from you. An order sent to a far venue
  can be overrun on the way: a team nearer the better prices takes them
  first, and your order executes at whatever its own venue still offers,
  even if that is worse than another venue's price.
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
  262 ends a feed. The **consolidated tape** (`V` with 207=0) is one
  feed per instrument from a processor at the hub: every venue's trades
  and every change of each venue's best bid or offer, each entry naming
  its venue (207). A venue's change reaches the processor after that
  venue's path to the hub plus a processing delay
  (`consolidated_processing_us`, published), and then you after the hub's
  path to you, so near a venue the direct feed is faster and the tape can
  be stale. Its report sequence (83) is one per instrument, the same for
  everyone; its snapshot's 83 is the last report included. Feeds and
  colocation are free, with no colocation purchase. Add 266=N to a direct
  feed request for **order by order** instead of price levels: every
  displayed order by an anonymous public reference (278) in queue order,
  then each order added, changed (displayed quantity; it keeps its place)
  or deleted, and each trade naming the order it hit. A reference is not
  any order ID you or the venue use; an iceberg's refreshed slice gets a
  new one. Some venues publish **broker numbers** (your participant ID),
  as Toronto venues do: on those, each order you show and each side of
  your trades names you (288 for a buyer, 289 for a seller) unless you
  send the order with `BuntingAnonymous` (10021) = Y. The scenario says
  which venues publish them; the consolidated tape never does
  (ADR 0036).

## Order types and time in force

FIX and BNP take the same orders:

- **Limit** at a price, and **market**, which takes what is on the other
  side now and cancels the rest (it is never posted).
- **Time in force** on a limit: DAY (expires at the venue's close), GTC
  (until canceled), IOC (fills what it can at once; the rest expires),
  FOK (fills completely at once or expires), GTD (expires at a run time
  you give). On FIX it is TimeInForce (59): no 59 means DAY, as in
  standard FIX, so send 59=1 for an order that should rest overnight;
  0 DAY, 1 GTC, 3 IOC, 4 FOK, 6 GTD with `BuntingExpireTimeNs` (10022)
  in run-time nanoseconds. A venue without a trading calendar never
  closes, so there a DAY order rests until canceled. A market order on
  FIX takes no 59, 59=0 or 59=3.
- **Post-only** (FIX ExecInst 18=6): rejected (`PostOnlyWouldCross`)
  instead of trading if it would take liquidity on arrival.
- **Iceberg** (FIX DisplayQty 1138): shows that much at a time; each
  refreshed slice joins the back of the queue.

IOC and FOK cannot be post-only or icebergs. An order a time in force
ends (a DAY order at the close, a GTD order at its time, the rest of an
IOC or FOK) is reported as expired (FIX ExecType C), not canceled.

## Trading sessions and auctions (ADR 0037)

The venue keeps one run clock: every order, cancel and request is stamped
with run time when the venue applies it, and anything due (an expiry, a
scheduled event, a session change) is applied at its time even if nobody
sends anything. The admin and organizer views show run time.

A scenario may give each venue a trading calendar. Without one, the venue
trades continuously, as before. With one, each trading day runs:

- **Closed** until the pre-open: new orders are rejected (`MarketClosed`);
  cancels are accepted. Good-till-cancel orders stay in the book overnight.
- **Pre-open**: limit orders collect in the book without trading, even if
  they cross. Market, IOC and FOK orders are rejected
  (`CallPhaseOrderType`); send a limit at the worst price you accept.
- **Opening auction** at the open: the venue trades the crossed book at one
  price, the one that trades the most, then leaves the least unmatched,
  then is nearest the last trade (or the opening mark), then is lowest.
  Orders fill in price-time priority, hidden iceberg quantity included.
  Both sides of an auction trade pay the taker fee.
- **Continuous** trading, as on a venue without a calendar.
- **Closing call**: orders collect again without trading.
- **Closing auction** at the close, by the same rule; then DAY orders
  expire and the venue is closed until the next trading day's pre-open.

At each close the venue records every listing's official closing price:
the closing auction price, else the day's last trade there, else the
previous close. A run may last several trading days. Positions, cash and
good-till-cancel orders carry from one day to the next; nights, weekends
and holidays are skipped, so the next pre-open follows the close at once.
During the day your net liquidation value uses each instrument's last
trade on any venue. At the end of each day, once every venue has closed,
each instrument is marked at the official close of its primary listing
(the scenario names it), and every team's value at those marks is recorded
as that day's score.

Your order reaches the venue over your path as always, so an order sent
just before the open or the close can arrive too late for the auction.

## Published limits

The event profile publishes the latency map, `max_connections`,
`max_messages_per_interval` per `rate_limit_window_ms`, admission-queue,
outbound-hold, wire-byte, journal, and pending-message limits before a
round, and the scenario publishes each participant's risk limits, including
`max_live_orders` (orders resting on any listing at once; IOC, FOK and market
orders do not count). The engine enforces risk limits for the participant
across all of its connections. A rejected message names the limit it exceeded.

Read what the venue sends you. A FIX connection whose socket stops taking a
message for `write_timeout_ms` (5 seconds by default) is closed, so a client
that hangs or vanishes frees its session and can log on again at once.
Log on again with your FIX sequence numbers and request a resend of what you
missed, as with any FIX disconnect. Reports the venue had not yet sent you
when the connection closed (still on their path), and reports for your
orders committed while you were away, are sent on your next logon, also
after a venue restart.

Execution reports for a team's orders, including fills on resting orders
caused by other teams or built-in agents, are delivered on that team's
connection after the venue commits them, over your virtual path from that
venue and then your real connection.

Resting orders survive a FIX disconnect. Reauthentication restores FIX sequence
and application state; it does not cancel or reprioritize book state. Operators
may halt the whole round for safety, and the settled result always comes from a
successful archive replay rather than the live display.

You may connect over FIX or over the Bunting Native Protocol (BNP) with the
certificate the organizers issue you ([wire contract](docs/specs/bnp-v1.md),
reference client `bunting-trader`). Both reach the same venues through the
same latency map; neither is faster through the venue, and your own client's
speed counts in both. One BNP session per team is live at a time,
independently of FIX. A BNP session identifies you only by your certificate;
keep its key private.

Credentials bind one connection to one roster participant. Sharing credentials,
attempting another participant's CompID, flooding, malformed framing, or
accessing private reports for another identity is prohibited and rejected.
One FIX session per team is live at a time; a reconnect replaces the closed
session.
