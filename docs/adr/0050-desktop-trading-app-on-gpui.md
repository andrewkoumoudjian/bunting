# ADR 0050: Desktop trading app on GPUI and gpui-component

- Status: Accepted (2026-10-10). The owner asked on 2026-10-10, after the TUI
  rebuild, to "reimplement the app using rust gpui components from the zed
  library and from the long bridge components library and refer to all
  starred gpui projects as reference". Phase 1 (this ADR and the
  `apps/bunting-desk` skeleton) landed in slice 31; the trading views build
  on `packages/bunting-trading-state` (ADR 0047, slice 30).
- Date: 2026-10-10
- Depends on: ADR 0031 (FIX and BNP only), ADR 0035 (latency), ADR 0036
  (market data), ADR 0040 (BNP v1), ADR 0044 (native `bunting server`), and
  ADR 0047 (shared `bunting-trading-state`, rendered by the TUI).
- Reserves: ADRs 0051 and 0052 for follow-up desktop decisions.

## Context

Andrew rejected browser UIs: the participant client is an application that
connects to a hosted venue (owner preferences, 2026-10-09). The TUI is being
rebuilt as a thin client over `packages/bunting-client`, and the coordinator
asked it to keep its session, books, tape, orders, positions and latency
state in a crate with no rendering. A desktop app should render that same
state rather than grow a second model.

Survey (2026-10-10; every item *observed* from crates.io, the repositories or
Andrew's public GitHub stars page unless marked):

1. **Zed's GPUI.** `gpui` on crates.io is `0.2.2` (2025-10-22), far behind Zed
   `main`. Longbridge publishes dated snapshots of Zed's GPUI as `gpui-pre`
   (`0.3.8`, 2026-10-05) with `gpui-pre-platform`, `-macros`, `-web` and
   friends. All Apache-2.0. Zed's `crates/ui` is GPL-3.0-or-later and is not
   a dependency or a copy source.
2. **Longbridge components.** The repository `longbridge/gpui-component` is
   now `longbridge/gpui-kit`. `gpui-kit 0.7.1` (2026-10-05, Apache-2.0)
   re-exports `gpui` (as `gpui-pre =0.3.8`), `gpui-base`, `gpui-component`
   and the Lucide asset set. Features: `component` and `assets` by default;
   `decimal`; `test-support` for headless window tests. Platforms: macOS
   (Metal), Windows, Linux X11 and Wayland, and `wasm32` through
   `gpui-pre-web`. Components that fit a trading screen: virtualized `Table`
   (fixed and resizable columns, sorting, hundreds of thousands of rows),
   virtual `List`, charts (line, bar, area, candlestick), `DockArea` with
   tab panels, resizable splits and serializable layout, `Input` and
   `NumberInput`, `Button`, `Select`, `Switch`, `Dialog`, `Notification`,
   `TitleBar`, `StatusBar`, `Tooltip`, theme tokens.
3. **Toolchain.** The `gpui-kit 0.7.1` dependency graph needs Rust 1.92 or
   newer (`oo7` declares 1.92; `serde-saphyr`, `smol_str` and `uuid` 1.89).
   The Bunting workspace is pinned to Rust 1.88 (`rust-toolchain.toml`).
4. **Andrew's starred GPUI projects** (his stars filtered by "gpui", plus all
   86 starred Rust repositories checked by hand; the GitHub API path for
   stars is blocked in this environment, the public page is not):

   | Project | License | What Bunting takes |
   |---|---|---|
   | `longbridge/gpui-kit` | Apache-2.0 | the dependency; `story` gallery and `system_monitor` (live charts) as usage examples |
   | `tobi/disktree` | MIT | the build shape: its own Cargo workspace with `rust-toolchain.toml` 1.97, `gpui-kit` pinned exactly from crates.io, a core crate with no GPUI under an app crate, and `gpui-kit` `test-support` window tests that draw real frames and press real keys in CI |
   | `zeronsh/zeron` | MIT | a shipping desktop app over a local or remote engine process: the app is a client, the engine runs separately and the app supervises it; release packaging per platform |
   | `futureboard/futureboard-studio` | MIT | a dense, real-time workspace (mixer meters at frame rate) driven by a separate engine thread; shows that a vendored GPUI fork is costly, which Bunting avoids |
   | `Glass-HQ/gpui` | not stated on the page | a standalone GPUI distribution; reference only, not a dependency, until its license is recorded |

