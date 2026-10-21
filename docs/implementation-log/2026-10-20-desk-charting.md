# Desk: universal Charting window and derived series (2026-10-20)

Charting + Portfolio derivatives of
[`docs/specs/rit-desk-window-parity.md`](../specs/rit-desk-window-parity.md),
superseded in part by the owner's chart-redesign decision the same
day. The work was done by one Muse Code agent
(`muse-wonder-full-1.0`) in the `muse/charts` worktree. The
coordinator reviewed and screenshotted the first chart
(`f9c7107`, approved as composed) and the custom `PriceChart`
(`55cfc3c`, approved: "a real trading chart"); one label-collision
nit from that review is fixed in `ac7e427`. Commits from
`ac7e427` on have compilation and headless-test evidence only.

## Commits

- `b228b19` trading state: per-listing 1 s bars (bounded,
  `MAX_FINE_BARS` 14,400) folded into 1 s/5 s/15 s/1 m/5 m bars,
  market VWAP and cumulative volume from direct feeds only;
  own-fill VWAP cost, realized/unrealized P&L and once-per-second
  total-P&L samples. All derived and labelled so. Feed trades are
  stamped by a series clock (latest stamped run time advanced at
  wall rate) because **BNP v1 feed updates carry no run time** —
  a protocol gap that belongs in the parity spec's list.
- `9db95ac` units fix from coordinator review: tick-lots and
  minor-unit fees stay apart; money = tick-lots × multiplier −
  fees at the API boundary. `Model::multiplier` returns
  `(1, false)` until BNP carries reference data; `PnlTotals` and
  `PnlSample` say when contract size 1 is assumed. Test injects
  multiplier 10 through a test-only setter.
- `f9c7107` first Charting window from gpui-kit 0.7.1 chart
  components (owner-approved direction at the time; visually
  rejected later the same day for the missing price axis).
- `7df5be8` merge of `integrate/desk-u1-u2` (b22b73a): Chart kept
  alongside Transactions, `OpenChart` alongside
  `OpenTransactions`; `tools/check_desk.sh` green on the merged
  tree before committing.
- `e352a11` saved windows carry a pane's own JSON state:
  `SavedWindow.state` (serde-default, v4 files still load),
  `Canvas::dump(cx)` records dock `dump` JSON,
  `PaneKind::build_with_state` restores Charting's `SavedChart`.
- `55cfc3c` `PriceChart`: one custom `Plot` on Longbridge's
  primitives (`ScaleBand`/`ScaleLinear`, `Grid`, `Line` shape,
  `PlotLabel`) — price/volume/RSI panes on one x scale and
  crosshair, right price axis with nice ticks, dashed last-price
  tag, run-time axis, OHLCV legend, MA(20)/EMA(20), RSI(14) with
  dashed 30/70, comparison mode (percent change) for ≥2 series.
  The panel owns `ViewRange` (wheel zoom around the cursor, drag
  pan, double-click/Follow-latest refollow); default 80 bars at
  5 s with density auto-pick. Pure helpers in `chart/range.rs`
  are unit-tested.
- `ac7e427` baseline P&L chart (`chart/pnl.rs`): 1.5 px line
  coloured by sign, fill to the zero line via the `Area` shape's
  `y0` split at interpolated zero crossings (unit-tested),
  money axis, latest tag, component tooltip crosshair. Same
  component in Portfolio's collapsible P&L section and in the
  new P&L Chart window kind (`"pnl"`, `OpenPnl`). The Chart
  window loses its Security/P&L toggle for one toolbar row.
  Axis labels stand down under the last-price/crosshair/time
  tags (coordinator screenshot nit on `55cfc3c`).
- `914306f` headless tests (`tests/chart.rs`): scripted trades
  become bars (stamped zero-quantity fills advance the series
  clock), 15 s folding, wheel zoom anchored mid-plot, drag pan,
  double-click and button refollow, comparison via the + menu,
  Portfolio/P&L-window figures from scripted fills, and a
  saved-state round trip through `Canvas::dump`/`set_screen`.
  The Portfolio section measures its region (container query)
  and sizes the chart from it, so the short template window
  yields chart height to the holdings table.

## Checks (commands actually run)

- `cargo test --locked -p bunting-trading-state` — 28 passed.
- `tools/check_desk.sh` before every desk commit (metadata,
  fmt check, clippy `--all-targets -D warnings`, tests) —
  green at 109 tests (55 lib incl. range/pnl/indicator units,
  9 appearance, 5 chart, 16 panes, 24 workspace) with
  `CARGO_TARGET_DIR=$TMPDIR/desk-target-ch`.
- Root: `cargo metadata --locked --no-deps` ok;
  `cargo fmt --all --check` ok; `cargo clippy --locked
  --workspace --all-targets -- -D warnings` clean;
  `cargo tree -p bunting-engine` shows no `orderbook-rs`;
  `cargo check --locked --workspace --target
  wasm32-unknown-unknown` ok; `git diff --check` ok.
