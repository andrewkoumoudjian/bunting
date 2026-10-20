# Reference apps for the desk UI

Study these for patterns. **Licence decides what may be copied.**

- Apache-2.0 and MIT sources may be adapted, with the repository, commit
  and path recorded in the commit message and the file header.
- GPL sources are read-only references: look at structure and numbers,
  then write Bunting's own code.
- Unpublished apps are visual references only.

## GPUI apps

| App | Licence | What to study |
|---|---|---|
| **gpui-kit** (Longbridge), <https://github.com/longbridge/gpui-kit> | Apache-2.0 (may adapt) | `crates/story` (the component gallery: every component in every state, idiomatic composition), `examples/dock`, `examples/sidebar`, `examples/system_monitor` (live charts at frame rate), `examples/dialog_overlay`, `examples/focus_trap`, `themes/` (JSON theme format). The pinned 0.7.1 source in `~/.cargo/registry` is authoritative for APIs. |
| **Zed**, <https://github.com/zed-industries/zed> | GPL-3.0-or-later (`crates/ui`, `workspace`, `title_bar`). Read-only: the owner declined relicensing the desk to GPL (2026-10-20). Take design values (sizes, structure, states) and write Bunting code on gpui-kit; never copy or line-by-line rewrite Zed source. `gpui` itself is Apache-2.0. | Density and chrome: see the table below. |
| **Longbridge Pro** (desktop trading app built on gpui-component) | proprietary | The closest product analogue: dense, real-time market screens in GPUI. Visual reference only. |

What to look at in Zed:

- `crates/ui/src/components/button/button_like.rs`: button sizes 32/28/22/18 and padding by size.
- `crates/ui/src/components/tab.rs` and `tab_bar.rs`: tab strip height and active-tab treatment.
- `crates/title_bar/src/title_bar.rs`: one-row title bar layout.
- `crates/workspace/src/pane.rs`, `dock.rs` and `status_bar.rs`: pane headers, dock chrome, status line.
- `crates/ui/src/styles/{spacing,typography,elevation}.rs`: spacing scale, type ramp, elevation levels.
- `crates/ui/src/components/keybinding_hint.rs`: key hints.
- `assets/themes/one/one.json`: a complete, well-balanced dark/light theme pair.
- `crates/sidebar/src/sidebar.rs` (`render_list_entry`, `render_thread`), `thread_switcher.rs` and `crates/agent_ui/src`: Zed's threads sidebar and agent panel. Delta's shell (sections such as Pinned, Projects and Threads, with dense rows) appears to be built from these (*inferred*: the public monorepo has no `delta` crate, only Delta artwork in `assets/images/delta_*` and `crates/ui/src/components/ai/delta_illustration.rs`). They are the closest readable source for the Delta sidebar look. GPL: take the design values, not the code.

Other GPUI apps listed in <https://github.com/zed-industries/awesome-gpui>, for example DBFlux, GitComet, Arbor and Zedis, show further real-world dense layouts. Check each licence before reading code with intent to adapt.

## Non-GPUI visual references

| App | Licence | Use |
|---|---|---|
| **Delta** (Zed Industries), <https://delta.dev> | unpublished | The frame the owner wants. Its app shell has:<br>• a narrow left sidebar with sentence-case muted section headers ("Pinned", "Projects", "Threads") and 24–28 px rows;<br>• a thin top strip of ghost icon buttons;<br>• content panes separated by 1 px borders, flat surfaces and no shadows;<br>• monospace accents for labels;<br>• an accent colour used sparingly. |
| **Buzz** (Block), <https://github.com/block/buzz> | Apache-2.0 | A modern workspace UI, but React/Tailwind/shadcn, **not GPUI**. Use `desktop/src/shared/styles/globals.css` only as a token and palette reference; there is no code to adapt into GPUI. |
| **Godel Terminal**, <https://godelterminal.com> | proprietary | Interaction model: a canvas of free windows, a command line with function codes, colour link groups. Visual reference only. |
| **RIT Client** (Rotman), feature guide PDF | proprietary | Module set and trader workflows to keep (Book Trader, Ladder, Order Entry, Blotter, Portfolio, Trader Info, News, Tenders). It is the legacy look to move away from. |

## Local mirrors (coordinator machine)

`~/.cache/bunting-skills-src/` holds mirrors that agents can read but
not write:

| Path | Contents |
|---|---|
| `gpui-kit/` | Sparse: `skills`, `crates/story`, `examples`, `themes`, at `4d63c73` |
| `zed/` | Sparse: `crates/ui`, `workspace`, `title_bar`, `theme`, `command_palette`, `settings_ui`, `sidebar`, `agent_ui`, `gpui/examples` and `assets/themes`, at `1d8a29e` |
| `buzz/globals.css` | Buzz's token sheet |
| `refs/delta.dev.png`, `refs/delta-app.png` | Screenshots of Delta's site and app shell, 2026-10-20 |

Mirrors are evidence for reading, not dependencies. Production code never
paths into them.
