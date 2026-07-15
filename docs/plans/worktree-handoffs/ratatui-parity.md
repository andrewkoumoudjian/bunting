# Ratatui parity worktree handoff

## Branch and base

- Branch: `codex/ratatui-parity`
- Base: `codex/product-contract` at `b935000`

## Completed client workflows

- Added named `local`, `remote` and `cloudflare-gateway` profiles with TCP or
  verified TLS, SenderCompID/TargetCompID, username, environment-injected
  password/token, team, run, requested role, heartbeat and explicit reset
  policy. Secrets are neither persisted nor displayed in raw FIX diagnostics.
- Extended `simfix-session` with bounded validated Logon fields so the client
  sends `553`, `554`, `10000`, and configured run/team/role fields through the
  normal sequenced session state machine. Reserved, duplicate and oversized
  fields reject.
- Reconnect preserves the in-memory session sequence, bounded journal, resend
  state and reconnect generation. Explicit reset creates a fresh session and
  sends `141=Y` only when the selected profile authorizes the request. The
  server remains authoritative over reset and identity.
- Added absolute incremental L2 application, bounded depth, committed cursor,
  `UC` reset reason, stale state and one-shot snapshot recovery after reconnect
  or reset. Snapshot and incremental price observations continue to feed the
  approved Longbridge candlestick component.
- Expanded the keyboard workspace to market, orders, account, simulation,
  collaboration, administration and session views. Named workspace layouts can
  be saved, loaded and removed through the command palette.
- Preserved limit/market ticket entry, persistent quick-order quantity,
  ladder-priced entry, cancel, replace, status, execution journal, portfolio
  fill projection, chart, help and bounded redacted FIX console.
- Added capability-aware loading/empty/disconnected/error/stale presentation.
  Product panels say `BACKEND UNAVAILABLE` until the corresponding FIX response
  type is observed. Administration remains deny-by-default because the current
  mapping does not return a verified privileged role claim.
- Kept the embedded engine acceptor behind explicit `--fixture`; ordinary
  `local` production mode connects to the portable server contract and the TUI
  never reads or mutates `bunting-engine`.

## Backend dependencies and honest gaps

- Current `simfix-mapping` implements only `D/F/G/H/V` and
  `8/9/j/W/X`. Security/run discovery (`x/y`, `U1/U2`), authoritative account
  and risk (`AN/AP`, `U3/U4`), news (`U5`), tenders (`U6`), OTC/composites
  (`U7`), assets/facilities (`U8`), reports/score (`U9`) and admin (`UA/UB`)
  require application-service and mapping lanes before their TUI workflows can
  be end-to-end.
- Bulk cancellation, IOC/FOK/post-only/iceberg/reserve/pegged/trailing/GTD/DAY,
  spreads, transport arbitrage, trade-at-settle and other special products
  remain capability-gated rather than being encoded as unsupported fake
  messages.
- The account view's position/cash/P&L numbers are explicitly labeled as a
  local, non-authoritative projection from observed fills. Buying power, cost
  basis, authoritative realized/unrealized P&L, NLV, exposure and penalties
  wait for AP/U4.
- Server-backed history/TAS and chart interval, moving average, EMA, RSI, zoom
  and annotations are missing. The existing chart is bounded OHLC/volume derived
  from observed FIX book snapshots.
- Chat/private chat/voice are disabled because no competition capability is
  negotiated. Compliance and instructor/assessor projections wait for strict
  authenticated audience support.
- FIX session state survives reconnects within one process but is not yet
  encrypted and persisted across process restarts. Hosted TLS, Cloudflare relay,
  QuickFIX/J and a second independent acceptor still require interoperability
  validation.
- Theme and sound preferences are represented in workspace configuration but
  are not yet active renderer/audio behavior, so the parity matrix leaves them
  missing.

## Source and dependency boundary

- Longbridge Terminal and `cli-candlestick-chart` attribution, exact revision
  and licenses are unchanged.
- Native TLS uses `rustls 0.23.42`, `tokio-rustls 0.26.4`,
  `rustls-native-certs 0.8.4` and `rustls-pemfile 2.2.0`. Their Cargo metadata
  records Apache-2.0/ISC/MIT-compatible licenses. They are native TUI transport
  dependencies only and do not enter the Worker dependency graph.
- No proprietary RIT resource, wording, layout or implementation body was
  copied. The workspace is independently organized around the committed
  workflow and FIX profile contracts.

## Verification

Focused verification completed before the root gates:

- `cargo test -p bunting-tui -p simfix-session` — 23 TUI tests and 9 session
  tests passed, including the engine-backed FIX acceptor end-to-end test.
- Coverage includes deterministic workspace state, keyboard/focus behavior,
  Logon field validation, reconnect/resend/reset/restore, snapshots and absolute
  incrementals, reset/stale state, raw-log redaction and golden terminal output.

All root-required checks ran from the parity worktree with
`CARGO_TARGET_DIR=/Users/andrewkoumoudjian/Documents/QUARCC/bunting/target`:

- `cargo metadata --locked --format-version 1 --no-deps` — passed.
- `cargo fmt --all --check` — passed.
- `cargo clippy --locked --workspace --all-targets -- -D warnings` — passed.
- `cargo test --locked --workspace` — passed.
- `cargo tree --locked -p bunting-engine | grep -F 'orderbook-rs v0.10.3'` —
  passed and printed `orderbook-rs v0.10.3`.
- `cargo check --locked --workspace --target wasm32-unknown-unknown` — passed.
- `git diff --check` — passed.
