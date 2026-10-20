# ADR 0051: Desk workspace: a canvas of floating windows, user themes and a command line

- **Status: Accepted Target (2026-10-20).** Nothing here is built yet; the
  implementation log records each slice (U1 to U4 below) as it lands.
  Supersedes ADR 0050 decision 6 *in part*: the `DockArea` workspace, its
  presets and the sidebar/top-bar arrangement. ADR 0050's pane kinds,
  saved named workspaces, one-click Book Trader and Ladder Trader
  behaviour, keyboard-first commands and its realism rules (decision 7)
  stay binding.
- **Date:** 2026-10-20.
- **Depends on:** ADR 0031 (FIX and BNP only), ADR 0035 (latency), ADR 0047
  (shared `bunting-trading-state`), ADR 0050 (the desk app on GPUI and
  `gpui-kit =0.7.1`).

## Context

Owner direction, 2026-10-20 (verbatim in substance): the app "should be a
workspace for users where they can create windows, save views, resize,
organize like they want, just like Bloomberg or Godel Terminal"; "I find
the app ugly … buttons are not styled correctly, divs are uncentered";
use Longbridge's components and the Zed/Delta look "by styling them
correctly and allowing users to set their own theme"; "the same window as
shown on delta's homepage, with the side panel used for our purposes and
the main workspace being a whole workspace where users can spawn windows,
arrange them"; "just like RIT but a lot more modern. RIT is the legacy
software, which we aim to highly modernize". Asked to choose, the owner
picked **free-floating windows** over a tiled grid and **in-app windows**
over native pop-outs (2026-10-20); command naming was left to the
recommendation.

Evidence (2026-10-20):

1. **Current desk** (*observed*, seven owner screenshots from 2026-10-11/12
   and the code at `main@f01cc3d`): a `DockArea` of tabbed panes. The
   problems the owner names are visible. Controls have inconsistent
   heights and padding, labels sit off-centre in buttons and table
   headers, every pane draws its own header row inside the dock tab, the
   sidebar repeats the palette, and empty panes are large grey
   rectangles. The serif display face is used at small sizes, where it
   reads as a rendering error rather than a title.
2. **Godel Terminal** (*observed*, godelterminal.com and its public
   docs): a canvas of free windows opened from a command line
   (`AAPL DES`, `G`, `N`, …), windows grouped by colour links so one
   symbol change drives several windows, layouts saved per account,
   keyboard first.
3. **Bloomberg Terminal** (*observed*, public documentation): mnemonic
   function codes after a security (`<ticker> <yellow key> <function>
   <GO>`), up to four panels plus launchpad windows, colour-linked
   groups.
4. **RIT client** (*observed*, Rotman *RIT User Application Feature
   Guide*): a single MDI window. Modules (Portfolio, Book Trader, Ladder
   Trader, Order Entry, Blotter, Transaction Log, Charting, News, Tenders,
   Trader Info, Assets, Lease, Converters, …) open as floating child
   windows from a menu; the layout saves on exit. Bunting keeps RIT's
   module set as the minimum and modernises the frame.
5. **Delta** (*observed*, delta.dev, Zed Industries; not open source):
   one quiet window, a collapsible left sidebar of navigation and
   context, a large content area, a thin title bar and status line, flat
   surfaces separated by 1px borders rather than shadows. It is a visual
   reference only; nothing is copied.
6. **`gpui-kit 0.7.1`** (*observed* in the crate source):
   - dock (splits, tabs, zoom), `Sidebar`, `TitleBar`, `StatusBar`,
     `Command`, settings pages, `Notification`, virtual `Table`, charts;
   - a `ThemeRegistry` that loads JSON themes and hot-reloads a directory
     (`watch_dir`);
   - every dock pane is a `PanelView` whose `view()` is an `AnyView` and
     whose `dump()` is a serializable `PanelState`, so the existing panes
     can render outside a dock and still be saved and restored through
     the panel registry;
   - **no** free-floating window manager. Separate OS windows are plain
     GPUI `open_window`.
7. **Zed `crates/ui`** is GPL-3.0-or-later: a reference for look and
   density only, never copied (ADR 0050 decision 2).

## Decision

### 1. One frame: title bar, sidebar, canvas, status line

The app is one window laid out like Delta:

- **Title bar.** `gpui-kit` `TitleBar` with the screen name, the command
  line (decision 4), the connection with each venue's measured round trip,
  and the account's P&L and buying power.
- **Sidebar.** Collapsible, resizable, toggled with ⌘/Ctrl+B. Its
  sections:
  - *Screens*: the saved workspaces, with new, rename, duplicate and
    delete;
  - *Windows*: every window on the current screen, including minimised
    ones; click to focus, drag to reorder the z-stack;
  - *Watchlist*: instruments with last, change and a spark of the feed's
    trades;
  - *Run*: run status, logical time, session phase, halts;
  - *Inbox*: news, tenders and notices, with unread counts.
  It replaces the "add pane" list; windows are added from the command
  line, a `+` menu in the sidebar, or keys.
