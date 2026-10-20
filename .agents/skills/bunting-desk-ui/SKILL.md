---
name: bunting-desk-ui
description: Bunting's rules for building and reviewing the look and feel of the GPUI desktop trading app `apps/bunting-desk` (ADR 0050, ADR 0051) — the floating-window canvas, window frame, sidebar, title bar, panes, themes, density, typography, empty states and trading-specific visual rules. Read it before writing or reviewing any visible change under apps/bunting-desk, together with the gpui-kit and gpui-kit-design-guides skills. Not for engine, protocol or server work.
---

# Bunting desk UI

The desk is a dense, keyboard-first trading workspace. The target is "RIT,
highly modernised": Godel/Bloomberg-style free windows on a canvas, inside a
quiet Delta/Zed-style frame. The owner's complaints about the previous build
were concrete: badly styled buttons, uncentred content and an ugly overall
look. Every rule below exists to prevent one of them.

## Read first, in this order

1. `docs/adr/0051-desk-workspace-canvas-of-floating-windows.md`, the
   decision, and `apps/bunting-desk/AGENTS.md`.
2. The **gpui-kit-design-guides** skill: read its whole guide for a new
   surface. It is normative.
3. The **gpui-kit** skill: its Coding Guides, plus the reference file for
   the mechanism you touch.
   - **Version caveat.** The skill tracks gpui-kit `main`, but Bunting pins
     `gpui-kit =0.7.1`.
   - Before you use any API, confirm it exists in the pinned source:
     `~/.cargo/registry/src/*/gpui-component-0.7.1`, `gpui-base-0.7.1`,
     `gpui-kit-0.7.1` and `gpui-pre-0.3.8`.
   - Never invent an API. Never translate a React/CSS idea by analogy.
4. [references/visual-spec.md](references/visual-spec.md): Bunting's
   tokens, sizes and per-surface recipes.
5. [references/reference-apps.md](references/reference-apps.md): which
   real GPUI apps to study for what, with licence limits.
6. Before finishing: [references/review-checklist.md](references/review-checklist.md).

Optional companions, if installed: `design-critique` (structured
critique of a screenshot), `design-system` (token consistency audits),
`accessibility-review` (contrast and keyboard), `ux-copy` (labels and
empty states).

## Non-negotiables

- **Tokens only.** Colours come from `cx.theme()`. Write no hex, `rgb()`
  or `hsla()` literals in view code. A colour the theme lacks becomes a
  named token in `assets/theme.json` and in every bundled theme.
- **One frame, one title.** Every pane is drawn inside the canvas's
  `WindowFrame`. Panes never draw their own title row.
- **Centre what sits on one line.** Every row of mixed icon, text and
  buttons is an `h_flex()` with `items_center()` and an explicit height.
  Text inside controls takes the control's line height; never set a
  different `line_height` on a child of a fixed-height row.
- **Use gpui-kit components; don't hand-roll them.**
  - Buttons: `Button` with one size per region (`xsmall` in title strips
    and toolbars, `small` in forms).
  - Inputs: `Input` or `NumberInput`.
  - Tables: virtualised `Table`/`DataTable`.
  - Menus and popovers: `PopupMenu`/`Popover`.
  - Command palette: `Command`.
- **Density with air.** Spacing sits on the 4 px grid; content is never
  flush against a border. 8 px is the minimum inner padding of any
  window body.
- **Typography.**
  - The sans (system SF Pro on macOS, Inter elsewhere) everywhere.
  - Tabular figures for every number.
  - The serif (Source Serif 4 Display) appears only for the screen name
    in the title bar and on the empty canvas, at 15 px or more.
- **Trading realism (ADR 0050 decision 7) beats aesthetics.**
  - Every order shows its venue.
  - Books are never merged.
  - Latency shown is measured.
  - The kill switch is one action with a confirmation.
- **No explanatory paragraphs in the chrome.** If something needs
  explaining, it becomes a tooltip or an empty state with an action.

## Visual acceptance

Compilation and headless window tests are not visual acceptance. The
coordinator runs the real app against a local venue on a headless Hyprland
output and judges screenshots. An agent without a display must:

- say which screens and states it changed;
- list every visual choice it was unsure about;
- never claim a screen "looks right".