5. **Stale branch `feat/gpui-market-terminal`** (reference only, never a
   base). It built `apps/bunting-terminal` as a FIX client with git
   dependencies on Zed and `gpui-component@88f102d`, its own workspace and
   lockfile, Rust 1.95, a `DockArea` shell with trading, research and
   competition presets, and a local-server helper that launched the
   retired WASIX module. Its dock layout and component inventory carry over;
   its FIX client, WASIX launcher and git-pinned dependencies do not.
6. **What BNP v1 carries today.** Direct venue feeds give price levels and
   trades (`Subscribe`, `MarketSnapshot`, `MarketUpdate`); orders, cancels,
   kill switch, open orders, account, fills and `Ping`/`Pong` round trips
   exist. Order-by-order (L3) feeds, broker identifiers and the consolidated
   tape exist on the venue's FIX feeds (slices 21–24) but not on BNP v1.

## Decision

1. **`apps/bunting-desk`** is the desktop trading app: one native GPUI
   window, binary `bunting-desk`. It is a participant client only, like the
   TUI. It never links `bunting-engine` and never holds market authority.
2. **Components come from `gpui-kit = "=0.7.1"` from crates.io**, exactly
   pinned, `default-features = false` with `component` and `assets`. That is
   Zed's GPUI (the `gpui-pre` snapshot) plus Longbridge's components, with
   no git dependencies. Bunting writes GPUI code only for market-specific
   composition. Any source adapted from a reference is Apache-2.0 or MIT
   and recorded with repository, commit and path; GPL and AGPL sources
   (Zed `crates/ui`, Coop, Zorite, Rox) are not copied.
3. **The app is its own Cargo workspace**, listed in the root workspace's
   `exclude`, with its own `Cargo.lock` and `rust-toolchain.toml` pinned to
   Rust 1.97. It depends on first-party packages (`bunting-client`, and the
   `bunting-trading-state`) by path. This is the one sanctioned
   exception to "the root owns the single `Cargo.lock`": the GPUI graph
   needs a newer compiler than the venue, pulls about 850 crates and system
   graphics libraries, and cannot run on `wasm32-unknown-unknown` the way
   the root gates require. The seven root gates therefore do not build it.
   Its own gates are `cargo fmt --check`, `cargo clippy --locked
   --all-targets -D warnings` and `cargo test --locked` run in
   `apps/bunting-desk` (`tools/check_desk.sh`), plus `cargo metadata
   --locked` and `git diff --check`, and a CI job runs them on Linux,
   macOS and Windows.
4. **Layering.** `bunting-client` owns the BNP connection. The shared state
   crate, `bunting-trading-state` (ADR 0047), owns session state, books, tape, orders,
   fills, positions and latency, and is the only model. `bunting-desk` owns
   GPUI views, layout and input. The shared crate's `Session` keeps the
   `Client` connected on its own thread and queues venue events (bounded);
   the app's `Desk` entity drains them every 4 ms on GPUI's foreground
   executor, stamping each with the instant the session received it, and
   redraws at most every 16 ms. Orders, cancels and the kill switch are
   written on the UI thread the moment the user acts, never queued behind
   the drain. No async runtime is added.
5. **Hosted first, local by spawning a venue.** The app connects to a hosted
   venue with the participant's certificate, CA and key. Local play starts a
   child `bunting server <config>` on loopback and connects over BNP like any
   other venue, reusing the TUI's local-venue launcher and certificate
   provisioning when the shared crate has them. The app stops only a child
   it started.