- **Canvas.** The rest of the window (decision 2).
- **Status line.** One row: the kill switch state, the latest notice, and
  key hints.

### 2. The canvas: free-floating windows (owner choice)

Each pane (an ADR 0050 `PaneKind`, unchanged names) lives in a **window**
on the canvas:

- it is moved by its title strip and resized from any edge or corner,
  with a minimum size per kind;
- while it moves or resizes it **snaps** to the canvas edges and to other
  windows' edges within 8 px, and shows the guide it snapped to;
- clicking a window brings it to the front;
- double-clicking the title strip maximises it within the canvas, and a
  second double-click restores it;
- a window can be minimised to the sidebar's Windows list;
- closing a window can be undone with ⌘/Ctrl+Shift+T, which reopens the
  last closed window with its state and geometry;
- **Tile** (⌘/Ctrl+Alt+T) arranges the screen's windows in a grid;
  **Cascade** fans them out;
- ⌘/Ctrl+\` and ⌘/Ctrl+Shift+\` move focus through the windows, and
  ⌘/Ctrl+W closes the focused window;
- when the app window shrinks, windows are clamped so each title strip
  stays reachable; their saved geometry is not rewritten until they move.

Geometry (snap, clamp, tile, cascade, hit-testing edges) is a set of pure
functions, unit-tested without a window. Windows hold the same panel views
the dock held: the canvas renders `PanelView::view()` and saves
`PanelView::dump()`, so panes do not change to move from the dock to the
canvas.

### 3. Screens are saved views

A **screen** is a named canvas: its windows, their kind, geometry,
z-order, minimised state, link group and pane state. Screens persist as
they do today:

- a versioned JSON file in the user's configuration directory, written
  500 ms after the last change;
- at most 16 screens, restored at start with the last one shown.

The format version moves from layout 3 to screen 4. A version-3 file is
renamed to `layouts.v3.json` (kept, never deleted) and the desk starts
from the presets; dock layouts are not migrated.

The presets (Trading, Market Making, Arbitrage) become screen templates
placed relative to the canvas size. Screens can be exported to a file and
imported from one, so an instructor can hand a class a screen.

### 4. A command line with function codes

The command line in the title bar (focused with \` or ⌘/Ctrl+K) opens
windows and runs commands. It keeps the palette's actions (ADR 0050). An
entry is an optional instrument followed by a function. Codes follow
Bloomberg and Godel where a counterpart exists, and RIT module names
are accepted as aliases:

| Code | Opens or does | RIT name accepted |
|---|---|---|
| `BT` | Book Trader | Book Trader |
| `LT` | Ladder Trader | Ladder Trader |
| `OE` | Order Entry | Order Entry |
| `TAPE` (`T`) | Tape | Time & Sales |
| `BL` | Blotter | Blotter |
| `FL` | Fills | Transaction Log |
| `PORT` | Portfolio | Portfolio |
| `TI` | Trader Info | Trader Info |
| `N` | News | News |
| `DEAL` | Tenders and OTC | Tenders |
| `NOT` | Notices | — |
| `G` | Chart (U4) | Charting |
| `THEME` | Theme picker | — |
| `TILE`, `CASC` | Tile, Cascade | — |

An entry such as `BNT BT` opens a Book Trader on instrument BNT. Matching
is fuzzy on codes, names and instrument symbols. Trading commands (cancel
all, kill switch) stay in the line but keep their confirmation (ADR 0050
decision 7).

### 5. Colour link groups

Each window carries an optional link colour (none, or one of five).
Choosing an instrument in a window sets it in every window of the same
colour; an unlinked window keeps its own instrument. This needs a
per-window instrument instead of the desk's one `Desk::instrument` (a
desk-local change; the shared state crate is unchanged). Until it lands
(U3), every window follows the desk's instrument, which is the same as
one link group.

### 6. Windows stay in the app (owner, 2026-10-20)

Windows float inside the app's canvas only. The owner preferred in-app
floating windows to native pop-outs. Second-monitor use (a second app
window on another screen, or pop-outs) is unresolved and not designed
here. Native
pop-out (GPUI `open_window` per pane) is **deferred**: it is not part of
U1 to U4 and is revisited only if owners ask for it.

### 7. A design system and user themes

Every surface is drawn from one set of tokens and a few Bunting
components. Pane code does not set ad-hoc sizes or colours.

- **Sizes.** Control heights are 22, 26 and 30 px (compact, default,
  comfortable). Spacing is on a 4 px grid. Radii are 4 px for controls and
  6 px for windows. Borders are 1 px. The only shadow is on the focused
  window.
- **Components.** `WindowFrame` (the one title strip: icon, title,
  instrument chip, link dot, actions), `Toolbar`, `Field` (label and
  input aligned on one baseline), `Stat` (label over a tabular value),
  `EmptyState` (icon, one line, and the action that fills it), and
  `DataTable` (gpui-kit `Table` with fixed header height,
  right-aligned numbers and centred header labels). These are built on
  `gpui-kit` components and styled with tokens; pane-specific GPUI code
  stays limited to market composition (ADR 0050 decision 2).
- **Type.**
  - Body text is SF Pro through the macOS system font, Inter elsewhere
    (ADR 0050).
  - Numbers use a tabular monospace, IBM Plex Mono, bundled under the
    OFL with provenance.
  - The serif display face stays for the screen name and the empty
    canvas only (the owner's 2026-10-11 direction), at 15 px or larger,
    never inside windows.
- **Themes.**
  - Themes are `gpui-kit` theme JSON. Bunting Dark and Bunting Light ship
    with the app; users add theirs to the `themes` folder in the app's
    configuration directory, which is hot-reloaded through
    `ThemeRegistry::watch_dir`.
  - `THEME` or the settings page picks a theme, the density (control
    height), the UI font size and the number font.
  - Market colours (bid, ask, up, down, own order, flash) are named
    tokens in the theme, so a colour-blind palette is a theme rather
    than a code change.
  - A theme that fails to parse is reported and ignored; it never blanks
    the app.

### 8. Unchanged

- The realism rules (ADR 0050 decision 7) still apply: every order names
  its venue; books are never merged; latency is measured; order entry and
  the kill switch are never queued.
- The app remains a BNP participant client over `bunting-trading-state`;
  no new interface, model or authority.
- Bounded, virtualized tables, and pane names fixed forever.

## Delivery

Each slice ends with **screenshot acceptance**: the coordinator runs
`bunting-desk --local` against a real local venue, captures the screens
the slice changed, judges them against this ADR and records them in the
implementation log. Compilation and headless window tests are not visual
acceptance (desk `AGENTS.md`).

| Slice | Scope |
|---|---|
| **U1** | Frame (title bar, sidebar sections Screens, Windows, Run), the canvas window manager (decision 2), screens v4 with templates (decision 3), `WindowFrame`; panes move from the dock unchanged |
| **U2** | Design system and themes (decision 7): tokens, components, every pane restyled, Light theme, user themes and the picker |
| **U3** | Command line and codes (decision 4), link groups and per-window instrument (decision 5), Watchlist and Inbox sections |
| **U4** | RIT parity beyond ADR 0050: charts (`G`) after a BNP history/reference-data ADR, Transaction Log, Assets and Converters as the run carries them |

## Consequences

- Users arrange the desk freely, keep several screens and theme the app
  without code changes; instructors can hand out screens.
- The desk owns a small window manager. Its geometry is pure and tested;
  focus, drag and z-order need GPUI window tests and screenshots.
- `gpui-kit`'s dock is no longer the layout engine. It stays available
  and could host a "docked" screen later if owners ask, but there is one
  layout model now.
- Saved v3 layouts are not migrated; users start from templates once.
- Per-window instruments move selection state out of `Desk` into
  windows (U3); panes take their instrument from their window.

## Rejected alternatives

- **Tiled grid (Bloomberg four-panel style or the current dock).** It is
  simpler and has no overlap, but it is what the owner rejected as
  RIT-like and rigid. Tile is still one command away (decision 2).
- **Native pop-out windows now.** Deferred (decision 6). The owner
  preferred in-app windows, and per-pane OS windows double the focus,
  save and restore cases before the canvas itself is proven.
- **Bunting-only command names.** Codes users already know from
  Bloomberg and Godel, plus RIT names, cost nothing, and new codes stay
  short and mnemonic.
- **Copying Zed `crates/ui` or Delta assets.** The first is GPL and the
  second is unpublished; both are visual references only.
- **A web canvas.** ADR 0031 and the owner's 2026-10-09 preference rule
  it out.

## Validation

- Unit tests for snap, clamp, tile, cascade, edge hit-testing, and screen
  (de)serialisation including the v3 rename.
- `gpui-kit` `test-support` window tests: open, move, resize, focus,
  close and undo, and switching screens restores geometry and pane state.
- `tools/check_desk.sh` passes; the root gates are unaffected.
- Screenshot acceptance per slice, as above.

## References

- [ADR 0050](0050-desktop-trading-app-on-gpui.md),
  [ADR 0047](0047-trading-terminal-on-shared-trading-state.md)
- Godel Terminal: <https://godelterminal.com/>
- Delta: <https://delta.dev/>
- RIT User Application Feature Guide (Rotman FRTL):
  <https://rotmanfrtl.github.io/RIT%20User%20Application%20(RIT%20Client)%20Feature%20Guide.pdf>
- `gpui-kit` 0.7.1: <https://crates.io/crates/gpui-kit>
