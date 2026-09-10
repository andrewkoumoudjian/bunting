# Bunting Canonical GPUI RIT Terminal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert `apps/bunting-terminal` into the canonical RIT-style Bunting desktop workstation using `gpui-kit` as the single GPUI/component/chart dependency and only authoritative server-backed market/account data.

**Architecture:** Keep the existing single native window and dock model, but replace the direct Zed/old component dependency graph with `gpui-kit = =0.6.1`, keep Bunting-specific composition in focused panel/view-model files, and make the Trading preset the deliberate RIT-like default. GPUI views consume `bunting-client` projections; charts use GPUI Kit chart primitives exclusively, and disconnected/missing authority is represented explicitly rather than filled from local guesses.

**Tech Stack:** Rust 1.95 desktop workspace, `gpui-kit` 0.6.1, `bunting-client`, GPUI Kit DockArea/Table/Input/Button/chart/test-support, macOS ARM64 packaging.

**Spec:** `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`

## Global Constraints

- `gpui-kit` is the only direct GPUI/component/platform/assets dependency of `apps/bunting-terminal`.
- Every graphical chart uses `gpui_kit::component::chart` or `gpui_kit::component::plot`; no custom chart painter exists in the app.
- Canonical OHLC comes only from authoritative market-history projection.
- The terminal contains no local matcher, account ledger, fill fallback, risk authority, or command acceptance logic.
- Default Trading geometry is RIT-like: top market/run strip, left market pane, center chart/info, right order/account/risk, bottom activity.
- Docking/resizing remains user-customizable after the deliberate default is established.
- UI Rust source uses semantic GPUI Kit theme tokens rather than raw product colors.
- Market/order/tender/event rows use domain IDs as stable identities, not list indexes.

---

### Task 1: Replace direct GPUI/component dependencies with GPUI Kit 0.6.1

**Files:**
- Modify: `apps/bunting-terminal/Cargo.toml`
- Modify: `apps/bunting-terminal/Cargo.lock`
- Modify: `apps/bunting-terminal/src/main.rs`
- Modify: imports throughout `apps/bunting-terminal/src/`
- Modify: `apps/bunting-terminal/THIRD_PARTY_NOTICES.md`
- Modify: `docs/gpui-terminal-reference-inventory.md`
- Modify: `.github/workflows/gpui-terminal.yml`

**Interfaces:**
- Manifest dependency:

```toml
[dependencies]
bunting-client = { path = "../../packages/bunting-client" }
gpui-kit = "=0.6.1"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "sync", "time"] }

[dev-dependencies]
gpui-kit = { version = "=0.6.1", features = ["test-support"] }
```

- [ ] **Step 1: Change only the manifest and run a deliberately red check**

Delete direct dependencies `gpui`, `gpui_platform`, `gpui-component`, and `gpui-component-assets`; add `gpui-kit`. Do not update source imports yet.

Run:

```bash
cd apps/bunting-terminal
cargo check
```

Expected: FAIL on unresolved direct GPUI/component imports.

- [ ] **Step 2: Migrate imports to GPUI Kit re-exports**

Use:

```rust
use gpui_kit::*;
use gpui_kit::component::{...};
```

and exact component module paths from GPUI Kit 0.6.1. Replace `gpui_component::...` with `gpui_kit::component::...`; replace direct `gpui::...` types with Kit re-exports where exposed. Do not add a second direct GPUI dependency to work around an import.

- [ ] **Step 3: Initialize Kit using its documented application/component initialization path**

Follow the 0.6.1 `README`/component startup example exactly. Keep Bunting's existing window creation and app-owned state; only initialization/dependency imports change.

- [ ] **Step 4: Regenerate the standalone lockfile and prove source graph**

```bash
cd apps/bunting-terminal
cargo update -p gpui-kit --precise 0.6.1
cargo check
cargo tree | grep -F 'gpui-kit v0.6.1'
```

Expected: PASS; `Cargo.toml` contains no git URL to Zed or old `gpui-component`.

- [ ] **Step 5: Update workflow provenance guards**

Replace old checks for the pinned Zed/gpui-component git revisions with:

```bash
grep -Fq 'gpui-kit = "=0.6.1"' apps/bunting-terminal/Cargo.toml
! grep -Eq '^gpui(_platform)?[[:space:]]*=' apps/bunting-terminal/Cargo.toml
! grep -Eq '^gpui-component(-assets)?[[:space:]]*=' apps/bunting-terminal/Cargo.toml
cargo tree --manifest-path apps/bunting-terminal/Cargo.toml | grep -F 'gpui-kit v0.6.1'
```

