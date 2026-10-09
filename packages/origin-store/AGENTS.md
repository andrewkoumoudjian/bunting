# Origin-store instructions

Keep persistence contracts host-independent and exact. A commit binds idempotency, canonical events, the resulting run version and the expected-version check into one atomic operation; a failed or ambiguous commit must never be acknowledged.

Current `CommitRequest` carries the full candidate `RunState`. The target is a command-sourced journal (input, admission metadata, resulting events, event-hash chain) with full state only in checkpoints. Keep IDs and sequences exact across every serialization; never coerce wide integers to floating point.
