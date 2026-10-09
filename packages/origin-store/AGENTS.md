# Origin-store instructions

Keep persistence contracts host-independent and exact. A commit binds idempotency, canonical events, the resulting run version and the expected-version check into one atomic operation; a failed or ambiguous commit must never be acknowledged.

`LiveRun` is the writer-owned run: it applies `JournalInput`s in place, keeps the idempotency index and event-hash chain, and rolls back an engine-poisoned transition from its last checkpoint plus the committed tail. Hosts persist each `CommandRecord` before acknowledging and use `RunRecovery` to rebuild from genesis or a checkpoint; recovery must re-execute and compare, never trust state it cannot re-derive. Admission metadata (ADR 0030) belongs in the record when the sequencer lands. Keep IDs and sequences exact across every serialization; never coerce wide integers to floating point.
