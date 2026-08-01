# GPUI market-terminal reference inventory

Status: source-backed adoption record for `apps/bunting-terminal`; no external application backend or market authority is adopted.

## Decision

The GPUI application is a new Bunting client over the existing native FIX stack. `bunting-engine`, server transactions and authenticated projections remain authoritative. The desktop application owns presentation, local workspace geometry and bounded command submission only.

The required professional-terminal workspace is implemented as one native GPUI window with movable, resizable, minimizable and closable internal market-tool windows. This is intentionally different from opening many operating-system windows: tool windows share a function bar, status line, connection, session and workspace preset.

## Approved dependencies and source adaptation

| Source | Exact identity | License | Portable material adopted | Material rejected |
|---|---|---|---|---|
| Zed GPUI | `zed-industries/zed@1a246efd7e1b83ab568ec5e3e6c1a43a42e1abba` | `crates/gpui`: Apache-2.0 | GPUI runtime/platform dependency; pointer gesture lifecycle from `crates/gpui/examples/painting.rs`; window APIs | Zed editor services and the GPL-3.0-or-later `crates/ui` implementation |
| gpui-component | `longbridge/gpui-component@88f102d13654fe25aa2fede076274b6b751a3704`; crates `0.5.2`/`0.5.1` | Apache-2.0 | Root, Input/InputState, Button variants, CandlestickChart, component initialization and assets | editor/markdown/tree-sitter feature surface not needed by the terminal |
| Comet | `zeronsh/comet@e5d8e9fb4c2ffe2350e4114db3bfd89979a2136d` | MIT | single-window application-shell boundary and minimum-window sizing | agent engine, auth, synchronization, terminal emulator and private product behavior |

The GPUI revision above is the exact git source recorded for `gpui` in the audited gpui-component 0.5.2 lockfile and currently uses Rust 1.95. The desktop app is therefore a standalone workspace; the Bunting engine/Worker workspace remains pinned to Rust 1.88.

## Awesome GPUI portability review

The project list at `zed-industries/awesome-gpui` was reviewed for transferable architecture rather than original product purpose.

| Application | License/evidence | Useful transferable parts | Bunting disposition |
|---|---|---|---|
| Zedis | Apache-2.0; `vicanso/zedis` | dense data application organization, connection sidebar, tabs, status bar, command palette, charts and gpui-component composition | approved design and selective future source-adaptation reference; no backend reuse |
| Futureboard | Apache-2.0; `futureboard/Futureboard` | spatial canvas, drag/selection interactions, grid and status organization | approved canvas/interaction reference; Bunting's first implementation uses the smaller GPUI example gesture instead of copying its collaboration stack |
| Comet | MIT; identity above | polished shell, single-window lifecycle and root-view organization | approved small shell-pattern reference |
| Omi | MIT; `BasedHardware/omi` | production GPUI app organization and theming | future packaging/theme reference |
| DBFlux / ZQLZ / Arbor | project-specific licenses require file review | data-browser panes, tables, tree navigation and database workspaces | future component inventory only; no current source copy |
| tty7 / termy | project-specific licenses require file review | terminal embedding and process panels | not needed for FIX market-terminal authority; possible future strategy-console panel |
| Rox | AGPL-3.0 | composable panel and terminal concepts | concept only; source copying prohibited for this Apache-2.0 app |

## Implemented feature-to-reference map

| Bunting requirement | Implementation | Source influence |
|---|---|---|
| one polished terminal window with floating tools | `Terminal` workspace plus `PanelState`, pointer gestures and z-order | GPUI painting example; Comet shell boundary; Bloomberg/Godel visual references supplied by the user |
| charting | bounded quote candles through gpui-component `CandlestickChart` | gpui-component chart public API |
| order entry, best-price trading and cancellation | `bunting_tui::client` messages over bounded `IoTask` channels | existing Bunting TUI and FIX contracts; no external trading code |
| book, fills, account, news, tenders, risk and score | shared `FixClient` projections | existing Bunting reducer and RIT-class specification |
| competition controls | authenticated `competition_action` messages | existing Bunting role/lifecycle contract |
| command field and workspace presets | GPUI Input/Button composition and first-party preset reducer | professional terminal interaction pattern; Zedis command organization as a design reference |
| session diagnostics and recovery | existing redacted FIX logs, state, cursor and reconnect commands | existing Bunting client only |

## License boundary

Verbatim or close adaptation is limited to Apache-2.0 or MIT material listed in `apps/bunting-terminal/THIRD_PARTY_NOTICES.md`. GPL-3.0-or-later Zed UI code and AGPL-3.0 Rox code are not copied. Screenshots supplied by the user inform visual direction but are not redistributed as assets.

## Remaining work after the initial draft

The first slice exposes all currently shared participant projections and core privileged run actions. Exact Rotman UI/formula equivalence is not claimed. Additional server-backed panels should land only when authoritative projections exist, including raw/private open-order state, time-and-sales/trade OHLC, multi-listing symbol routing, OTC, facilities, reports, historical analytics and saved custom workspace geometry.
