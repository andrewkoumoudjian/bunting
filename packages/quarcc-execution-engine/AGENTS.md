# QUARCC execution-engine instructions

Preserve the public `quarcc.v1` service names, field meanings, and enum discriminants in the `compatibility` module without importing unlicensed implementation text.

- Keep the default crate WASM-safe, deterministic, bounded, and transport-neutral.
- Legacy floating-point fields stay at the compatibility boundary and never enter execution state without checked fixed-point conversion.
- The engine owns participant-local intent, lifecycle, ID mapping, risk, positions, reconciliation, and snapshots; it never owns venue truth.
- Do not add sockets, Worker APIs, Tokio, filesystem access, SQLite, wall-clock reads, or mutable market-engine references.
- Duplicate and out-of-order venue reports must be explicit and idempotent; ambiguous state is quarantined instead of guessed.
