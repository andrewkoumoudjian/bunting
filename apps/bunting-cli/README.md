# Bunting local market terminal

This native Ratatui terminal starts a loopback FIX 4.4 acceptor backed by the
real `bunting-engine`, then connects as a FIX initiator over TCP. It displays
the engine book and raw FIX traffic and supports limit/market orders,
cancel, replace, status, snapshot, logout, and quit commands.

```bash
cargo run --locked -p bunting-cli
```

Use `--address 127.0.0.1:9880` to change the endpoint. Use `--remote` to skip
the embedded local market and connect to an existing compatible acceptor.
The loopback acceptor is a native test harness; the Cloudflare Worker remains
outbound-TCP-only and does not accept raw TCP.

The component adaptations and their licenses are recorded in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