- [ ] **Step 6: Update notices/reference inventory**

Record GPUI Kit Apache-2.0 as the direct UI dependency. Remove statements saying Bunting directly pins/adapts the old `gpui-component` commit or Zed GPUI git source.

- [ ] **Step 7: Run terminal validation**

```bash
cd apps/bunting-terminal
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/bunting-terminal .github/workflows/gpui-terminal.yml docs/gpui-terminal-reference-inventory.md
git commit -m "refactor: migrate desktop terminal to gpui-kit"
```

---

### Task 2: Lock semantic theme and density rules

**Files:**
- Create: `apps/bunting-terminal/src/style.rs`
- Modify: `apps/bunting-terminal/src/main.rs`
- Modify: `apps/bunting-terminal/src/shell/view.rs`
- Modify: terminal panel view files
- Test: `apps/bunting-terminal/src/style.rs`

**Interfaces:**
- Produces Bunting layout metrics only, not colors:

```rust
pub const TOP_STRIP_HEIGHT: Pixels = px(40.0);
pub const LEFT_PANE_WIDTH: Pixels = px(320.0);
pub const RIGHT_PANE_WIDTH: Pixels = px(360.0);
pub const BOTTOM_PANE_HEIGHT: Pixels = px(250.0);
pub const DENSE_ROW_HEIGHT: Pixels = px(26.0);
```

All colors come from `cx.theme()` semantic fields or GPUI Kit component variants.

- [ ] **Step 1: Add a source guard test for raw colors**

Create a test that scans terminal Rust source files and rejects application color constructors/patterns such as `rgb(`, `rgba(`, `hsl(`, `hsla(`, and six-digit `#RRGGBB` literals outside `style.rs`. `style.rs` itself must contain no raw color either for this phase.

- [ ] **Step 2: Run and observe current failures**

```bash
cd apps/bunting-terminal
cargo test no_raw_ui_colors
```

Expected: FAIL if current shell/views contain raw colors.

- [ ] **Step 3: Replace raw colors with semantic theme tokens**

Use `cx.theme().foreground`, `muted_foreground`, `background`, `sidebar`, `border`, `success`, `danger`, `warning`, `list_head`, and established component variants. Keep trading semantics consistent: bid/buy success, ask/sell danger only where the Kit theme supports those semantics.

- [ ] **Step 4: Centralize geometry constants**

Move the deliberate default pane widths/heights and dense row heights to `style.rs`. Do not wrap arbitrary spacing tokens; use GPUI Kit spacing APIs directly elsewhere.

- [ ] **Step 5: Run tests/Clippy**

```bash
cargo test no_raw_ui_colors
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src
git commit -m "style: standardize terminal theme and density"
```

---

### Task 3: Rebuild the default Trading workspace around the RIT information hierarchy

**Files:**
- Modify: `apps/bunting-terminal/src/shell/layout.rs`
- Modify: `apps/bunting-terminal/src/terminal/panel.rs`
- Modify: `apps/bunting-terminal/src/shell/view.rs`
- Test: `apps/bunting-terminal/src/shell/layout.rs`

**Interfaces:**
- Trading default panel placement:
  - top persistent strip: run/scenario, instrument, logical time/status, participant/connection;
  - left dock: `OrderBook` primary, `Tenders` adjacent;
  - center: `Chart` primary with instrument/time-and-sales tabs inside/adjacent where available;
  - right dock: `OrderTicket` primary, `Account`, `Risk` adjacent;
  - bottom dock: `Orders` primary, `News`, `Session` adjacent.
- `Research` and `Competition` remain presets over the same panel instances.

- [ ] **Step 1: Extract a pure layout specification and write a failing test**

Define:

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceSpec {
    center: Vec<PanelKind>,
    left: Vec<PanelKind>,
    right: Vec<PanelKind>,
    bottom: Vec<PanelKind>,
    active_left: usize,
    active_right: usize,
    active_bottom: usize,
}

