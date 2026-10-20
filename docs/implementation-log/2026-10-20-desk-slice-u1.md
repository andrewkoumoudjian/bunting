# Desk slice U1 — screens on a canvas of floating windows (2026-10-20)

Branch `muse/desk-u1` on base `7b4b759`. Driven by the task file
`desk-u1.md` (ADR 0051 decisions 1, 2, 3, 5, 7), amended by
`desk-u1-visual.md` (the 6 px gutter), the `bunting-desk-ui` skill
(binding visual spec), and coordinator review 1.

## Commits

1. `842a858` — pure canvas window geometry with a 6 px gutter.
   `apps/bunting-desk/src/canvas/geometry.rs`: `Rect`/`Point`/`Edge`,
   `GUTTER = 6.`, `snap_move`/`snap_resize` (aligned exact, adjacent and
   canvas edges keep the gutter, returning `Guide` lines), `resize_rect`
   and `enforce_min`, `clamp_to_canvas` (title strip stays reachable),
   `hit_edge` (corners win), `tile` and `cascade` (inside the
   gutter-inset canvas), `place_fraction` for templates. No GPUI types;
   11 unit tests.
2. `5df6211` — the canvas and `WindowFrame`
   (`src/canvas/mod.rs`, `src/canvas/frame.rs`). `Canvas` owns its
   windows back-to-front: move by the title strip, resize from any edge
   or corner, snap while dragging with 1 px guide lines in
   `drag_border`, mouse-down brings to front (title press also focuses
   the pane; content press does not steal input focus), double-click
   title maximises within the gutter-inset canvas, minimise, close with
   a bounded reopen stack of 16, tile and cascade, render-time clamping
   that does not rewrite saved rects, and an empty state. Windows
   occlude, so grips of background windows never receive events through
   a foreground window (found by the deals test: a background grip stole
   a click meant for an input). Drag is tracked on both the canvas root
   and each window, because occlusion also blocks the root's handlers.
   `WindowFrame` is the one 28 px strip: 14 px icon muted, 12 px medium
   truncated title, instrument chip for panes that follow one, and
   20×20 minimise/maximise/close actions at opacity 0 unless the window
   is hovered or focused; close hovers danger with a foreground that
   stays legible on it (0.7.1's custom button variant has one foreground
   for all states, so it matches the ghost buttons' secondary
   foreground). `PaneKind` gains `min_size`, `default_size` and
   `follows_instrument`. Bounds and content: body `table.background`,
   strip `tab_bar.background`, 1 px `border`, focused border `ring` at
   60% alpha and the only shadow (`shadow_md`).
3. `7a6de0c` — screens v4 and the Delta-style frame. `src/layouts.rs`:
   layout file version 4; a file of any other version is renamed to
   `layouts.v3.json` beside it (numbered if taken), never deleted, and
   the desk starts from defaults. `Saved` becomes `Screen` holding
   `Option<Vec<SavedWindow>>` (`kind`, `rect`, `z`, `minimised`,
   `maximised`); panes dump no custom state, verified by reading every
   panel constructor. `Preset`s are templates of canvas fractions
   (Trading 7 windows with blotter/fills/portfolio/trader info separate
   and no news/deals/notices; Market Making 8; Arbitrage 8), placed
   with the gutter between and around; a unit test places every
   template on 1040×660 and 1680×960 and asserts no overlap, gutter
   spacing, on-canvas, and no window below its minimum — minimum sizes
   were lowered to usable floors (Book Trader 360×200, ladder/ticket
   240×260, tables 140 high) so templates do not fight the resize
   clamp. `src/workspace.rs`: 36 px title bar in three flex columns
   (sidebar toggle and serif 15 px screen name; a 400×26 search field
   with the real `Kbd` binding that opens the palette — command codes
   are U3; connection dot with the desk label, per-venue round trips,
   run chip, kill-switch chip), 240 px sidebar (sentence-case 12 px
   semibold section headers with trailing "+" dropdown menus dispatching
   the existing actions; 28 px rows, truncating labels; Screens with
   templates/inline rename (Enter or blur commits, Escape cancels) and
   confirmed delete; Windows listing the screen's windows, minimised
   ones muted with a trailing minus; Run with status, logical time and
   halts), 24 px status line. Actions: ToggleSidebar (secondary-b),
   TileWindows (secondary-alt-t), CascadeWindows (palette only),
   CloseWindow (secondary-w), ReopenClosedWindow (secondary-shift-t),
   FocusNext/PreviousWindow (ctrl-` / ctrl-shift-`), MaximizeWindow
   (palette and title double-click); all in the palette under Screen and
   Window groups. Every canvas edit emits `CanvasEvent::Changed` into
   the existing 500 ms debounced save. The dock (`DockArea`, tab
   groups, panel registry) is gone from product code. `src/main.rs`:
   `WindowOptions` gains `app_id: "bunting-desk"` and title "Bunting";
   the asset source composes gpui-kit's default bundle with two catalog
   icons (Pencil, Trash) via `icon_assets!`, following gpui-kit's
   examples (Apache-2.0, recorded in a comment at the use site).

