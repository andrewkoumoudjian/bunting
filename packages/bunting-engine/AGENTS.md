# Unified Bunting engine instructions

This crate owns the authoritative sans-I/O run transition, the single economic ledger integration and the private deterministic order book (`src/book.rs`, ADR 0029).

- The book is the only matcher. Keep it clock-free, randomness-free, `Clone`, `O(log n)` per operation and canonically serializable.
- OrderBook-rs is a dev-dependency differential oracle only; extend the oracle test when changing shared semantics (GTC limit, market, cancel, priority).
- Any new order type needs explicit Bunting semantics, book tests and engine tests before entering the canonical command schema.
- Keep listing books private; public callers submit canonical Bunting commands and receive canonical outcomes.
- Keep every collection bounded and canonical serialization deterministic.
- Persistence, transports (FIX, native protocol, RIT), admission timing and participant execution engines remain outside this crate.
- NBC compatibility was removed in slice 11 (ADR 0032). Do not reintroduce an NBC mode, NBC commands/events or a step barrier named after NBC; a lockstep feature must be specified as Bunting-native behavior.
- Transitions must stay cheap relative to state size: matching costs microseconds, while full-state clones and serialization cost milliseconds (2026-10-09 measurement). Prefer in-place transitions with a no-mutation-on-error contract; do not add whole-state copies or hashes to the per-command path.
- Per-participant limits (including live-order counts) are engine risk admission, not adapter state.
- Add tests for multi-listing isolation, staged rollback, sequence advancement, snapshot restore/replay, state hashes and ledger conservation.