fn workspace_spec(preset: WorkspacePreset) -> WorkspaceSpec;
```

Test Trading exactly:

```rust
assert_eq!(spec.center, vec![PanelKind::Chart]);
assert_eq!(spec.left, vec![PanelKind::OrderBook, PanelKind::Tenders]);
assert_eq!(spec.right, vec![PanelKind::OrderTicket, PanelKind::Account, PanelKind::Risk]);
assert_eq!(spec.bottom, vec![PanelKind::Orders, PanelKind::News, PanelKind::Session]);
```

- [ ] **Step 2: Run and verify failure**

```bash
cargo test workspace_spec
```

Expected: FAIL because layout is currently encoded directly in `reset_workspace`.

- [ ] **Step 3: Implement the pure spec and make `reset_workspace` consume it**

Keep `DockArea::new("bunting-market-workspace", Some(DOCK_VERSION), window, cx)` and existing panel entities. Build `DockItem` tabs from `WorkspaceSpec`; set widths from `style.rs`. Increment `DOCK_VERSION` because canonical default geometry semantics change.

- [ ] **Step 4: Keep presets as rearrangements, not separate dashboards**

Research must continue to use the same Chart/News/Book/Account/Risk/Session/Orders panel instances. Competition uses the same core panels plus `Competition`/`Tenders`; it does not create a separate root shell.

- [ ] **Step 5: Add persistence/reset behavior test**

Using GPUI Kit test support, initialize a DockArea with the new version, verify the default Trading placements, change one dock size, then reset Trading and verify the canonical sizes return. A normal app restart with unchanged `DOCK_VERSION` must preserve user sizes.

- [ ] **Step 6: Run tests**

```bash
cargo test --features gpui-kit/test-support workspace
```

If Cargo does not permit dependency feature syntax on the command line, expose an app feature `test-support = ["gpui-kit/test-support"]` in this same task and run `cargo test --features test-support workspace`.

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/bunting-terminal/src/shell apps/bunting-terminal/src/terminal/panel.rs apps/bunting-terminal/Cargo.toml
git commit -m "feat: make RIT layout the canonical workstation"
```

---

### Task 4: Replace the market chart with GPUI Kit authoritative candlesticks

**Files:**
- Create: `apps/bunting-terminal/src/chart.rs`
- Modify: `apps/bunting-terminal/src/main.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_trading.rs`
- Modify: `apps/bunting-terminal/src/terminal/state.rs`
- Test: `apps/bunting-terminal/src/chart.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug, PartialEq)]
pub struct CandleDatum {
    pub label: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume_lots: u64,
    pub last_event_sequence: u64,
}

pub fn candle_data(history: &MarketHistoryProjection) -> Vec<CandleDatum>;
```

The only renderer is `gpui_kit::component::chart::CandlestickChart`.

- [ ] **Step 1: Write a failing projection-conversion test**

Given one authoritative bar with integer tick values `100,110,90,105`, assert `candle_data` preserves those values exactly as `100.0,110.0,90.0,105.0`, retains volume/event sequence, and emits no point when history has no bars.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test candle_data
```

Expected: FAIL because `chart.rs` does not exist.

- [ ] **Step 3: Implement the pure view-model conversion**

Format the label from deterministic bar start/end logical time, not wall clock. Use checked/explicit numeric conversion; prices remain tick values unless instrument metadata supplies an exact display scale.

- [ ] **Step 4: Render with GPUI Kit's documented candlestick API**

Use the 0.6.1 component surface:

```rust
use gpui_kit::component::chart::CandlestickChart;

CandlestickChart::new(candles)
    .x(|d| d.label.clone())
    .open(|d| d.open)
    .high(|d| d.high)
    .low(|d| d.low)
    .close(|d| d.close)
    .body_width_ratio(0.55)
    .tick_margin(8)
