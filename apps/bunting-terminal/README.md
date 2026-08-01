# Bunting Market Terminal

A native GPUI desktop client for the Bunting FIX server. The application deliberately translates Zed's workspace model into a market-terminal use case: one native application window, one component title bar, a dock-managed pane tree, tab groups, resizable edge docks, and a compact status bar.

It does not implement a second window manager or a free-floating panel canvas.

## Implemented terminal surface

- native GPUI application window and `gpui-component` title bar;
- command/search field and deterministic Trading, Research, and Competition workspace presets;
- `DockArea` center, left, right, and bottom regions with resizable, collapsible docks;
- tabbed, closable, and zoomable market panels using the component `Panel` contract;
- live quote-candlestick chart backed by bounded FIX L1 snapshots;
- aggregated bid/ask order book with best-price actions;
- limit and market order entry;
- cancel requests and execution-report history;
- authoritative cash, holdings, realized/unrealized P&L and local fill fallback;
- targeted news and tender accept/decline workflows;
- risk/score projection;
- instructor/administrator run controls, still verified by the server;
- redacted FIX diagnostics, sequence state, recovery status and reconnect;
- persistent dock geometry under a versioned workspace key;
- explicit disconnected banner, offline panel states, and dock/status controls.

The application does not contain a matcher or a second account model. It imports the same native profile, TLS, FIX-session, recovery, bounded-channel, and projection reducer used by `bunting-tui`.

## Direct reference translation

| Reference pattern | Bunting translation |
|---|---|
| Zed native workspace window | Bunting's single market-terminal window |
| Zed pane groups and tab strips | market chart, book, order, account, risk, news, tender, competition, and FIX panels |
| Zed left/right/bottom docks | market depth and tenders; order/account/risk; orders/news/session |
| Zed title bar and toolbar | command/search field, workspace presets, refresh, and reconnect |
| Zed status bar | connection, profile, role, FIX state, and committed sequence |
| gpui-component loading and empty states | explicit waiting, stale, and disconnected states instead of blank panes |

The translation changes the domain semantics only. The shell, docking, tabbing, status hierarchy, and component behavior follow the supplied GPUI references rather than a custom terminal windowing concept.

## Component system

The terminal directly pins `longbridge/gpui-component@88f102d13654fe25aa2fede076274b6b751a3704`. Its Apache-2.0 component surface supplies the root, title bar, status bar, docks, tabs, resizable panes, tables, charts, inputs, buttons, tooltips, spinners, themes, and window helpers used by the terminal.

Custom UI code is limited to Bunting market composition and market-specific commands. Coop and Zorite remain useful GPUI interaction references, but both are GPL-3.0-or-later; their source is not copied into this Apache-2.0 application.

## Build

The pinned GPUI/component source requires Rust 1.95, so this standalone app carries a local toolchain file and is intentionally excluded from the repository's Rust 1.88 workspace.

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

## Command field

The title-bar command field accepts `TRADING`, `RESEARCH`, `COMP`, `REFRESH`, `RECONNECT`, `GO`, `BOOK`, `BNT`, and their documented aliases. Workspace buttons call the same deterministic layout reducer.

## Units and authority

Prices are displayed in Bunting price ticks, quantity in lots, and account values in exact minor units. Quote candles summarize bid/ask snapshots; they are not represented as authoritative trade OHLC bars. All commands remain subject to server-side authentication, role, risk, run lifecycle, idempotency, and FIX sequence rules.

## Validation

The macOS workflow builds the Apple Silicon application, packages the DMG, runs tests, Clippy with warnings denied, and formatting diagnostics. GitHub Actions is diagnostic evidence rather than the product's delivery authority, but branch-caused findings are fixed before the preview is reported as validated.

## Source provenance

See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and [`../../docs/gpui-terminal-reference-inventory.md`](../../docs/gpui-terminal-reference-inventory.md). The application uses Apache-2.0 GPUI and the approved gpui-component surface, plus an MIT Comet shell pattern. GPL/AGPL applications are reference-only.
