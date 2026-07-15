# Product reconciliation worktree handoff

## Branch and merge record

- Branch: `codex/reconcile-bunting-product`
- Base: `origin/main` at `9c53967943d6d3b556a40b7fce9f6edc8ab3b3b3`
- Product contract merge: `4ba9105`
- Simulation domain merge: `98c9f2e`
- Portable server merge: `03e5725`
- Ratatui parity merge: `4036b83`

The source branches were merged in the required order. Git reported no textual
conflicts. Semantic compilation found one cross-lane API conflict:
`simfix-session::SessionConfig` gained initiator Logon fields in the Ratatui
lane while the native acceptor retained the older initializer. The acceptor now
uses an empty initiator-only field set; server-side CompID, profile and
credential validation remains authoritative.

The checked-in local server profile also referenced a nonexistent
`./scenario.json`. It now references the canonical minimal immutable scenario
at `apps/bunting-server/config/scenario.json`, so the documented root-level
startup command can create run 1 without an external file.

## Older work disposition

- `bunting-wt-local-market-tui` contains no committed behavior absent from the
  Ratatui lane; its Longbridge chart, order ticket, ladder, portfolio and local
  FIX fixture are already present.
- `bunting-wt-s5-subscriptions` has dirty tRPC-specific subscription work. Its
  committed cursor, reset and bounded-private-backlog intent is already covered
  by the browser stream and FIX recovery tests; the obsolete tRPC envelope was
  not copied.
- `bunting-wt-s4-trpc-clients-fix` has dirty duplicate `fix-*` and tRPC client
  packages superseded by `simfix-*`, `bunting-application`, the native server
  and the Worker adapter. Nothing was copied.
- `bunting-rit-analysis-20260713` remains external proprietary/static evidence.
  No implementation bytes, formulas, UI text or inferred internal behavior
  were imported.

## Verified integrated capabilities

- `REC-01`: one production `bunting-engine` privately owns OrderBook-rs 0.10.3.
- `REC-02`: scenario publication/lifecycle, run transitions, deterministic
  workflows, scoring and leaderboard projections pass simulation-domain tests.
- `REC-03`: all seven built-in policies emit bounded participant intent through
  mandatory QUARCC composition, with deterministic snapshot/RNG tests.
- `REC-04`: native FIX and Worker preparation commit identical authoritative
  state, and native file recovery restores the acknowledged state.
- `REC-05`: FIX reconnect, resend, gap fill, snapshot restore, incremental
  reset/stale behavior and bounded private browser backlogs pass package tests.
- `REC-06`: actor/participant mismatch is denied, audience selection is
  deny-by-default, and QUARCC reports are isolated to their participant.
- `REC-07`: the Ratatui engine-backed FIX fixture trades and refreshes the book;
  local/remote/Cloudflare-gateway profiles parse without storing secrets.
- `REC-08`: native startup using the checked-in local profile returns a healthy
  response and an authenticated run-1 projection.

## Genuinely unresolved requirements

- `REC-GAP-01` — FIX-only full competition surface: `x/y`, `U1` through `UB`,
  `AN/AP`, tenders, OTC, facilities, reports, scoring and administrator mapping
  are not connected to `bunting-application`. Evidence: the Ratatui handoff and
  parity matrix keep these workflows missing or partial. The blocker is missing
  versioned mapping/application implementations, not a merge conflict.
- `REC-GAP-02` — black-box Ratatui-to-native-server coverage: the TUI has an
  engine-backed FIX acceptor fixture and the native server has FIX/Worker path
  equivalence, but no automated test launches the separate server binary and
  drives the interactive TUI transport against it. This needs a reusable
  process/socket harness and deterministic shutdown contract.
- `REC-GAP-03` — Cloudflare-to-external-gateway live proof: topology,
  configuration, relay and Worker outbound session code exist, but no deployed
  Worker staging socket was available for this reconciliation. Local relay
  configuration is not evidence of Cloudflare network interoperability.
- `REC-GAP-04` — independent FIX interoperability: no QuickFIX/J plus second
  independent implementation was available locally. Internal wire/session
  golden tests do not satisfy the two-implementation gate.
- `REC-GAP-05` — complete stream integration: bounded committed browser streams
  and FIX snapshot/incremental mappings are tested independently, but the
  expanded public/private competition projections in `REC-GAP-01` are not
  connected end to end.
- `REC-GAP-06` — dynamic multi-tenant role isolation: type-level audience and
  participant isolation tests pass, while the native server and relay support
  one static identity/session binding per process. Team/instructor/admin FIX
  projections need authenticated multi-tenant bindings before parity can be
  claimed.
- `REC-GAP-07` — canonical native/Wasm runtime replay: native snapshot/replay
  equality and workspace Wasm compilation pass, but a Wasm runtime executing
  the same canonical replay vector was not available. Compilation alone is not
  runtime equivalence evidence.

Unknown RIT formulas remain unresolved and are not blockers to the explicitly
versioned Bunting-native policies. They do block any claim of exact RIT internal
equivalence.

## Verification evidence

During reconciliation, `cargo check --locked --workspace --all-targets` first
exposed `SessionConfig.logon_fields`; it passed after the semantic repair.
`cargo test --locked --workspace` then passed, including 23 Ratatui tests, FIX
session/wire/mapping tests, simulation lifecycle/replay/scoring tests,
QUARCC-agent tests, Worker subscription/audience tests, and native
FIX/Worker/restart equivalence.

The native server was launched from the repository root with the local profile
and an isolated `/tmp` origin path. `GET /health` returned
`{"service":"bunting-server","status":"ok"}` and authenticated
`GET /admin/runs/1` returned run 1 at committed/event sequence 0.

The final metadata, formatting, Clippy-with-warnings-denied, workspace-test,
OrderBook-rs pin, Wasm and `git diff --check` gates all passed. The Worker
release build produced its ignored JavaScript shim and optimized Wasm module;
both D1 migrations and every checked-in server/product/FIX/scenario JSON file
were discoverable and parseable. Dependency metadata showed no package-to-app
or package-to-`bunting-rs` edge, generated release files were untracked, and
the remaining old tRPC/path strings were confined to explicitly historical
plans, ADRs and handoff evidence rather than active manifests or source paths.
