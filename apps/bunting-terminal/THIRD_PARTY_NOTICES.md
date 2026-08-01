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
- Audited commit: `88f102d13654fe25aa2fede076274b6b751a3704`
- Dependency releases: `gpui-component = 0.5.2`, `gpui-component-assets = 0.5.1`
- License: Apache-2.0
- Referenced/adapted paths:
  - `examples/hello_world/src/main.rs` for `gpui_component::init` and `Root` initialization;
  - `examples/input/src/main.rs` and `crates/story/src/stories/input_story.rs` for `InputState`/`Input` ownership;
  - `docs/docs/components/chart.md` for `CandlestickChart` construction;
  - public Button variants and theme-compatible component APIs.
- Local divergence: components are composed into Bunting-specific market panels and operate only on shared Bunting FIX projections and commands.

## Comet

- Repository: `https://github.com/zeronsh/comet`
- Commit: `e5d8e9fb4c2ffe2350e4114db3bfd89979a2136d`
- License: MIT
- Referenced path: `crates/ui/src/lib.rs`
- Retained behavior: a single native GPUI application window with a minimum size and a first-party root shell.
- Local divergence: Bunting uses standard native titlebar chrome and an internal floating market-window canvas instead of Comet's agent workspace.

## Additional audited design references not copied

- `https://github.com/vicanso/zedis` — Apache-2.0; data-heavy connection, tabs, status and command-palette organization.
- `https://github.com/futureboard/Futureboard` — Apache-2.0; spatial canvas, dragging, selection and status-bar organization.
- `https://github.com/zealsprince/rox` — AGPL-3.0; panel-composition concept only. No source copied.
- `https://github.com/zed-industries/zed/tree/main/crates/ui/src` — GPL-3.0-or-later; public design/API reference only. No source copied.
