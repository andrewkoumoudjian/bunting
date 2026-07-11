# market-run-do instructions

One object is the authoritative sequencer for one run. Persist accepted events before acknowledgement, use SQLite for recovery, bound WebSocket fan-out, and make alarm work idempotent.