6. **Layout.** A `DockArea` workspace of panes modelled on RIT's modules
   (Book Trader, Order Entry, Tape, Blotter, Fills, Portfolio, Trader
   Info, Notices), each openable any number of times from the sidebar,
   then moved, split, tabbed and resized. Named workspaces start from a
   preset (Trading, Market Making, Arbitrage); every workspace's layout is
   saved on each change to one versioned JSON file in the user's
   configuration directory and restored at the next start (slice 47). A
   left sidebar (workspaces, add pane, watchlist), a top bar (workspace,
   save and reset, connection and per-venue round trips) and a status bar
   frame the dock. Type: SF Pro through the macOS system font, Inter
   elsewhere, Source Serif 4 Display for titles, tabular figures for
   numbers; Inter and Source Serif 4 are bundled under the SIL OFL 1.1
   (`apps/bunting-desk/assets/fonts/README.md`). A dark theme is the
   default (`apps/bunting-desk/assets/theme.json`). Every command is a
   GPUI action (`src/actions.rs`) bound to keys and listed in a command
   palette (gpui-kit `Command`, ⌘/Ctrl+Shift+P or ⌘/Ctrl+K). The Book
   Trader follows RIT: bids left and asks right, best on top; new orders
   light green and traded ones red for 250 ms; your own orders are
   highlighted from the venue's order-by-order reference (slice 44); and
   one-click trading, off until switched on, with V and O: left-click a
   bid buys V at its price + O (an ask sells at − O), right-click takes
   the clicked side at market, shift+right-click sweeps through the price
   with an immediate-or-cancel limit, alt+click cancels yours at a price
   (slice 48). Each one-click order goes to the venue whose book was
   clicked. The Ladder Trader shows one venue, which the trader picks, on
   a fixed price axis (Centre or Space recentres): your bids, bid size,
   price, ask size, your asks; clicking a size sends a limit for V at that
   price, clicking your quantity cancels it. The ticket takes an iceberg's
   shown quantity (slice 49). News lists headlines newest first with the
   chosen story beside them; Tenders & OTC answers tenders and proposes,
   counters, accepts, rejects and breaks negotiations, each applied at the
   hub through the sequencer like any order; the Book Trader says when an
   instrument is halted on every venue and the status bar shows the run's
   status. The shared state keeps them as the venue last committed them,
   bounded (slice 50).
   Visual direction (owner, 2026-10-10: "zed and delta has very good
   ui"): Zed's quiet, dense workspace chrome (one title bar, tabbed docks,
   a status bar, a command palette, keyboard first). *Corrected
   2026-10-11:* the owner clarified that Delta is "delta (project from
   zed)" (*inferred*: Zed Industries' DeltaDB), not the Delta portfolio
   app this decision first assumed, and floated it as a shared team status
   database; whether and how to use it is being evaluated with the RIT
   feature-parity research, not decided here. Further owner direction
   (2026-10-11): a visually pleasing, configurable workspace with Zed as
   the reference (dockable, resizable panes, saved layouts); full RIT
   feature parity as the minimum, better, lighter and faster than RIT;
   serif fonts for titles and SF Pro for body text (SF Pro through the
   macOS system font only, since Apple's licence covers Apple platforms;
   an open-licensed fallback elsewhere). The owner first asked for mockup
   images, then the same day for the implementation itself; the shell
   above is its first step, built against the RIT parity spec
   (`/mnt/project-files/bunting/rit-parity/2026-10-11-rit-parity-desk-spec.md`
   in the project files) and the owner's two dark-mode examples. All references are design
   references only; no source or assets are copied.
7. **Realism rules for the UI** (ADR 0035):
   - every order names its venue; there is no default router and no
     "best venue" button that hides the choice;
   - the app shows each venue's book as that venue's feed reported it, with
     the feed's own staleness; it never merges venue books into a book
     no venue published, and the consolidated tape is shown as the
     processor at the hub published it, late by its own distance;
   - latency shown is measured (`Ping`/`Pong` round trip and the logical
     time on each stamp); the app never smooths, delays, batches or
     compensates timing, and order entry goes straight to the I/O thread;
   - the kill switch is one action with a confirmation and is never queued
     behind other UI work.
8. **Bounded views.** Tables are virtualized; the tape, fills and logs are
   ring buffers with fixed capacity shared with the TUI state crate.
9. **BNP gaps go to the protocol thread.** Order-by-order feeds, broker
   identifiers, the consolidated tape and published venue latency over BNP
   are requested through the coordinator. The desk app shows what BNP
   carries and labels the rest as not yet available; it never falls back to
   FIX feeds or a side channel.

## Consequences

- Participants get a desktop trading screen on Linux, macOS and Windows that
  talks to the same venue and through the same latency path as FIX and the
  TUI.
