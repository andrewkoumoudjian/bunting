# GPUI market-terminal reference inventory

Status: source-backed adoption record for `apps/bunting-terminal`; no external application backend or market authority is adopted.

## Decision

The GPUI application is a new Bunting client over the existing native FIX stack. `bunting-engine`, server transactions, and authenticated projections remain authoritative. The desktop application owns presentation, local dock geometry, and bounded command submission only.

The terminal now directly translates the supplied Zed and gpui-component workspace patterns:

- one native application window;
- one component title bar and command/search surface;
- a `DockArea` pane tree with center, left, right, and bottom regions;
- tab groups inside docks;
- resizable and collapsible edge docks;
- closable and zoomable panels through the component `Panel` contract;
- a compact component status bar;
- explicit loading, stale, disconnected, and recovery states.

The first custom floating-window implementation was removed. Bunting no longer maintains panel positions, z-order, collision behavior, custom minimize controls, or a separate internal window manager.

## Approved dependencies and source adaptation

| Source | Exact identity | License | Portable material adopted | Material rejected |
|---|---|---|---|---|
| Zed GPUI | `zed-industries/zed@1a246efd7e1b83ab568ec5e3e6c1a43a42e1abba` | `crates/gpui`: Apache-2.0 | GPUI runtime/platform dependency and native application/window APIs | Zed editor services and GPL-3.0-or-later UI implementation source |
| Zed workspace UI | same pinned Zed identity | design reference where licensing prevents source reuse | pane-group hierarchy, native workspace shell, title/status hierarchy, tabs, edge docks, compact controls, and first-class connection/project states | editor-specific behavior, assets, and verbatim GPL UI code |
| gpui-component | direct git dependency `longbridge/gpui-component@88f102d13654fe25aa2fede076274b6b751a3704`; packages `gpui-component 0.5.2` and `gpui-component-assets 0.5.1` | Apache-2.0 | `Root`, `TitleBar`, `StatusBar`, `DockArea`, `DockItem`, `Panel`, tables, candlestick chart, inputs, buttons, spinners, tooltips, theme tokens, and window helpers | product-specific story/demo behavior and optional editor language features that do not serve the terminal |
| Comet | `zeronsh/comet@e5d8e9fb4c2ffe2350e4114db3bfd89979a2136d` | MIT | single-window application-shell boundary and minimum-window sizing | agent engine, auth, synchronization, terminal emulator, and private product behavior |

The GPUI revision above is the exact git source recorded for `gpui` in the audited gpui-component 0.5.2 lockfile and currently uses Rust 1.95. The desktop app is therefore a standalone workspace; the Bunting engine/Worker workspace remains pinned to Rust 1.88.

## Direct translation map

| Supplied reference pattern | Bunting market-terminal translation |
|---|---|
| Zed native workspace window | one Bunting terminal process and one native GPUI window |
| Zed central pane group | quote-candlestick market chart |
| Zed left dock | order book and tenders in tab groups |
| Zed right dock | order entry, account/positions, and risk/score in tab groups |
| Zed bottom dock | orders/fills, news, and FIX session diagnostics in tab groups |
| Zed pane tabs | named market-panel tabs using the component `Panel` contract |
| Zed pane zoom/close behavior | component-provided zoom and close controls; chart remains the non-closable center anchor |
| Zed dock resizing/collapse | component `DockArea` split handles and status-bar dock toggles |
| Zed title bar/tool strip | Bunting identity, command/search input, workspace presets, refresh, and reconnect |
| Zed status bar | live/stale state, status text, profile, role, FIX state, and committed sequence |
| Zed first-class unavailable state | prominent FIX disconnection banner and explicit offline panel bodies |
| gpui-component data surfaces | component tables, inputs, buttons, candlestick chart, spinner, tokens, and scrolling |

This is a domain translation, not a new interaction system. Only labels, panel contents, commands, and market semantics differ from the reference architecture.

## Workspace presets

Presets rebuild the same dock tree deterministically rather than placing arbitrary windows on a canvas.

| Preset | Center | Left dock | Right dock | Bottom dock |
|---|---|---|---|---|
| Trading | chart | book, tenders | order entry, account, risk | orders/fills, news, FIX session |
| Research | chart | news, book | account, risk | FIX session, orders/fills |
| Competition | chart | competition controls, tenders | account, risk, order entry | orders/fills, news, FIX session |

Dock geometry is persisted under a versioned workspace key. Changing the dock schema increments that version and resets incompatible saved geometry.

## Component adoption

The component library is the default source for standard UI behavior. Bunting-specific code remains only where the terminal requires market semantics.

