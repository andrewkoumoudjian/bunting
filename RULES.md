# Competition rules

All teams trade on one shared market. The venue batches authenticated commands
into 100 ms discrete matching intervals, assigns a monotonic arrival sequence,
and commits each interval in that sequence. Every team receives the same public
depth and publication cadence.

The event profile publishes `max_connections`, `max_messages_per_interval`,
`max_open_orders`, `max_interval_queue`, wire-byte, journal, and pending-message
limits before a round. A rejected message names the limit it exceeded.

Resting orders survive a FIX disconnect. Reauthentication restores FIX sequence
and application state; it does not cancel or reprioritize book state. Operators
may halt the whole round for safety, and the settled result always comes from a
successful archive replay rather than the live display.

Credentials bind one connection to one roster participant. Sharing credentials,
attempting another participant's CompID, flooding, malformed framing, or
accessing private reports for another identity is prohibited and rejected.
