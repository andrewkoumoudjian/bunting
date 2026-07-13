# Unified Bunting engine instructions

This crate owns the authoritative sans-I/O run transition and its private adapter around `orderbook-rs = 0.10.3`.

- Do not add Bunting-owned price levels, FIFO queues, matching loops, snapshot formats, risk hooks, or depth analytics already supplied upstream.
- Prefer upstream per-call result APIs.
- Preserve exact version and audited commit metadata.
- Keep listing books private; public callers submit canonical Bunting commands and receive canonical outcomes.
- Keep every collection bounded and canonical serialization deterministic.
- Persistence, Worker bindings, tRPC, FIX, REST, and participant execution engines remain outside this crate.
- Add tests for adapter conversion, multi-listing isolation, staged rollback, sequence advancement, snapshot restore/replay, and state hashes.
- A fork requires ADR 0013's Wasm-blocker process and MIT attribution.