| Component family | Terminal use |
|---|---|
| `TitleBar`, window helpers, and theme | native outer chrome and consistent terminal tokens |
| `StatusBar`, tooltips, and icons | live/stale/session/role/sequence state and dock visibility controls |
| `DockArea`, `DockItem`, `Panel`, resizable, and tabs | the primary workspace architecture, saved geometry, pane tabs, close, zoom, and edge collapse |
| buttons and inputs | command field, order entry, cancellation, tender actions, and run controls |
| `CandlestickChart` | bounded FIX L1 quote candles |
| tables and scrollable containers | order book, orders/fills, positions, and long diagnostic histories |
| spinner and explicit empty states | connection, recovery, loading, and offline states instead of blank panes |
| dialogs, sheets, popovers, menus, and notifications | approved for later confirmations and server feedback when the workflow requires them |

## Awesome GPUI portability review

The project list at `zed-industries/awesome-gpui` was reviewed for transferable architecture rather than original product purpose.

| Application | License/evidence | Useful transferable parts | Bunting disposition |
|---|---|---|---|
| Zedis | Apache-2.0; `vicanso/zedis` | dense data-application organization, tabs, status bar, command palette, charts, and gpui-component composition | approved design and selective source-adaptation reference; no backend reuse |
| Futureboard | Apache-2.0; `futureboard/Futureboard` | spatial canvas and drag interactions | reviewed but rejected for the primary shell because the user required the Zed workspace translation rather than a canvas |
| Comet | MIT; identity above | polished shell, single-window lifecycle, and root-view organization | approved small shell-pattern reference |
| Omi | MIT; `BasedHardware/omi` | production GPUI app organization and theming | packaging/theme reference |
| DBFlux / ZQLZ / Arbor | project-specific licenses require file review | data-browser panes, tables, tree navigation, and database workspaces | component inventory only; no current source copy |
| tty7 / termy | project-specific licenses require file review | terminal embedding and process panels | not needed for FIX market authority; possible later strategy-console panel |
| Coop | GPL-3.0-or-later; `lumehq/coop@bb455871e536273d3366ce2ac9578cdcb65aab85` | polished shell, theme switching, responsive navigation, and compact activity states | interaction reference only; no implementation copy into Apache-2.0 Bunting |
| Zorite | GPL-3.0-or-later; `packetThrower/zorite@86a52230cbc6d1cd75f4d0a635643a5c9402b021` | persisted tabs/workspace state, command/search interaction, tokens, and virtualized views | interaction and architecture reference only; no implementation copy into Apache-2.0 Bunting |
| Rox | AGPL-3.0 | composable panel and terminal concepts | concept only; source copying prohibited for this Apache-2.0 app |

## Implemented feature-to-reference map

| Bunting requirement | Implementation | Source influence |
|---|---|---|
| one polished terminal workspace | component `TitleBar`, `DockArea`, `DockItem`, `Panel`, and `StatusBar` | Zed workspace hierarchy translated through gpui-component APIs |
| no overlapping panes | dock-managed center and edge regions with component split handles and tabs | Zed pane groups and edge docks |
| charting | bounded quote candles through `CandlestickChart` | gpui-component public API |
| order entry, best-price trading, and cancellation | component inputs/buttons over `bunting_tui::client` messages and bounded `IoTask` channels | existing Bunting TUI and FIX contracts; no external trading code |
| book, fills, account, news, tenders, risk, and score | component tables/panels over shared `FixClient` projections | existing Bunting reducer and RIT-class specification |
| competition controls | authenticated `competition_action` messages in the Competition dock | existing Bunting role/lifecycle contract |
| command field and presets | title-bar `Input`/`Button` composition and deterministic dock-tree reducer | Zed command/title organization and workspace switching |
| session diagnostics and recovery | bottom FIX panel, status bar, banner, redacted logs, cursor, and reconnect commands | Zed status hierarchy plus existing Bunting client |

## License boundary

Verbatim or close adaptation is limited to Apache-2.0 or MIT material listed in `apps/bunting-terminal/THIRD_PARTY_NOTICES.md`. The approved Apache-2.0 longbridge component surface is authorized. GPL-3.0-or-later Zed UI, Coop, and Zorite code, plus AGPL-3.0 Rox code, are not copied. Screenshots supplied by the user inform the use-case mapping but are not redistributed as assets.

## Validation policy

GitHub Actions is diagnostic evidence, not a delivery gate. Branch-caused compiler or correctness findings are still fixed when visible. The dedicated macOS workflow builds the ARM64 application, packages the DMG, runs tests, runs Clippy with warnings denied, and checks formatting against the exact branch head.

## Remaining work

The current slice exposes the shared participant projections and core privileged run actions. Exact Rotman UI/formula equivalence is not claimed. Additional server-backed panels should land only when authoritative projections exist, including raw/private open-order state, time-and-sales/trade OHLC, multi-listing symbol routing, OTC, facilities, reports, and historical analytics.

Visual runtime verification still requires launching the packaged macOS application against a reachable FIX server. A successful compiler/package workflow confirms API compatibility and distributable construction, not subjective visual acceptance or live-server behavior.