## Checks

`CARGO_TARGET_DIR=$TMPDIR/desk-target bash tools/check_desk.sh` passes
on the final tree: `cargo metadata --locked`, `cargo fmt --all
--check`, `cargo clippy --locked --all-targets -- -D warnings`, and
`cargo test --locked` (24 library tests including 11 geometry and 5
layouts, 20 workspace integration tests, 0 failed). The workspace
tests drive the real GPUI surface throughout: the base panel suite is
unchanged in scope, plus title drag and corner resize by mouse, close
and reopen with geometry, tile via its keybinding compared against
`geometry::tile`, switch-and-back geometry restore, and a saved-file
round trip asserting identical windows and rects. Root-workspace gates
were not run; this slice touches only `apps/bunting-desk` and docs,
which the root workspace excludes (ADR 0050).

## Remains and choices made under uncertainty

- `background_gradient` has no token field in gpui-kit 0.7.1's theme
  structs, so the canvas paints `theme.background` (#0b0d10 in the
  shipped theme, against the gradient's #101216→#0b0d10).
- The close button's hover foreground cannot differ from its rest
  foreground in 0.7.1 (`ButtonCustomVariant` has a single foreground;
  an external `.hover()` panics with "hover style already set"). It
  uses `secondary_foreground`, matching the ghost buttons beside it and
  legible on danger.
- Account equity/P&L is not in the title bar: the model's account
  answer carries balances, not a computed equity, and deriving one is
  U2 pane work.
- Screen rows rename inline (not via the palette); NewWorkspace still
  creates a Trading screen directly.
- Visual review against a running build is the coordinator's: title
  strip centring, chip and button geometry, template feel at laptop
  size, and guide colour are the items to eyeball.

## Visual fixes after live review (same day)

- Templates now wait for the first painted canvas bounds before being
  placed, and a screen is never saved from a pending (unpainted)
  placement; previously the seed size froze the template short of the
  real canvas edges. Covered by a test whose canvas seed is
  deliberately wrong: after paint the Trading template's rightmost
  edge lands at canvas width minus the gutter.
- Screens remember the canvas size their rects were laid out for
  (`canvas_w`/`canvas_h` in the v4 file, defaulting to the current
  canvas when missing). When the painted size differs — startup on
  another display or a window resize — `geometry::reflow` scales every
  window proportionally inside the gutter-inset canvas, enforces
  minimum sizes, keeps windows inside, and records the new size;
  maximised windows stay maximised with their floating rect reflowed.
  Unit-tested 1600×940 → 1040×660 and back within a pixel.
- The active Screens row's rename/delete actions now live inside the
  row (hover-revealed, press does not switch screens), so its
  highlight spans the full sidebar width like every other row.

Gate after these fixes: `tools/check_desk.sh` green (26 library, 22
workspace tests).
- Reflow is non-cumulative: the canvas keeps a base layout (rects plus
  the canvas size they were laid out for, as saved) and derives live
  rects from it at the painted size. Only user edits (template
  placement, move/resize drag end, tile, cascade, open, close, reopen)
  rewrite the base at the then-current size, so shrinking below
  minimum sizes and growing back restores the layout exactly. Tested
  end to end with a real test-window resize to 700×500 and back.
