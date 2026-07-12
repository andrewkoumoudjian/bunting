# Order-book adapter instructions

This crate is a thin adapter around `orderbook-rs = 0.10.3`.

- Do not add Bunting-owned price levels, FIFO queues, matching loops, snapshot formats, risk hooks, or depth analytics already supplied upstream.
- Prefer upstream per-call result APIs.
- Preserve exact version and audited commit metadata.
- Add adapter tests for ID/unit conversion, snapshot restore, typed errors, kill switch, and Worker recovery.
- A fork requires ADR 0013's Wasm-blocker process and MIT attribution.
