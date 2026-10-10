# Competition rules

All teams trade on one shared market with continuous price-time matching.

**Distance does not decide; your client does.** The venue measures each
connection's one-way network delay and removes it in both directions (ADR
0034, `equalized` mode):

- An order reaches the book at about *the time your client sent it* plus a
  published constant `D`, whether you are next to the server or across an
  ocean, on fibre or on a slow ISP.
- Every report and market-data response is held so that it reaches every team
  at about *commit time + `D`*. Nobody sees a fill or a book change earlier
  because they are closer.
- What remains is your own processing time: how fast your client reads,
  decides and sends. Pipelining several orders, keeping one warm connection,
  disabling Nagle (`TCP_NODELAY`) and parsing efficiently all keep their full
  advantage.
- Looking slower does not help. The delay estimate is the minimum of the
  kernel's TCP round trip (from your operating system's ACKs) and FIX
  TestRequest round trips, kept over the connection's lifetime, so answering
  heartbeats slowly earns nothing. Behind a proxy the venue may measure with
  TestRequests only (`rtt_sources: probe_only`, published); there a reply
  delay would be counted as distance, so answer TestRequests as soon as they
  arrive. Deliberately delaying them is prohibited (below).
- The organizer publishes `D` before the round. A team whose one-way delay
  exceeds `D` is compensated only up to `D`; the organizer's health page shows
  each connection's measured delay so this can be checked before trading.
  Network jitter above your path's minimum delay and asymmetric routes are not
  removed; a wired connection minimizes them.

The event profile publishes the admission mode, `D`, `rtt_sources`,
`max_connections`, `max_messages_per_interval` per `rate_limit_window_ms`,
admission-queue, outbound-hold, wire-byte, journal, and pending-message limits
before a round, and the scenario publishes each participant's risk limits, including
`max_live_orders` (orders resting on any listing at once; IOC, FOK and market
orders do not count). The engine enforces risk limits for the participant
across all of its connections. A rejected message names the limit it exceeded.

Execution reports for a team's orders, including fills on resting orders
caused by other teams or built-in agents, are delivered on that team's
connection at the equalized time above: commit time plus `D`, the same for
every team.

Resting orders survive a FIX disconnect. Reauthentication restores FIX sequence
and application state; it does not cancel or reprioritize book state. Operators
may halt the whole round for safety, and the settled result always comes from a
successful archive replay rather than the live display.

Credentials bind one connection to one roster participant. Sharing credentials,
attempting another participant's CompID, flooding, malformed framing, or
accessing private reports for another identity is prohibited and rejected.
Deliberately delaying TestRequest replies or TCP acknowledgements to inflate
the measured network delay is prohibited; the organizer compares each team's
measured delay with its registered location.
