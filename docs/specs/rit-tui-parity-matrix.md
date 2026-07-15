# RIT workflow to Ratatui parity matrix

Status: target workflow contract derived from clean-room static evidence;
implementation state reconciled 2026-07-15

This matrix specifies user-visible workflow parity, not proprietary layout,
wording or implementation. RIT resources, UI text and bytes are prohibited to
copy. Every participant workflow must be backed by the FIX profile so the TUI
is a replaceable view, while instructor/admin workflows use the same application
service and role model.

| Workflow | Required Ratatui behavior | FIX/application source | Evidence | Current state |
|---|---|---|---|---|
| Connect/authenticate | Configure endpoint/profile, log on, show role/run/session and recover sequence state. | `A`, `0`-`5`, `10000` | `0048`, `0053` observed surface; behavior partly unresolved | Partial: named local/remote/gateway profiles, TCP/TLS, credential/profile Logon fields, reconnect, authorized reset, health and redacted journal are implemented; backend role confirmation and persisted session restore are missing. |
| Scenario/run selection | List eligible published scenarios/runs, status, period/tick/time remaining and policy versions. | `U1/U2` | `0001`-`0003` observed | Missing. |
| Instrument discovery | Browse security master, capabilities, tick/lot, currency, bounds, fees and dependencies. | `x/y` | `0004`-`0006`, `0051` observed | Partial: single configured market. |
| Market overview | Show L1, last, volume and run state with stale/reset indication. | `V/W/X/Y/UC` | `0017`, `0047`, `0050` observed | Partial: snapshots, absolute incremental updates and explicit stale/reset state are implemented; current backend supplies only the narrow snapshot slice. |
| Depth | Navigate bounded raw and aggregated L2 and market-impact estimates. | `V/W/X` | `0015`, `0016`, `0019`, `0050` observed | Partial: bounded raw depth and ladder navigation consume snapshots/incrementals; aggregated depth and impact are missing. |
| Chart/history/time-and-sales | Select instrument/interval and inspect price history, bars and committed trades. | `AD/AE`, `V/W/X` | `0018`, `0055` observed; bar rules unresolved | Partial: attributed Longbridge OHLC/volume rendering uses bounded observed book samples; server history/TAS, interval selection, MA/EMA/RSI, zoom and annotations are missing. |
| Submit order | Enter buy/sell, market/limit, quantity, price and supported TIF; show preflight and committed result. | `D`, `8`, `j` | `0007` observed; validation order unresolved | Partial: market/limit ticket exists. |
| Cancel/replace/status | Select own order, cancel/replace/query; support bounded bulk cancellation when policy allows. | `F/G/H`, `8/9/j` | `0008`-`0010` observed; expression grammar unresolved | Partial: command actions exist; full lifecycle missing. |
| Orders/fills | Filter live, open and historical orders and fills, preserving IDs and partial-fill progression. | `8/9`, `H` | `0008`, `0020`, `0047` observed | Partial: orders tab exists. |
| Positions/account | Show position, pending/reserved state, cash, buying power, NLV, cost, VWAP and realized/unrealized P&L. | `AN/AP`, `U3/U4` | `0021`-`0023` observed; formulas unresolved | Partial presentation only: a clearly labeled non-authoritative fill projection and capability state exist; authoritative AP/U4 account data is missing. |
| Risk/limits/fines | Show instrument and gross/net limits, utilization, warnings, fines, stop-loss and rate-limit waits. | `U3/U4`, `UB` admin | `0025`-`0028` observed; formulas unresolved | Missing. |
| News | Browse ordered public/private news, unread state and logical publication time. | `U5` | `0031`, `0050` observed; market effects unresolved | Missing. |
| Tenders | List active/history, inspect terms/expiry, enter bid or accept/decline and show outcome. | `U6` | `0029`, `0030`, `0050` observed; allocation unresolved | Missing. |
| OTC | Propose, counter, accept, reject and inspect lifecycle/history where enabled. | `U7` | `0013` observed; selection/settlement unresolved | Missing. |
| Assets/leases/conversions | Inspect facilities/assets/history/capacity and perform authorized lease/use/release/conversion actions. | `U8` | `0032`-`0035`, `0048` observed; formulas unresolved | Missing. |
| Composite/special products | Enter spread, transport-arbitrage, trade-at-settle or product actions only when capability/policy is explicit. | `U7` plus discovery | `0011`, `0012`, `0014`, `0038`-`0040` observed; formulas unresolved | Missing. |
| Reports/score/leaderboard | Request participant reports and show transaction/P&L/TAS/OTC summaries, score and rank with policy version. | `U9` | `0043`, `0044`, `0054` observed; equations unresolved | Missing. |
| Instructor run control | Publish/select scenario, create run, start/pause/resume/advance/set pacing/terminate and audit interventions. | `UA` | `0001`-`0003`, `0057` observed | Missing. |
| Instructor monitoring/compliance | Select run/team/participant, view allowed projections, issue targeted news/limits/compliance actions. | `U5`, `UB` | `0031`, `0057` observed; exact permissions unresolved | Missing. |
| Recovery and diagnostics | Display connection/FIX/committed cursors, gaps, resets, stale panels and actionable reconnect state. | session messages, `UC` | `0047`; Bunting-added recovery | Partial: FIX and committed cursors, bounded journal, reconnect generation, stale/reset reason and snapshot recovery request are implemented; disk-persisted session restore and hosted interoperability remain missing. |
| Help/accessibility | Discover keys and capabilities without hiding an action in a mouse-only path; preserve bounded readable tables. | local presentation | Bunting-added | Partial: keyboard help, seven workspaces, focus tests and save/load/remove layout commands exist; theme/sound and broader accessibility validation are missing. |
| Voice/chat | No parity requirement in core TUI unless a competition profile explicitly enables collaboration. | external collaboration adapter | `0056` UI-only; competition relevance unresolved | Explicitly out of core scope. |

## Acceptance rule

Parity for a row means the workflow is usable end to end against committed
application-service state, has loading/empty/error/stale/reset states, enforces
its audience, and has keyboard-operable tests. A visually similar panel without
the action, recovery or authorization path is not parity. Formulas labeled
unresolved use a displayed versioned Bunting policy and cannot be labeled RIT
compatible.
