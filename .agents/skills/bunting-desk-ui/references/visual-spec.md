# Bunting desk visual spec (ADR 0051)

Token names are `cx.theme()` fields mapped from
`apps/bunting-desk/assets/theme.json` (Bunting Dark). Sizes are logical
pixels at the default density. U2 makes density, font size and themes
user-configurable, so code must read these from one place (`src/theme.rs`
helpers) rather than repeating literals.

## Scale

| Thing | Value |
|---|---|
| Spacing grid | 4 px (2 px only inside chips and badges) |
| Window gutter (between windows, and canvas inset) | 6 px |
| Radii | controls 4 px, windows 6 px, popovers 6 px |
| Borders | 1 px, `border` (structure) or `input.border` (fields) |
| Control height | 22 (ghost icon buttons and toolbars), 26 (inputs and buttons), 30 (comfortable density) |
| Bars | title bar 36, window title strip 28, status line 24, sidebar row 28, table row 24 |
| Icons | 14 px in rows and strips, 16 px in empty states |
| Font sizes | 11 (chips, kbd, table heads), 12 (window titles, dense tables), 13 (body, sidebar rows), 15 (serif screen name), 22 (serif on empty canvas) |
| Weights | regular body, medium for titles and the active row, semibold for section headers |

Reference: Zed's dense chrome uses button heights 32/28/22/18
(`crates/ui/src/components/button/button_like.rs`, GPL, read-only). Delta's
app uses about 24 px rows and 13 px text.

## Surfaces

| Surface | Background | Border | Notes |
|---|---|---|---|
| Canvas | `background` | none | Flat, with no grid or pattern. |
| Window body | `list.background` / `table.background` | 1 px `border`, radius 6, `overflow_hidden` | — |
| Focused window | (same as above) | `ring` at about 60% alpha | One soft shadow (`shadow_md`). Unfocused windows get no shadow. |
| Window title strip | `tab_bar.background` | bottom 1 px `border` | — |
| Title bar | `title_bar.background` | bottom `title_bar.border` | — |
| Sidebar | `sidebar.background` | right `sidebar.border` | — |
| Status line | `status_bar.background` | top `status_bar.border` | — |
| Popovers, menus, palette | `popover.background` | 1 px `border` | Radius 6, `shadow_lg`. |

## Recipes

### WindowFrame title strip (28 px)

`h_flex().h(28).px(8).gap(6).items_center()`

- **Left:** a pane icon at 14 px in `muted.foreground`, then the title at
  12 px medium in `foreground` (sans), then an optional instrument chip:
  11 px text in `muted.foreground` on `muted.background`, radius 4,
  padding 2×6.
- **Right:** `Button::new(id).ghost().xsmall().icon(..)` for minimise
  (`Minus`), maximise (`Maximize`/`Minimize`) and close (`Close`), with
  2 px gaps.
  - The close button's hover uses `danger`.
  - All three are visible only while the window is hovered or focused.
    Hide them with opacity, not removal, so the layout never shifts.
- **Behaviour:** dragging the strip moves the window, and a double-click
  maximises or restores it.

### Sidebar (240 px, collapsible with ⌘/Ctrl+B)

Delta's app is the model. Sections are separated by space, not boxes.

- **Section header row:** 28 px with left padding 12, a 12 px semibold
  label in `muted.foreground`, sentence case ("Screens", "Windows",
  "Run"), and a trailing ghost xsmall `+` where relevant, right-aligned
  and centred.
- **Rows:** 28 px with left padding 12 and a horizontal margin of 4,
  radius 4. A 14 px icon, an 8 px gap, then a 13 px label that truncates
  with an ellipsis. Trailing values are right-aligned, tabular, in
  `muted.foreground`.
- **Row states:**
  - hover: `sidebar.accent.background`;
  - active: `sidebar.accent.background` plus `sidebar.accent.foreground`
    in medium weight;
  - minimised window rows: `muted.foreground`.
- No wordmark or logo block in the sidebar.

### Title bar (36 px)

- **Left:** the sidebar toggle (ghost xsmall `PanelLeft`), then the
  screen name in the serif at 15 px.
- **Centre:** the command field, 320–480 px wide and 26 px high, radius
  6, `secondary.background`, 1 px `input.border`.
  - Contents: a `Search` icon at 14 px in muted, a 12 px muted
    placeholder, and a `Kbd` hint (⌘K, or Ctrl K elsewhere) on the
    right.
  - Clicking it opens the palette or command line.
- **Right:** a connection dot (`success`/`danger`) with the venue label,
  per-venue measured round trips (`V1 296µs`, tabular, muted), then the
  account summary.

### Empty canvas

Centred both ways:

- the screen name in the serif at 22 px;
- a 13 px muted line, e.g. "Open a window with ⌘K, or start from a
  template";
- a row of `small` outline buttons: "Book Trader", "Trading template",
  "Market Making template".

### Pane bodies

- Inner padding is at least 8 px.
- Tables have a sticky 24 px header: 11 px semibold labels in
  `table.head.foreground`. Numeric columns are right-aligned, and header
  labels align with their column.
- Numbers use tabular figures. Prices are coloured by side (`chart.bullish`
  bid / `chart.bearish` ask); quantities are not coloured.
- Empty panes show an `EmptyState`: a 16 px muted icon, one 13 px muted
  line and the action that fills it. Never leave a large grey
  rectangle.
- Forms use label-over-field or one baseline per row. Field widths line
  up in a column, and a disabled primary action never spans the full
  width just to explain a missing choice. Say what is missing next to
  the field instead.

## Colour semantics (trading)

| Meaning | Token |
|---|---|
| Bid, buy, up | `chart.bullish` / `success` |
| Ask, sell, down | `chart.bearish` / `danger` |
| Own order highlight | `list.active.background` with a `list.active.border` left edge |
| New-order flash (250 ms) | bullish at low alpha |
| Trade flash (250 ms) | bearish at low alpha |
| Halted, stale feed | `warning` |
| Kill switch active | `danger` background banner in the status line |
