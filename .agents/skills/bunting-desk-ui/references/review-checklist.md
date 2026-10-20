# Desk UI review checklist

Run every item against the change before reporting it done. Also run the
gpui-kit Design Guides' own "Design review checklist". Answer each item
with yes, no or n/a, and add a sentence wherever the answer is no.

## Alignment and sizing (the owner's top complaint)

- [ ] Every single-line row (title strips, toolbars, sidebar rows, table
      headers, status line) uses `items_center()` with an explicit
      height from the spec.
- [ ] Button labels and icons are vertically centred, and all buttons in
      one region share one size.
- [ ] Table header labels align with their column's cells (numbers
      right-aligned in both).
- [ ] No content touches a border; window bodies have at least 8 px inner
      padding.
- [ ] Every gap and padding is on the 4 px grid, apart from the
      documented exceptions (1 px borders, 6 px gutter and radius, 2 px
      chip padding).

## Tokens and type

- [ ] No colour literals in view code; every colour comes from
      `cx.theme()`.
- [ ] The serif appears only for the title-bar screen name and the empty
      canvas.
- [ ] All numbers use tabular figures; prices are coloured by side and
      quantities are not.
- [ ] Font sizes come only from the spec's set (11, 12, 13, 15, 22).

## Windows and canvas

- [ ] Windows keep a 6 px gutter from each other when snapped and from
      the canvas edges; maximise fills the inset canvas.
- [ ] Only the focused window has a shadow and the ring border.
- [ ] Title-strip actions appear on hover or focus without shifting the
      layout.
- [ ] Dragging and resizing show snap guides; minimum sizes hold; and
      when the app window shrinks, each title strip can still be
      grabbed.
- [ ] No pane draws a second title.

## States

- [ ] Hover, focus, active, disabled and loading are each visibly
      distinct and consistent.
- [ ] Empty panes show an icon, one line and an action, never a large
      blank area or a paragraph of explanation.
- [ ] Disconnected, stale feed, halted and kill-switch-active states are
      visible, using `warning` or `danger` as appropriate.

## Keyboard

- [ ] Every new command is an action in `src/actions.rs`, bound where it
      has a key, listed in the palette, and runs the same code as its
      button.
- [ ] Escape closes the topmost overlay and returns focus to where it
      was.

## Realism (ADR 0050 decision 7)

- [ ] Every order path names its venue; books are not merged; latency
      shown is measured; the kill switch is confirmed and is never
      queued.

## Report

- [ ] List the screens and states changed, and every visual choice you
      were unsure about, for the coordinator's screenshot check.