- `cargo test --locked --workspace --no-fail-fast`: 80 suites
  green, 314 passed, 3 failures — all in `bunting-server` and
  not chart-related:
  - `tcp_rtt::tests::accepted_loopback_socket_reports_a_kernel_rtt`
    fails identically on the untouched base `b22b73a` in this
    sandbox (no kernel RTT in the network namespace);
  - `built_in_agents` and `agent_restart` live-venue tests timed
    out under full-workspace parallel load ("book never
    matched") and pass in isolation on both this tree and
    `b22b73a` (1.1 s each). Recorded as environment/load, not
    regressions; CI on an uncontended runner is the check that
    can settle them.

## Decisions and seams worth knowing

- 0.7.1 limits that shaped this: `CandlestickChart` has no
  y-axis/y-domain/reference line; `LineChart` is single-series
  and point-scaled; `AreaChart` fills to the plot bottom
  (`.y0(height)`), which misreads losses — hence two custom
  `Plot`s on the library's own primitives, structure following
  its `candlestick_chart.rs`/`line_chart.rs` (Apache-2.0,
  provenance noted in `price_chart.rs`).
- Series colours are fixed-order theme tokens (chart_2, yellow,
  magenta, cyan; the theme's chart.1–5 are a one-hue ramp, not
  categorical). The dataviz palette-validator script was not
  present in the agent sandbox, so the palette is unvalidated
  by that script.
- Indicator warm-ups average what exists (a band scale cannot
  leave leading bars blank and stay aligned); documented in
  `chart/mod.rs`. Bar-size auto-pick counts *seconds that
  traded* (finest bars do not retain a trade count).
- The pane icon is `IconName::Frame` (Chart) and
  `GalleryVerticalEnd` (P&L): the component icon subset has no
  candlestick/line icon in 0.7.1.
- In the short template Portfolio window the P&L chart is
  omitted below 24 px of spare height (stats remain); a
  full-size Portfolio window shows the 140 px chart.

## What remains

- Coordinator screenshot acceptance of `ac7e427`/`914306f`
  (Portfolio section at default sizes, P&L Chart window, both
  themes, comparison mode).
- The parity spec's BNP gaps still gate the real figures:
  history/backfill, instrument reference data (multiplier,
  asset type), and venue valuation (NLV/margin). Feed run-time
  stamps would retire the series clock.
- Portfolio rows, Trader Info P&L section and the Transaction
  Log's running P&L consume the same trading-state APIs from
  the integration branch; nothing chart-side is left for them.

## Coordinator screenshot review round (same day, after `914306f`)

Live screenshots with two desks trading against the local venue
approved `55cfc3c` and sent back eight fixes, landed as:

- `306ad59` (model) the P&L series opens with a zero sample at
  run time 0 (the first fill's figure no longer reads back to
  the start), and samples mark from the last two-sided touch:
  an agent re-quoting left a one-sided book for an instant and
  sampling that instant (live mark falling back to the last
  trade) spiked the series. Also fixes a clippy lint in the
  merged Order Entry preselect.
- `655a301` integral ranges never step below a whole tick in
  `nice_ticks` (98/99/100, not 97.6/98.4); price domain pads at
  least a quarter-range/one step each side and never shows
  under a four-tick range; volume gets 15% headroom; candle
  bodies cap at 14 px with ~30% gap past a 20 px step.
- `74a8543` P&L x is true run time with whole-step run-clock
  labels, and the line/fill are step-after (`stepped_runs`:
  sign changes at the step's vertical through zero, never a
  diagonal — P&L only changes at events, so the flat zero
  before the first fill no longer ramps). Latest-value tags on
  both charts use the fills' own contrasting tokens
  (success/danger foreground).
- This commit: Portfolio's P&L region scrolls when the window
  is too short for stats + chart + a usable table (chart yields
  first, table keeps at least 96 px), and the Trading template
  gives Portfolio the full-height bottom-right column (Fills
  and Trader Info stack in the middle column).

`tools/check_desk.sh` green before each (113 tests at the end:
58 lib incl. the new unit tests, 5 chart window, 17 panes,
24 workspace, 9 appearance). One appearance test flaked once
under parallel load and passed isolated and on re-run.

## Review round 3 (2026-10-20, after fast-forwarding `integrate/desk-u1-u2` = `adf35a3`)

- `b1d5a1b` — P&L/price time-axis labels cannot overlap at any
  span: `range::time_axis_labels` (step chosen for label width,
  greedy drop after clamping) and `range::keep_clear`; property
  test over spans 5 s..24 h × plot/label widths.
- `91efdce` (+`67623e0` lint fixes) — Book Trader venue cards:
  place is priority 0 like ahead/broker, and the panel stacks
  venue cards when every card cannot keep qty+price, instead of
  clipping the far venue in a sideways scroll. Test at template
  size: every card and its best-ask row inside the window.
- `b1a741a` — `Model::average_cost` (exact tick-lots/lots) and
  `format::average_price` (one decimal, two when needed) in the
  Portfolio cost column, Book Trader Cost header and Ladder.
- Charting opens single-series (venue-1, candles); adding the
  same instrument again overlays its next venue as a line on
  the price axis; percent comparison only across instruments.

Checks: `tools/check_desk.sh` exit 0 before each commit (118
tests: 61 lib, 9, 6 chart, 17 panes, 25 workspace);
`cargo test --locked -p bunting-trading-state` 44 passed and
clippy clean for the average-cost commit. Visual acceptance of
this round is the coordinator's screenshots.
