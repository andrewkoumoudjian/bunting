# Bunting trading terminal

This native Ratatui workstation adapts the Longbridge Terminal component
hierarchy and interaction model to Bunting. It starts a loopback FIX 4.4
acceptor backed by the real `bunting-engine`, then connects as a FIX initiator
over TCP. The market, orders and FIX-session tabs expose the engine book,
execution reports and bounded raw protocol traffic; overlays provide help,
order entry and the FIX console.

```bash
cargo run --locked -p bunting-tui
```

Use `--address 127.0.0.1:9880` to change the endpoint. Use `--remote` to skip
the embedded local market and connect to an existing compatible acceptor.
The loopback acceptor is a native test harness; the Cloudflare Worker remains
outbound-TCP-only and does not accept raw TCP.

The component adaptations and their licenses are recorded in
[`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