- The TUI and the desktop app cannot drift on market semantics: both render
  one state crate. Changes to it are shared API and are coordinated.
- Contributors need Rust 1.97 (rustup fetches it from the app's toolchain
  file) and, on Linux, X11/Wayland, xkbcommon, Vulkan, fontconfig and
  freetype development packages to build the app. The venue and packages
  keep Rust 1.88 and are unaffected.
- `gpui-kit` is pre-1.0; upgrades are deliberate, one pinned version at a
  time, with the app's tests and gates.
- The desk app's lockfile can resolve different versions of shared
  dependencies than the root's. Path dependencies compile from the same
  source, so behavior follows the source; only third-party versions differ.

## Rejected alternatives

- **Raise the whole workspace to Rust 1.97 and add the app as a member.**
  Moves the MSRV for the engine and every package for one client, puts the
  GPUI graph and its system libraries into every root gate and runner, and
  still needs a `wasm32` exclusion. Rejected; revisit if the root toolchain
  moves past 1.92 for its own reasons.
- **Git dependencies on Zed `main` and `longbridge/gpui-component`** (the
  stale branch's approach). Pulls the whole Zed source tree, needs network
  git in CI and gives no released version to record. `gpui-kit` from
  crates.io pins the same code by version.
- **Vendoring GPUI** (Futureboard's approach). A fork Bunting would have to
  maintain; no Bunting need justifies it.
- **`gpui 0.2.2` from crates.io with no component library.** A year old and
  without tables, docks or charts; the owner asked for Longbridge's
  components.
- **A web or Tauri UI.** The owner rejected browser UIs, and ADR 0031 allows
  no browser command surface.
- **Embedding the engine for local play.** The app would be a second venue;
  ADR 0022 allows one market authority.

## Validation

- `tools/check_desk.sh` (fmt, clippy `-D warnings`, tests, locked metadata)
  passes in `apps/bunting-desk`, and the CI desk job runs it on Linux, macOS
  and Windows.
- Headless view tests with `gpui-kit`'s `test-support` render the workspace
  and drive order entry, cancel and kill switch against scripted server
  messages.
- End to end (slice 32, by hand): `bunting-desk --local` under Xvfb with
  Mesa's software Vulkan started a real `bunting server`, showed its live
  book and trades, and a buy picked from the book was accepted and filled
  (order round trip shown, account debited). An automated end-to-end test
  is still to come.
- The seven root gates still pass, and `cargo tree` of the root workspace
  contains no `gpui` crate.

## Operational impact

The app ships as its own release archive per platform,
`bunting-desk-<tag>-<target>.tar.gz`, built by the `desk` job of
`.github/workflows/release.yml` for the venue's five release targets (the
venue archive does not carry the GPU stack). CI gains one desk job per OS,
which also runs when `bunting-trading-state` changes. The root workspace, its lockfile and the
venue's release are unchanged.

## Security impact

The app holds a participant certificate and private key; it reads them from
files the user selects and never writes or displays the key. It accepts no
inbound connections. A locally spawned venue listens on loopback only. The
app adds no participant interface: it speaks BNP to the venue like any
client.

## References

- [ADR 0031](0031-proposed-bunting-native-client-protocol.md),
  [ADR 0035](0035-latency.md), [ADR 0036](0036-public-market-data-feeds.md),
  [ADR 0040](0040-bunting-native-protocol-v1.md),
  [ADR 0044](0044-native-server-binary-host.md)
- [BNP v1 contract](../specs/bnp-v1.md)
- `gpui-kit` 0.7.1: <https://crates.io/crates/gpui-kit>,
  <https://github.com/longbridge/gpui-kit>
- Zed GPUI: <https://github.com/zed-industries/zed/tree/main/crates/gpui>
- References from Andrew's stars: <https://github.com/tobi/disktree>,
  <https://github.com/zeronsh/zeron>,
  <https://github.com/futureboard/futureboard-studio>,
  <https://github.com/Glass-HQ/gpui>
- Stale branch `feat/gpui-market-terminal` at `767863e` (reference only):
  `apps/bunting-terminal`, `docs/gpui-terminal-reference-inventory.md`
