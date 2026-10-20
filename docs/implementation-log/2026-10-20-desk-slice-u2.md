# Desk slice U2: themes, appearance and restyled panes (2026-10-20)

ADR 0051 decision 7. The work was done by three Muse Code agents
(`muse-wonder-full-1.0`, xhigh), each in its own worktree. The coordinator
(Claude) reviewed every commit, merged them, wrote the integration commit,
and accepted the result by screenshots of the live app.

## Commits

### U2a, themes and appearance (`muse/desk-u2a`)

- `382d52f`: bundled **Bunting Dark** and **Bunting Light** themes in
  `assets/themes/bunting.json`.
  - Both themes carry the same 99 tokens.
  - Text meets 4.5:1 contrast.
  - Control boundaries (`input.border`, switch, ring) meet 3:1.
  - Separators stay subtle (`#262a31` dark, `#dcdfe4` light), after
    coordinator review.
- `c819a0a`: `appearance.rs`.
  - A versioned `appearance.json` holds theme, UI font size (12–16) and
    density.
  - User themes load from the config `themes/` folder and hot-reload
    with `ThemeRegistry::watch_dir`. A bad file is skipped.
  - Theme, font-size and density actions, plus a palette group.
- `d4a0bf8`, `d935389`: appearance tests.

### U2b, restyled panes (`muse/desk-u2b`)

- `fcad604`: `ui.rs`, the shared pane components.
  - Pane body, toolbar, field, stat, empty state and segments.
  - Tables whose columns fit the measured width. Each column has a
    minimum, a weight and a priority; low-priority columns drop whole
    rather than clip.
- `06e4065`: Order Entry as an aligned field column.
  - A side-coloured submit button that names the order and its venue.
- `77e1995`: Book Trader venue cards and the Ladder.
- `179fee4`: the tables (Tape, Blotter, Fills, Portfolio), with empty
  states.
- `22b083d`: Trader Info, News (sans headlines), Deals and Notices.
- `903efa5`: legacy row helpers removed.
- `75460ab`: pane tests.

### Integration (coordinator)

- `0f29777`: three changes.
  - **Hooks:** the appearance actions and palette group are wired into
    the workspace.
  - **Sizing:** the window rem size follows the UI font size (16 px rems
    at 13 px), every desk size is a rem, and density sets row, toolbar,
    title-strip, sidebar-row and control heights from
    `appearance::metrics`.
  - **Test isolation:** window tests keep appearance settings and user
    themes in a private directory (`init_in`). Before this, every test
    run would have read, and created files in, the developer's real
    configuration directory.

### Fix round 1 (`muse/desk-u2b`, from the coordinator's screenshots)

- `0cbcf09`:
  - **Order Entry:** a sticky footer. The submit button, Cancel all and
    the Kill switch are always visible, and the fields scroll above
    them. At normal window sizes they had been cut off and unreachable.
  - **Panes:** bodies scroll instead of clipping.
  - **Ladder:** the V input fits 5 digits.
  - **Trader Info:** a quiet refresh icon.
  - **Notices:** text truncates to one line, with a tooltip.
  - **Book Trader:** book sides fit the measured card width, dropping
    the broker column, then "ahead", instead of clipping.

## Checks

`CARGO_TARGET_DIR=~/.cache/bunting-agents-tmp/desk-target
tools/check_desk.sh` passes on `integrate/desk-u1-u2` at `15a529b`:

- `cargo metadata --locked`;
- `fmt --check`;
- `clippy --locked --all-targets -D warnings`;
- `cargo test --locked`: 36 library, 9 appearance, 7 pane and 24
  workspace tests, 0 failed.

`git diff --check` is clean.

The root workspace's seven gates were not run: `git diff main..integrate
-- . ':!apps/bunting-desk' ':!docs'` is empty, and the desk is excluded
from the root workspace (ADR 0050).

## Visual acceptance

The real `bunting-desk` ran against a local `bunting server` on a headless
Hyprland output (2400×1500 at scale 1.5), with an isolated
`XDG_CONFIG_HOME`. Screens checked:

- Trading, Market Making and Arbitrage at 1600×1000 logical;
- Trading at 1280×720;
- Bunting Light;
- compact at 12 px and comfortable at 15 px.

The before-and-after page with these screenshots is a private artifact
shared with the owner.

## What remains

The working list is `docs/plans/2026-10-20-desk-backlog.md`, and the
acceptance contract is `docs/specs/rit-desk-window-parity.md`:

- RIT window parity (Portfolio columns, Blotter tabs, Transaction Log,
  Order Entry "+", position and cost headers, copy to clipboard);
- universal charts (in progress on `muse/charts`);
- the multi-participant local market (in progress on
  `muse/local-market`);
- the multi-identity practicality test;
- the BNP reference-data, history and valuation ADR;
- U3 (command line and link groups).
