# Test instructions

Prioritize: ledger conservation and no-mutation-on-reject invariants;
deterministic replay from genesis and checkpoints; snapshot/hash equivalence;
book behavior against the OrderBook-rs oracle; crash/restart recovery of the
origin journal; multi-session FIX behavior (maker reports, reconnect, limits);
admission ordering under ADR 0030; malformed protocol fuzzing; bounded queues
and slow consumers; FIX interoperability (`tests/interop/quickfixgo`).

Goldens (`tests/goldens/`) are regenerated only through verified replay
(`BUNTING_BLESS=1`), never hand-edited. Avoid wall-clock-dependent tests; drive
time through injected clocks.