```

Do not implement custom `Paint`/canvas/candle geometry in the app.

- [ ] **Step 5: Preserve L1 metrics without mixing them into history**

The chart header can show current bid, ask, spread, represented history sequence, and stale status. Change the old label `FIX L1 • 72 QUOTE WINDOW` to an authoritative history label such as `TRADE OHLC • SEQ <N>` when history exists.

- [ ] **Step 6: Explicitly render unavailable/stale states**

No bars + connected: `No authoritative trades in selected history window`.

No history response: `Authoritative trade history unavailable`.

Stale projection: retain last chart but show `STALE • HISTORY SEQ N / COMMITTED M`; do not silently relabel stale data live.

- [ ] **Step 7: Add source guard for chart implementations**

Test that terminal source has no `struct Candlestick` renderer or custom chart paint module and that `views_trading.rs` references `gpui_kit::component::chart::CandlestickChart` through `chart.rs`/imports. Also ensure old `quote_candles` symbol is absent.

- [ ] **Step 8: Run terminal tests**

```bash
cargo test chart
cargo clippy --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add apps/bunting-terminal/src
git commit -m "feat: render authoritative history with gpui-kit"
```

---

### Task 5: Modernize dense market/order/account panels with GPUI Kit components

**Files:**
- Modify: `apps/bunting-terminal/src/terminal/views_trading.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_admin.rs`
- Modify: `apps/bunting-terminal/src/terminal/views_research.rs`
- Modify: `apps/bunting-terminal/src/terminal/panel.rs`
- Test: `apps/bunting-terminal/tests/workstation_ui.rs`

**Interfaces:**
- Tables/lists consume immutable view rows keyed by domain IDs:

```rust
struct BookRowKey { side: Side, price_ticks: i64 }
struct OrderRowKey { order_id: String }
struct TenderRowKey { tender_id: String }
struct NewsRowKey { news_id: String }
```

- [ ] **Step 1: Add GPUI Kit UI test harness**

Create `tests/workstation_ui.rs` using `#[gpui_kit::test]` and the documented 0.6.1 test support. Add a smoke test that opens the app shell and can find/activate the Order Book, Chart, Order Ticket, Orders, Account, Risk, News, and Session panels.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test --features test-support --test workstation_ui
```

Expected: FAIL until test-support feature/init and stable panel identifiers are wired.

- [ ] **Step 3: Give every panel and repeated row stable IDs**

Use `PanelKind` stable names for panels. Book rows key by side+price; orders by authoritative order ID; tenders/news by their IDs. Never use loop index as GPUI element identity for mutable market collections.

- [ ] **Step 4: Use Kit tables/virtualized lists for dense data**

Order Book: SIDE / PRICE / QTY with right-aligned numeric columns and best-level emphasis.

Orders: ORDER ID / EVENT / STATUS / REASON.

Account: cash/buying power/NLV/positions only from authoritative account projection.

Risk: limits/fines/score only from authoritative risk projection.

News/Tenders: stable IDs, timestamps/logical time where authoritative, concise rows with details on selection.

- [ ] **Step 5: Keep order entry immediately reachable**

Use Kit `Input`, buttons/selectors/number input where API is appropriate. Limit/market selection remains explicit. Disable BUY/SELL while disconnected, stale in a way that prevents safe submission, or missing required server/run identity. Do not locally mark an order accepted on click; status changes only from server/FIX responses.

- [ ] **Step 6: Add interaction tests**

With a fake/in-memory client projection entity, test:

```text
focus symbol/command input -> enter
focus quantity -> type
select limit/market
submit buy -> exactly one OutboundCmd queued
submit sell -> exactly one OutboundCmd queued
disconnected -> submit controls disabled/no OutboundCmd
select order -> cancel -> exact authoritative order ID used
```

- [ ] **Step 7: Run UI tests**

```bash
cargo test --features test-support --test workstation_ui
```

Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add apps/bunting-terminal/src/terminal apps/bunting-terminal/tests apps/bunting-terminal/Cargo.toml
git commit -m "feat: modernize dense RIT workstation panels"
```

---

### Task 6: Make server/offline state first-class in the shell

**Files:**
- Modify: `apps/bunting-terminal/src/shell/view.rs`
- Modify: `apps/bunting-terminal/src/shell/layout.rs`
- Modify: `apps/bunting-terminal/src/local_server.rs`
- Test: `apps/bunting-terminal/tests/workstation_ui.rs`

**Interfaces:**
- Top strip displays: scenario/run, symbol/instrument, logical time/run status, committed sequence, participant/role, FIX state, local server health.
- Root diagnostics present `LocalServerDiagnostic.code`, detail, log path, and owned/external state.

- [ ] **Step 1: Add failing offline-state UI test**

