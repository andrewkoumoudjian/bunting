# GPUI market-terminal reference inventory

Status: source-backed adoption record for `apps/bunting-terminal`; no external application backend or market authority is adopted.

## Decision

The GPUI application is a new Bunting client over the existing native FIX stack. `bunting-engine`, server transactions and authenticated projections remain authoritative. The desktop application owns presentation, local workspace geometry and bounded command submission only.

The required professional-terminal workspace is implemented as one native GPUI window with movable, resizable, minimizable and closable internal market-tool windows. This is intentionally different from opening many operating-system windows: tool windows share a function bar, status line, connection, session and workspace preset.

## Approved dependencies and source adaptation

| Source | Exact identity | License | Portable material adopted | Material rejected |
|---|---|---|---|---|
| Zed GPUI | `zed-industries/zed@1a246efd7e1b83ab568ec5e3e6c1a43a42e1abba` | `crates/gpui`: Apache-2.0 | GPUI runtime/platform dependency; pointer gesture lifecycle from `crates/gpui/examples/painting.rs`; window APIs | Zed editor services and the GPL-3.0-or-later `crates/ui` implementation |
| gpui-component | direct git dependency `longbridge/gpui-component@88f102d13654fe25aa2fede076274b6b751a3704`; packages `gpui-component 0.5.2` and `gpui-component-assets 0.5.1` | Apache-2.0 | the complete `crates/ui/src` component surface is approved: Root, TitleBar, StatusBar, dock, resizable, table, chart, plot, input, button, menu, popover, dialog, notification, sidebar, skeleton, progress, tabs, tooltips, badges, lists, trees, themes, virtual lists and window helpers | only product-specific story/demo behavior and optional editor language features that do not serve the terminal |
| Comet | `zeronsh/comet@e5d8e9fb4c2ffe2350e4114db3bfd89979a2136d` | MIT | single-window application-shell boundary and minimum-window sizing | agent engine, auth, synchronization, terminal emulator and private product behavior |

The GPUI revision above is the exact git source recorded for `gpui` in the audited gpui-component 0.5.2 lockfile and currently uses Rust 1.95. The desktop app is therefore a standalone workspace; the Bunting engine/Worker workspace remains pinned to Rust 1.88.

## Component adoption plan

The component library is the default source for standard UI behavior. Bunting-specific code remains only where the terminal requires market semantics or true free-floating internal windows.

| Component family | Terminal use |
|---|---|
| `TitleBar`, window helpers and theme | polished cross-platform outer chrome and consistent terminal tokens |
| `StatusBar`, badges, tooltips and icons | live/stale/session/role/sequence state and compact discoverable controls |
| buttons, inputs, forms, selects, checkboxes, switches and radio | command field, order entry, filters, strategy controls and confirmations |
| charts and plots | quote candles, depth, P&L, score and risk visualizations |
| tables, lists, trees, pagination and virtual lists | book, orders, fills, positions, news, reports, instruments and long histories |
| dialogs, sheets, popovers, menus and notifications | destructive confirmations, errors, command palette, panel launcher and server feedback |
| dock, resizable and tabs | optional saved docked layouts and tab groups alongside the primary Bloomberg-style floating canvas |
| sidebar, collapsible, accordion and group boxes | instrument navigation, research drill-down and compact configuration surfaces |
| skeleton, spinner and progress | explicit connection/recovery/loading/risk states instead of empty panes |

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
| Coop | GPL-3.0-or-later; `lumehq/coop@bb455871e536273d3366ce2ac9578cdcb65aab85` | polished cross-platform shell, theme switching, responsive navigation and compact activity states | interaction reference only; no implementation copy into Apache-2.0 Bunting |
| Zorite | GPL-3.0-or-later; `packetThrower/zorite@86a52230cbc6d1cd75f4d0a635643a5c9402b021` | persisted tabs/workspace state, command/search interaction, theme tokens, virtualized views and infinite-canvas UX | interaction and architecture reference only; no implementation copy into Apache-2.0 Bunting |
| Rox | AGPL-3.0 | composable panel and terminal concepts | concept only; source copying prohibited for this Apache-2.0 app |

## Implemented feature-to-reference map

| Bunting requirement | Implementation | Source influence |
|---|---|---|
| one polished terminal window with floating tools | component `TitleBar` shell plus `Terminal` workspace, `PanelState`, pointer gestures and z-order | gpui-component title bar; GPUI painting example; Comet shell boundary; Bloomberg/Godel visual references supplied by the user |
| charting | bounded quote candles through gpui-component `CandlestickChart` | gpui-component chart public API |
| order entry, best-price trading and cancellation | `bunting_tui::client` messages over bounded `IoTask` channels | existing Bunting TUI and FIX contracts; no external trading code |
| book, fills, account, news, tenders, risk and score | shared `FixClient` projections | existing Bunting reducer and RIT-class specification |
| competition controls | authenticated `competition_action` messages | existing Bunting role/lifecycle contract |
| command field and workspace presets | GPUI Input/Button composition and first-party preset reducer | professional terminal interaction pattern; Zedis command organization as a design reference |
| session diagnostics and recovery | existing redacted FIX logs, state, cursor and reconnect commands | existing Bunting client only |

## License boundary

Verbatim or close adaptation is limited to Apache-2.0 or MIT material listed in `apps/bunting-terminal/THIRD_PARTY_NOTICES.md`. The complete Apache-2.0 longbridge component tree is authorized. GPL-3.0-or-later Zed UI, Coop and Zorite code, plus AGPL-3.0 Rox code, are not copied. Screenshots supplied by the user inform visual direction but are not redistributed as assets.

## Validation policy

GitHub Actions is diagnostic evidence, not a delivery gate for this work. Branch-caused compiler or correctness findings should still be fixed when visible, but implementation continues and unresolved validation is reported explicitly rather than blocking on a workflow.

## Remaining work after the initial draft

The first slice exposes all currently shared participant projections and core privileged run actions. Exact Rotman UI/formula equivalence is not claimed. Additional server-backed panels should land only when authoritative projections exist, including raw/private open-order state, time-and-sales/trade OHLC, multi-listing symbol routing, OTC, facilities, reports, historical analytics and saved custom workspace geometry.
