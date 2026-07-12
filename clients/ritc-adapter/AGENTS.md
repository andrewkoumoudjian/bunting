# RITC adapter instructions

This is a native-only client boundary. Tokio, native TLS and HTTP/WebSocket libraries may be used here but never leak into core crates.

Keep venue request/response models, authentication, rate limiting, retries, reconnect and fixture transport here. Do not implement strategy formulas, authoritative positions, matching, ledger or risk in this client.

Translate venue prices and quantities through validated instrument metadata into `PriceTicks` and `QuantityLots`. Credentials come from injected configuration and must never appear in source, logs or fixtures.

Reconciliation uses `crates/order-reconciliation`; quote generation uses `crates/market-making`; all orders still pass the normal Bunting risk engine.