Inject `LocalServerState::Exited` with diagnostic `SCENARIO_HASH_MISMATCH` and a disconnected FIX state. Assert the shell renders both the structured server reason and `OFFLINE`, and BUY/SELL are disabled.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test --features test-support --test workstation_ui offline
```

Expected: FAIL until typed diagnostics are rendered.

- [ ] **Step 3: Build compact top status strip**

Use a fixed `TOP_STRIP_HEIGHT`; no dashboard cards. Keep action buttons `Start/Stop Server`, `Refresh`, and `Reconnect` compact and state-aware. `Stop Server` is only enabled for an app-owned child.

- [ ] **Step 4: Render root-cause diagnostics in a bounded disclosure surface**

Show stable diagnostic code + concise detail. Provide a copyable log path but do not read/show more than the bounded sanitized tail produced by `local_server.rs`.

- [ ] **Step 5: Run UI tests**

```bash
cargo test --features test-support --test workstation_ui
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src/shell apps/bunting-terminal/src/local_server.rs apps/bunting-terminal/tests/workstation_ui.rs
git commit -m "feat: surface authoritative terminal health state"
```

---

### Task 7: Add keyboard workstation paths and command surface

**Files:**
- Modify: `apps/bunting-terminal/src/shell/layout.rs`
- Modify: `apps/bunting-terminal/src/shell/view.rs`
- Modify: `apps/bunting-terminal/src/terminal.rs`
- Test: `apps/bunting-terminal/tests/workstation_ui.rs`

**Interfaces:**
- Preserve textual commands: `TRADING`, `RESEARCH`, `COMPETITION`/`RIT`, `SERVER`, `STOP SERVER`, `RECONNECT`, `REFRESH`.
- Add direct focus/actions through GPUI action/keybinding machinery for symbol, order quantity, price, buy/sell, cancel, and bottom activity tabs.

- [ ] **Step 1: Add failing keyboard interaction tests**

Test that keyboard actions can switch Trading/Research/Competition, focus the order ticket, submit a valid mocked order, and return focus to the symbol/command field without mouse interaction.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test --features test-support --test workstation_ui keyboard
```

Expected: FAIL before keybindings/actions are registered.

- [ ] **Step 3: Register typed GPUI actions**

Define Bunting-specific action structs/enums in a focused module if current `terminal.rs` becomes large. Route actions to existing `Terminal` methods; never duplicate command construction in key handlers.

- [ ] **Step 4: Add visible focus and disabled states through Kit components**

Use Kit focus/input/button states. Do not implement custom focus painting.

- [ ] **Step 5: Run UI tests**

```bash
cargo test --features test-support --test workstation_ui
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal/src apps/bunting-terminal/tests/workstation_ui.rs
git commit -m "feat: add keyboard-first trading workflow"
```

---

### Task 8: Update macOS packaging and canonical desktop documentation

**Files:**
- Modify: `apps/bunting-terminal/scripts/package-macos-arm64.sh`
- Modify: `.github/workflows/gpui-terminal.yml`
- Modify: `apps/bunting-terminal/README.md`
- Modify: `README.md`
- Modify: `docs/specs/bunting-product-contract.md`
- Modify: `docs/specs/rit-tui-parity-matrix.md`
- Modify: PR #19 description after implementation is verified

**Interfaces:**
- Release artifact contains GPUI terminal + bundled Bunting WASM server/config templates and documents Wasmer 7.2.1 requirement/validation.
- GPUI terminal is described as canonical desktop UI; TUI remains migration fallback until the separate retirement gate passes.

- [ ] **Step 1: Update workflow dependency checks and tests**

Run terminal unit/UI tests, source-graph guards, local-server smoke, package ARM64 app/DMG, codesign verification, and architecture checks before publishing preview assets.

- [ ] **Step 2: Build package locally/Actions**

```bash
cd apps/bunting-terminal
cargo build --release
cargo test --features test-support
cargo clippy --all-targets -- -D warnings
./scripts/package-macos-arm64.sh
```

Expected: app bundle and DMG generation succeed on macOS ARM64; generated binary links/builds from GPUI Kit graph.

- [ ] **Step 3: Exercise the packaged app against a live local venue**

Use a fresh application-support directory, start the bundled server through the app-owned launcher, verify versioned admin health, establish FIX, request market book/account/risk/history, and submit/cancel a test order in a fixture scenario. Capture no credentials in artifacts.

- [ ] **Step 4: Update docs only to verified parity**

Describe the RIT-style default, authoritative history, GPUI Kit dependency, docking/customization, and current unsupported features. Do not claim pixel-identical RIT parity or proprietary formula equivalence.

- [ ] **Step 5: Update PR #19 body**

Replace the stale direct Zed/gpui-component and floating-window wording with the actual GPUI Kit/RIT architecture and validation evidence. Keep PR draft until all dependent correctness plans are merged/green.

- [ ] **Step 6: Commit**

```bash
git add apps/bunting-terminal .github/workflows/gpui-terminal.yml README.md docs/specs/bunting-product-contract.md docs/specs/rit-tui-parity-matrix.md
git commit -m "docs: make GPUI workstation the canonical desktop"
```
