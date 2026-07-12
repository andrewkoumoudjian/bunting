# Findings — documentation & agent-instruction inventory

Captured 2026-07-12 from a read-only pass over every doc and `AGENTS.md`.

## Summary counts

- **48 AGENTS.md files**: one root, one per top-level tree, one per crate/client/worker/service.
  They form a nested hierarchy where each level narrows its parent's rules.
- **No CLAUDE.md files** anywhere. All agent instructions live in `AGENTS.md`.
- **No `CONTRIBUTING.md`** / dedicated developer-setup file. Build/setup instructions are spread
  across root `README.md`, root `AGENTS.md`, `docs/architecture.md` §14, `docs/implementation-pathway.md`,
  and `.github/workflows/ci.yml`.
- **`.github` config**: only `ci.yml`. No issue/PR templates, no CODEOWNERS.

## Root & tree-level AGENTS.md (rules that constrain the reorg)

- **Root `AGENTS.md`** — master contract. Binding decisions: `orderbook-rs = 0.10.3` is the
  production dependency; no second matching engine/book/snapshot/replay/kill-switch; forks only for
  unfixable Wasm incompatibility (preserve MIT license + divergence log); plain Worker deployment
  (no Durable Object without a new ADR); Workers Cache mandatory for checksum-addressed snapshots.
  Lists Bunting-owned vs upstream responsibilities; `ref/` is read-only evidence; Worker packages
  must compile for `wasm32-unknown-unknown`; required checks (fmt, clippy, tests, dep policy, wasm).
- **`crates/AGENTS.md`** — crates stay protocol-focused, no Worker bindings except `orderbook`
  (wraps `OrderBook-rs`) and `worker-cache` (may use workers-rs). No parallel matching engine.
- **`clients/AGENTS.md`** — native runtimes/Tokio allowed; share protocol fixtures, reconnect
  safely, preserve sequence ordering, bound buffers.
- **`workers/AGENTS.md`** — plain Workers; no Durable Object without an ADR; `edge-api` owns
  HTTP/WebSocket routing; commit origin events before ack/publish.
- **`services/AGENTS.md`** — only where Cloudflare lacks a Rust binding; narrow, typed, authenticated.
- **`web/AGENTS.md`**, **`scenarios/AGENTS.md`**, **`tests/AGENTS.md`**, **`ref/AGENTS.md`**,
  **`vendor/AGENTS.md`**, **`docs/AGENTS.md`**, **`docs/adr/AGENTS.md`**, **`docs/ports/AGENTS.md`** —
  each narrows its domain (browser public APIs only; schema-validated scenarios; test invariants;
  read-only reference; vendor policy; docs record decisions not planned-as-implemented; ADR required
  sections; ports as provenance/divergence records).

## Per-crate AGENTS.md (present for real crates AND stubs)

Real crates: `market-types`, `market-events`, `orderbook` (thin adapter, fork only via ADR-0013),
`ledger`, `risk-engine`, `origin-store`, `command-transaction` (sans-I/O, commit origin before
cache), `quarcc-trading-engine` (preserve `quarcc.v1` names without importing unlicensed text),
`worker-cache`.

Stub crates (AGENTS.md only): `order-reconciliation`, `market-making`, `matching-engine`,
`scenario-schema`, `scenario-engine`, `simulation-clock`, `agent-models`, `protocol-legacy-nbc`,
`protocol-native`, `scoring`, `test-fixtures`, `replay-format`, `simfix-session`, `simfix-mapping`,
`simfix-wire`.

Per-client/worker/service/web/scenario: `clients/{python-sdk,rust-sdk,nautilus-adapter,
ritc-adapter,fix-bridge}`, `workers/{edge-api,analytics-consumer,export-consumer,
strategy-dispatch-consumer}`, `services/strategy-loader`, `web/trader-ui`, `scenarios/nbc`,
`tests/oracles`.

## docs/ contents

- `architecture.md` (canonical spec), `implementation-pathway.md`, `core-implementation-questions.md`,
  `codex-implementation-prompt.md`, `reference-inventory.md`, `reference-adoption.md`,
  `rust-reference-sprint-map.md`, `orderbook-rs-example-adoption.md`, `joaquin-repository-audit.md`.
- `docs/adr/0001`–`0013` at session start (plus `0014` added on main during the session).
  ADR-0013 (worker OrderBook-rs kernel) is authoritative and supersedes DO-based ownership.
- `docs/ports/`: `quarcc-trading-engine.md`, `nbc-simulation.md`, `nbc-scenario-catalog.md`,
  `ritc-market-making.md` (all provenance/divergence records; licenses unresolved → oracle-only).

## Structural conventions to preserve

- `ref/` is read-only evidence; production manifests use released packages.
- "There is no `market-run-do` runtime in the accepted architecture" (architecture.md §4); CI
  asserts `test ! -e workers/market-run-do/AGENTS.md`.
- Cache key: `https://cache.bunting.invalid/v1/orderbooks/{run_id}/{instrument_id}/{event_sequence}/{snapshot_checksum}`.
- Canonical check commands: `cargo fmt --all --check`; `cargo clippy --locked --workspace
  --all-targets -- -D warnings`; `cargo test --locked --workspace`; `cargo check --locked
  --workspace --target wasm32-unknown-unknown`.

## Inconsistencies to resolve during the reorg

1. **`crates/matching-engine/AGENTS.md`** names a crate that conflicts with the "no second matching
   engine" policy and is absent from README/architecture. Likely stale scaffolding from superseded
   ADR-0001. → remove or rename to a non-matching role and reconcile prose.
2. **`tests/AGENTS.md`** still lists "Durable Object recovery" as a test priority, though ADR-0013
   removed the Durable Object requirement.
3. **`docs/ports/nbc-scenario-catalog.md`** lists `workers/market-run-do/` as a canonical target,
   while architecture §4 says it's superseded and CI asserts it never exists.
4. **ADRs 0003/0004/0007/0012 + `workers/strategy-dispatch-consumer/AGENTS.md`** still describe an
   authoritative "MarketRun Durable Object" / "Durable Object SQLite" ownership model. Don't rewrite
   accepted ADR history — add a one-line superseded-by-0013 banner where prose reads as current, and
   fix the consumer AGENTS.md prose.
5. **`ref/ritc_mm/AGENTS.md`** is effectively empty (a lone blank line) — leave it; it's inside a
   read-only reference tree.
</content>
