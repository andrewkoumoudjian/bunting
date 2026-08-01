# Bunting Market Terminal

A native GPUI desktop client for the Bunting FIX server. It uses one native application window containing movable and resizable market-tool windows, following the workspace model of professional terminals rather than a fixed dashboard.

## Implemented terminal surface

- live quote-candlestick chart backed by bounded FIX L1 snapshots;
- aggregated bid/ask order book with best-price actions;
- limit and market order entry;
- cancel requests and execution-report history;
- authoritative cash, holdings, realized/unrealized P&L and local fill fallback;
- targeted news and tender accept/decline workflows;
- risk/score projection;
- instructor/administrator run controls, still verified by the server;
- redacted FIX diagnostics, sequence state, recovery status and reconnect;
- Trading, Research and Competition workspace presets;
- close, minimize, z-order, drag and resize for every internal tool window;
- a Bloomberg-style function field for workspace and panel commands.

The application does not contain a matcher or a second account model. It imports the same native profile, TLS, FIX-session, recovery, bounded-channel and projection reducer used by `bunting-tui`.

## Build

The GPUI revision used by `gpui-component 0.5.2` requires Rust 1.95, so this standalone app carries a local toolchain file and is intentionally excluded from the repository's Rust 1.88 workspace.

```bash
cargo run --manifest-path apps/bunting-terminal/Cargo.toml
```

The default profile is `local` and connects to `127.0.0.1:9880`. The local-development credential fallback matches `bunting-tui`.

Environment overrides:

```text
BUNTING_TERMINAL_CONFIG=/path/to/terminal.json
BUNTING_TERMINAL_PROFILE=local|remote|cloudflare-gateway
BUNTING_TERMINAL_ENDPOINT=host:port
BUNTING_TERMINAL_PASSWORD=process-only-password-override
```

Production credentials should use the selected profile's existing `password_env`; `BUNTING_TERMINAL_PASSWORD` is an explicit process-only override and is never persisted or logged.

## Function field

The top field currently accepts `TRADING`, `RESEARCH`, `COMP`, `REFRESH`, `RECONNECT`, `BOOK`, `ORDERS`, `NEWS`, `SESSION`, and `BNT`.

## Units and authority

Prices are displayed in Bunting price ticks, quantity in lots, and account values in exact minor units. Quote candles summarize bid/ask snapshots; they are not represented as authoritative trade OHLC bars. All commands remain subject to server-side authentication, role, risk, run lifecycle, idempotency and FIX sequence rules.

## Source provenance

See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and [`../../docs/gpui-terminal-reference-inventory.md`](../../docs/gpui-terminal-reference-inventory.md). The application uses Apache-2.0 GPUI and gpui-component code/API patterns and an MIT Comet shell pattern. GPL/AGPL applications are reference-only.
