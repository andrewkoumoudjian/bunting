# Findings — repository structure, build, and references

Captured 2026-07-12 from a read-only exploration of the workspace.

## Overview

Bunting is a Rust stock-market simulation / exchange-testing platform whose runtime target is
a **plain Cloudflare Worker** and whose matching core is the released `OrderBook-rs` crate
(v0.10.3). Edition 2024, Rust 1.88, Apache-2.0.

A defining characteristic: the repo is **mostly scaffolding**. Nearly every directory contains
only an `AGENTS.md` describing planned responsibilities; only 10 crates are implemented and
wired into the Cargo workspace.

## 1. Top-level layout (at session start, before any reorg)

```
/
├── Cargo.toml            # workspace root (10 members)
├── Cargo.lock
├── rust-toolchain.toml   # pinned 1.88.0, target wasm32-unknown-unknown
├── README.md
├── AGENTS.md             # mission + binding architectural decisions
├── .gitmodules           # 27 ref/ submodules
├── .gitignore
├── .cargo/config.toml    # wasm getrandom backend cfg
├── .github/workflows/ci.yml
├── crates/               # 24 dirs, only 9 are real crates (rest = AGENTS.md stubs)
├── workers/              # Cloudflare Workers; only edge-api is real
├── clients/              # all stubs
├── services/             # stub (strategy-loader)
├── web/                  # stub (trader-ui)
├── scenarios/            # stub (nbc)
├── tests/                # stub (oracles)
├── vendor/               # empty except policy README.md + AGENTS.md
├── docs/                 # ADRs, architecture, port specs
└── ref/                  # 27 git submodules + 3 checked-in port sources
```

## 2. The Cargo workspace

Root `Cargo.toml`: `resolver = "2"`, `exclude = ["ref","vendor"]`, 10 members.

| Path | Crate name | Purpose | Intra-workspace deps |
|------|-----------|---------|----------------------|
| `crates/market-types` | `bunting-market-types` | Checked identifiers + fixed-point values | (leaf, serde) |
| `crates/market-events` | `bunting-market-events` | Protocol-neutral commands + canonical event envelopes | market-types |
| `crates/orderbook` | `bunting-orderbook` | Thin version-pinned adapter around `OrderBook-rs` | market-events, market-types + orderbook-rs, pricelevel, getrandom(wasm_js), uuid(js) |
| `crates/ledger` | `bunting-ledger` | Participant cash / position / reservation projections | market-events, market-types |
| `crates/risk-engine` | `bunting-risk-engine` | Bunting participant/account limits | ledger, market-events, market-types |
| `crates/origin-store` | `bunting-origin-store` | Authoritative projections, idempotency, expected-version commits, recovery | ledger, market-events, market-types, risk-engine |
| `crates/command-transaction` | `bunting-command-transaction` | Recovery/risk/matching/accounting/commit orchestration | ledger, market-events, market-types, orderbook, origin-store, risk-engine + sha2 |
| `crates/quarcc-trading-engine` | `quarcc-trading-engine` | WASM-safe compatibility contract for legacy `quarcc.v1` surface | serde |
| `crates/worker-cache` | `bunting-worker-cache` | Immutable Workers Cache snapshot adapter | market-types + worker |
| `workers/edge-api` | `bunting-edge-api` | Cloudflare Worker entrypoint; `crate-type = ["cdylib","rlib"]` | command-transaction, market-events, market-types, orderbook, origin-store, worker-cache + worker (d1) |

Dependency layering: `market-types → market-events → {ledger, orderbook} → risk-engine →
origin-store → command-transaction → edge-api`.

**Stub crate directories under `crates/` (AGENTS.md only, NOT members, no Cargo.toml):**
`agent-models`, `market-making`, `matching-engine`, `order-reconciliation`, `protocol-legacy-nbc`,
`protocol-native`, `replay-format`, `scenario-engine`, `scenario-schema`, `scoring`,
`simfix-mapping`, `simfix-session`, `simfix-wire`, `simulation-clock`, `test-fixtures`.

## 3. Vendored / forked dependencies

- **No active fork.** `orderbook-rs = "=0.10.3"`, `pricelevel = "=0.8.4"`, `worker = "=0.8.5"`
  come from crates.io (declared in `[workspace.dependencies]`), not git/path.
- **`vendor/`** is deliberately empty — only `vendor/README.md` (admission policy requiring
  `LICENSES/`, `NOTICE.md`, `UPSTREAM.md`, `PATCHES.md`) and `vendor/AGENTS.md`.
- A minimal attributed `OrderBook-rs` fork is permitted *only* if a wasm incompatibility can't be
  fixed upstream (per AGENTS.md / ADR-0013) — not currently exercised.

## 4. WASM build target & output

