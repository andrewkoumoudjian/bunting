# Third-party notices — Bunting Market Terminal

Bunting's first-party files in this directory are Apache-2.0. The dependencies retain their own licenses.

## Zed GPUI

- Repository: `https://github.com/zed-industries/zed`
- Commit: `1a246efd7e1b83ab568ec5e3e6c1a43a42e1abba`
- License for `crates/gpui`: Apache-2.0
- Adapted path: `crates/gpui/examples/painting.rs`
- Retained behavior: the mouse-down, mouse-move and mouse-up pointer lifecycle used to begin, update and end a drag gesture.
- Local divergence: drawing paths became movable/resizable internal terminal panels with z-order, minimum sizes, close and minimize state.
- Dependency paths: `crates/gpui` and `crates/gpui_platform`, pinned to the commit above.

Zed's separate `crates/ui` package is GPL-3.0-or-later. Its implementation is not copied into Bunting; it is consulted only for public API and interaction-design research.

## GPUI Component

- Repository: `https://github.com/longbridge/gpui-component`
- Commit and direct git dependency: `88f102d13654fe25aa2fede076274b6b751a3704`
- Packages: `gpui-component` (`crates/ui`, package version `0.5.2`) and `gpui-component-assets` (`crates/assets`, package version `0.5.1`)
- License: Apache-2.0
- Approved source surface: the complete `crates/ui/src` tree, including title bar, status bar, dock, resizable, table, chart, plot, input, button, menu, popover, dialog, notification, sidebar, skeleton, progress, tabs, tooltips, badges, lists, trees, theme, virtual-list and window helpers.
- Currently used/adapted paths:
  - `crates/ui/src/lib.rs` for component initialization and exported component surface;
  - `crates/ui/src/root.rs` for application root ownership;
  - `crates/ui/src/input/**` and `crates/story/src/stories/input_story.rs` for `InputState`/`Input` ownership;
  - `crates/ui/src/button.rs` for button variants;
  - `crates/ui/src/chart/**` for `CandlestickChart`;
  - `crates/ui/src/status_bar.rs`, `crates/ui/src/title_bar.rs`, `crates/ui/src/menu/**`, `crates/ui/src/tooltip.rs`, and `crates/ui/src/badge.rs` as the preferred terminal chrome surface;
  - `crates/ui/src/dock/**`, `crates/ui/src/resizable/**`, and `crates/story/examples/dock.rs` as the preferred saved-layout and split-pane source when a docked mode is added.
- Local divergence: components are composed into Bunting-specific market panels and operate only on shared Bunting FIX projections and commands. The floating-window geometry and authority boundary remain first-party Bunting code.

## Comet

- Repository: `https://github.com/zeronsh/comet`
- Commit: `e5d8e9fb4c2ffe2350e4114db3bfd89979a2136d`
- License: MIT
- Referenced path: `crates/ui/src/lib.rs`
- Retained behavior: a single native GPUI application window with a minimum size and a first-party root shell.
- Local divergence: Bunting uses a professional terminal shell and an internal floating market-window canvas instead of Comet's agent workspace.

## Additional audited design references not copied

- `https://github.com/vicanso/zedis` — Apache-2.0; data-heavy connection, tabs, status and command-palette organization.
- `https://github.com/futureboard/Futureboard` — Apache-2.0; spatial canvas, dragging, selection and status-bar organization.
- `https://github.com/lumehq/coop@bb455871e536273d3366ce2ac9578cdcb65aab85` — GPL-3.0-or-later; polished cross-platform GPUI shell, theme switching, conversation navigation and responsive layout concepts only. No source copied.
- `https://github.com/packetThrower/zorite@86a52230cbc6d1cd75f4d0a635643a5c9402b021` — GPL-3.0-or-later; tabs, persisted workspace state, search/command interaction, theme tokens, virtualized long-form views and infinite-canvas concepts only. No source copied.
- `https://github.com/zealsprince/rox` — AGPL-3.0; panel-composition concept only. No source copied.
- `https://github.com/zed-industries/zed/tree/main/crates/ui/src` — GPL-3.0-or-later; public design/API reference only. No source copied.
