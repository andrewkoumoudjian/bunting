# Unified Bunting engine instructions

This crate owns the authoritative sans-I/O run transition, the single economic ledger integration and the private deterministic order book (`src/book.rs`, ADR 0029).

- The book is the only matcher. Keep it clock-free, randomness-free, `Clone`, `O(log n)` per operation and canonically serializable.
- OrderBook-rs is a dev-dependency differential oracle only; extend the oracle test when changing shared semantics (GTC limit, market, cancel, priority).
- Any new order type needs explicit Bunting semantics, book tests and engine tests before entering the canonical command schema.
- Keep listing books private; public callers submit canonical Bunting commands and receive canonical outcomes.
- Keep every collection bounded and canonical serialization deterministic.
- Persistence, transports (FIX, REST, RIT), and participant execution engines remain outside this crate.
- Add tests for multi-listing isolation, staged rollback, sequence advancement, snapshot restore/replay, state hashes and ledger conservation.