- Target `wasm32-unknown-unknown` (pinned in `rust-toolchain.toml`).
- `.cargo/config.toml` sets `rustflags = ["--cfg", 'getrandom_backend="wasm_js"']` for that target.
- Build tool `worker-build` (workers-rs), invoked by wrangler: `workers/edge-api/wrangler.toml`
  → `[build] command = "worker-build --release"`, `main = "build/worker/shim.mjs"`.
- Output lands in `workers/edge-api/build/worker/` (not committed; dir not present until built).
  `.gitignore` ignores `dist/`, `/target/`, `.wrangler/` but **not** `build/`.
- No `/out`, `/dist`, or `/releases` dir; no release artifacts checked in.

## 5. workers/ (Cloudflare) setup

```
workers/
├── edge-api/                       # ONLY implemented worker
│   ├── Cargo.toml                  # bunting-edge-api, cdylib+rlib
│   ├── wrangler.toml               # D1 binding ORIGIN_DB → bunting-origin
│   ├── migrations/0001_origin_store.sql
│   ├── src/lib.rs                  # worker entry + routes
│   └── src/d1_origin.rs            # D1-backed origin store
├── analytics-consumer/AGENTS.md    # stub
├── export-consumer/AGENTS.md       # stub
└── strategy-dispatch-consumer/AGENTS.md  # stub
```

- `wrangler.toml`: `compatibility_date = "2026-07-12"`, one D1 binding (`ORIGIN_DB` /
  `bunting-origin`, `database_id = "REPLACE_WITH_D1_DATABASE_ID"`), `migrations_dir = "migrations"`.
- Routes: `POST /v1/runs/:run_id/instruments/:instrument_id/orders` and `.../orders/:order_id/cancel`.
  Bearer auth + `X-Bunting-Participant-Id`.
- No Durable Objects (ADR-0013); Workers Cache API used for snapshots.

## 6. Reference material, ports, external code

`ref/` — 27 git submodules (all unpopulated at session time) declared in `.gitmodules`, plus 3
**checked-in port source trees** (real content):

- `ref/quarcc-trading-engine/` — C++ QUARCC engine (`CMakeLists.txt`, `contracts/`, `engine-cpp/`,
  `python_client/`, `scripts/`). Target crate `crates/quarcc-trading-engine`; spec `docs/ports/quarcc-trading-engine.md`.
- `ref/nbc_engine/` — NBC engine source (`app/`). Target `crates/protocol-legacy-nbc` + `scenarios/nbc`;
  specs `docs/ports/nbc-simulation.md`, `nbc-scenario-catalog.md`.
- `ref/ritc_mm/` — Rust `rit-market-maker` (axum/tokio/reqwest). Target `crates/market-making` +
  `clients/ritc-adapter`; spec `docs/ports/ritc-market-making.md`.

Empty reference submodules cover FIX (`ironfix`, `fixer`, `ferrumfix`, `quickfixj`, `ironsbe`),
order books (`orderbook-rs`, `pricelevel`, `liquibook`, `exchange-core`, `option-chain-orderbook`,
`fauxchange`), simulators (`abides`, `nbc-hft-simulation`, `nautilus-trader`, `market-maker-rs`,
`barter-rs`), infra (`workers-rs`, `wirefilter`, `cqrs`, `nexosim`, `slotmap`, `intrusive-rs`,
`rand`, `postcard`, `proptest`).

## 7. CI / build scripts

- Only CI file: `.github/workflows/ci.yml` — single job `rust-kernel`. Steps: checkout
  (submodules: false), install pinned toolchain + wasm target, an **architecture
  dependency-policy gate** (greps that pin `orderbook-rs "=0.10.3"`, `worker "=0.8.5"`,
  `getrandom "=0.3.4"`, `uuid "=1.23.4"`, the wasm getrandom cfg, `Cache::default()`, and asserts
  `workers/market-run-do/AGENTS.md` absent), then `cargo fmt --check`, `cargo clippy --locked
  --workspace --all-targets -D warnings`, `cargo test --locked --workspace`, `cargo tree`
  upstream-version verification, `cargo check --target wasm32-unknown-unknown`.
- No `justfile`, `Makefile`, or `build.rs`. Build automation is cargo + wrangler/worker-build.
- Root `Cargo.toml` lints: `unsafe_code = "forbid"`, clippy `pedantic`/`all` warn,
  `unwrap_used`/`expect_used`/`panic` denied, `float_arithmetic` warn.

## Key takeaway

The target directory skeleton already exists via AGENTS.md-only stubs across `crates/`,
`workers/`, `clients/`, `services/`, `web/`, `scenarios/`, `tests/`, with matching port source in
`ref/quarcc-trading-engine`, `ref/nbc_engine`, `ref/ritc_mm`, and per-target specs in `docs/ports/`.
Only the core kernel path is implemented today.
</content>
