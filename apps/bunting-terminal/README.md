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
- explicit disconnected banner, offline panel states, and dock/status controls;
- app-managed local Wasmer/WASIX server startup using the bundled portable server module.

The application does not contain a matcher or a second account model. It imports the same native profile, TLS, FIX-session, recovery, bounded-channel, and projection reducer used by `bunting-tui`.

## Start the bundled local WASM server

The macOS DMG includes:

```text
Bunting Market Terminal.app/Contents/Resources/server/
├── bunting-server.wasm
├── local.json
└── scenario.json
```

Install Wasmer 7.2.1, open the app, and press **Start Server** in the title bar or disconnected-state banner. The app:

1. finds Wasmer through `WASMER_BIN`, the app resources, `~/.wasmer/bin`, Homebrew, or `PATH`;
2. copies the local config and scenario to a writable application-support directory on first use;
3. launches the portable module directly with `wasmer run` and no shell;
4. grants only directories derived from the selected config;
5. detects readiness on the loopback FIX endpoint and reconnects the terminal;
6. writes server output to `bunting-server.log`;
7. stops only the child process that it owns when requested or when the app exits.

Writable state and logs live at:

```text
~/Library/Application Support/Bunting Market Terminal/server
```

Operator-edited config and scenario files are preserved across launches. If another process already listens on `127.0.0.1:9880`, the terminal marks it as `EXTERNAL`, reconnects, and never tries to stop it.

Local-server overrides:

```text
WASMER_BIN=/absolute/path/to/wasmer
BUNTING_SERVER_ARTIFACT=/absolute/path/to/bunting-server.wasm
BUNTING_SERVER_CONFIG=/absolute/path/to/local.json
```

The bundled config listens only on loopback. Selecting a different config is an operator action and does not change the terminal's server-authority boundary.

## Direct reference translation

| Reference pattern | Bunting translation |
|---|---|
| Zed native workspace window | Bunting's single market-terminal window |
| Zed pane groups and tab strips | market chart, book, order, account, risk, news, tender, competition, and FIX panels |
| Zed left/right/bottom docks | market depth and tenders; order/account/risk; orders/news/session |
| Zed title bar and toolbar | command/search field, workspace presets, local server, refresh, and reconnect |
| Zed status bar | local WASM state, connection, profile, role, FIX state, and committed sequence |
| gpui-component loading and empty states | explicit waiting, stale, and disconnected states instead of blank panes |

The translation changes the domain semantics only. The shell, docking, tabbing, status hierarchy, and component behavior follow the supplied GPUI references rather than a custom terminal windowing concept.

## Component system

The terminal directly pins `longbridge/gpui-component@88f102d13654fe25aa2fede076274b6b751a3704`. Its Apache-2.0 component surface supplies the root, title bar, status bar, docks, tabs, resizable panes, tables, charts, inputs, buttons, tooltips, spinners, themes, and window helpers used by the terminal.

Custom UI code is limited to Bunting market composition, market-specific commands, and local process lifecycle. Coop and Zorite remain useful GPUI interaction references, but both are GPL-3.0-or-later; their source is not copied into this Apache-2.0 application.

## Build

The pinned GPUI/component source requires Rust 1.95, so this standalone app carries a local toolchain file and is intentionally excluded from the repository's Rust 1.88 workspace.

```bash
cargo run --manifest-path apps/bunting-terminal/Cargo.toml
```

For the **Start Server** helper to work in a source checkout, first build the portable server module and install Wasmer:

```bash
tools/build_wasi_server.sh
cargo run --manifest-path apps/bunting-terminal/Cargo.toml
```

The default profile is `local` and connects to `127.0.0.1:9880`. The local-development credential fallback matches `bunting-tui`.

Terminal environment overrides:

```text
BUNTING_TERMINAL_CONFIG=/path/to/terminal.json
BUNTING_TERMINAL_PROFILE=local|remote|cloudflare-gateway
BUNTING_TERMINAL_ENDPOINT=host:port
BUNTING_TERMINAL_PASSWORD=process-only-password-override
```

Production credentials should use the selected profile's existing `password_env`; `BUNTING_TERMINAL_PASSWORD` is an explicit process-only override and is never persisted or logged.

## Command field

The title-bar command field accepts `TRADING`, `RESEARCH`, `COMP`, `REFRESH`, `RECONNECT`, `GO`, `BOOK`, `BNT`, and their documented aliases.

Local-server commands:

```text
SERVER
START SERVER
SERVER START
WASM
STOP SERVER
SERVER STOP
```

Workspace buttons call the same deterministic layout reducer.

## Units and authority

Prices are displayed in Bunting price ticks, quantity in lots, and account values in exact minor units. Quote candles summarize bid/ask snapshots; they are not represented as authoritative trade OHLC bars. All commands remain subject to server-side authentication, role, risk, run lifecycle, idempotency, and FIX sequence rules.

Starting the local process does not grant the client administrative authority. It only starts the existing server artifact and then reconnects through the normal authenticated FIX client.

## macOS ARM64 package

The release workflow builds and smoke-tests the portable server on Linux, builds and tests the native terminal on macOS with Rust 1.95, then places the portable `.wasm`, config, and scenario inside the signed `.app` before creating the DMG.

The preview is ad-hoc signed and not Apple-notarized. On first launch, macOS may require Control-clicking the application and selecting **Open**.

## Validation

The macOS workflow verifies:

- the portable `.wasm` release input with Wasmer 7.2.1;
- local server health and authenticated admin reads;
- terminal tests;
- Clippy with warnings denied;
- formatting;
- native Apple Silicon Mach-O output;
- bundled server resources;
- app code signature;
- compressed DMG and SHA-256 checksum.

GitHub Actions is build and release evidence, but runtime visual acceptance still requires launching the packaged app on an Apple Silicon Mac.

## Source provenance

See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md) and [`../../docs/gpui-terminal-reference-inventory.md`](../../docs/gpui-terminal-reference-inventory.md). The application uses Apache-2.0 GPUI and the approved gpui-component surface, plus an MIT Comet shell pattern. GPL/AGPL applications are reference-only.
