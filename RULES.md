# Competition rules

All teams trade on one shared market. The venue batches authenticated commands
into 100 ms discrete matching intervals, assigns a monotonic arrival sequence,
and commits each interval in that sequence. Every team receives the same public
depth and publication cadence.

The event profile publishes `max_connections`, `max_messages_per_interval`,
`max_interval_queue`, wire-byte, journal, and pending-message limits before a
round, and the scenario publishes each participant's risk limits, including
`max_live_orders` (orders resting on any listing at once; IOC, FOK and market
orders do not count). The engine enforces risk limits for the participant
across all of its connections. A rejected message names the limit it exceeded.

Execution reports for a team's orders, including fills on resting orders
caused by other teams or built-in agents, are delivered on that team's
connection as soon as they are committed.

Resting orders survive a FIX disconnect. Reauthentication restores FIX sequence
and application state; it does not cancel or reprioritize book state. Operators
may halt the whole round for safety, and the settled result always comes from a
successful archive replay rather than the live display.

Credentials bind one connection to one roster participant. Sharing credentials,
attempting another participant's CompID, flooding, malformed framing, or
accessing private reports for another identity is prohibited and rejected.
