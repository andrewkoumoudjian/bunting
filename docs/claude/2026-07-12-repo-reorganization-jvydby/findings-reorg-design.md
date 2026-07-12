# Findings — reorganization design (Cargo mechanics)

Derived 2026-07-12. This is the design reasoning that fed the plan. Where it differs from the
authoritative `docs/repository-reorganization.md` + ADR-0014 on `main`, defer to those (see the
session `README.md` reconciliation table).

## Verified facts that constrain the design

- Single root workspace, `resolver=2`, 10 members, `exclude=["ref","vendor"]`.
- Library package names are `bunting-*` except `quarcc-trading-engine`. The worker is
  `bunting-edge-api` (`crate-type=["cdylib","rlib"]`, `worker` + `d1`).
- The 8 library crates reference each other as **siblings** (`path = "../ledger"`, etc.). Only
  `worker-cache` and `edge-api` reference across directory levels.
- `workspace.dependencies` path-pins only two crates (`bunting-market-events`, `bunting-market-types`);
  the rest are per-crate `path = "../…"`.
- `.cargo/config.toml` is a single target-global flag (`getrandom_backend="wasm_js"`) — path-independent.
- Worker deploy: `wrangler.toml` `main = "build/worker/shim.mjs"`, `[build] command =
  "worker-build --release"`. `worker-build` hardcodes output into `./build/` relative to the crate.
- `.gitignore` ignores `/target/`, `.wrangler/`, `dist/` — **not** `build/` or `out/`.
- CI greps exact paths: root `Cargo.toml`, `crates/orderbook/Cargo.toml`, `.cargo/config.toml`,
  `crates/worker-cache/src/lib.rs`, asserts `workers/market-run-do/AGENTS.md` absent,
  `cargo tree -p bunting-orderbook`. All workspace commands use `--locked --workspace`.
- `crates/orderbook` (`bunting-orderbook`) is a wasm-shim wrapper (getrandom/uuid) around the
  crates.io `orderbook-rs=0.10.3` — **not a fork**.

## Verdict: one workspace, not two

The user's conceptual `packages/` + `bunting-rs/` split is good; implementing it as **two Cargo
workspaces is not idiomatic** and carries real cost:

- Two `Cargo.lock` and two `target/` → version drift risk (the `orderbook-rs`/`worker` pins), harder
  wasm reproducibility, deps compiled twice in CI.
- `[workspace.*]` inheritance does not cross workspace boundaries — `workspace.package`,
  `workspace.dependencies`, and especially `workspace.lints` (`unsafe_code=forbid`,
  `unwrap_used=deny`, clippy pedantic) would be duplicated and hand-synced. **Biggest reason to stay unified.**
- `--workspace` spans only one workspace → CI runs twice.
- `cargo tree -p bunting-orderbook` (a CI gate) resolves within one workspace only.

**Decision: one workspace at root**, members grouped into role directories. Single `Cargo.lock`,
single `target/`, single `--workspace`, shared lints.

## Migration mechanics (history-preserving)

All moves via `git mv`. The intra-`packages/` sibling path deps are unchanged; only the two crates
that cross directory levels need manifest edits.

- Root `Cargo.toml`: update `members` (keep **explicit, not globbed** — a `packages/*` glob breaks on
  the `Cargo.toml`-less scaffolds), the two path-pinned `workspace.dependencies`, and `exclude`
  (add `out`).
- CI grep paths to update: `crates/orderbook/Cargo.toml` → new path;
  `crates/worker-cache/src/lib.rs` → new path. Root `Cargo.toml` / `.cargo/config.toml` greps and the
  name-based `cargo tree` are unchanged.
- `.cargo/config.toml`: no change (target-global rustflag).

**Note on `--locked`:** moves change no dependency *versions*; `Cargo.lock` identifies path members
by name, not path, so the lockfile is byte-unchanged and `--locked` keeps passing. If it complains,
it's a manifest typo, not lock drift.

## WASM output / releases

- `worker-build` always emits into `./build/` relative to the built crate; `wrangler.toml` `main`
  resolves relative to the wrangler dir. You **cannot** point worker-build at a sibling `/out`.
- Treat `/out` as a release-staging mirror, not the wrangler path: gitignore `/out/` and the worker
  `build/`; a tag-driven release workflow runs `worker-build --release`, copies the `.wasm` + shim
  into `out/`, emits SHA-256 + a manifest (commit/toolchain/target/versions), and uploads to a
  GitHub Release. **Never commit the binary.**

## orderbook-rs fork — flag and reject (for now)

A fork contradicts the current binding decision (`orderbook-rs=0.10.3` released; no second book;
fork only for unfixable wasm break). Keep the crates.io dependency. The CI gate
`cargo tree -p bunting-orderbook | grep 'orderbook-rs v0.10.3'` enforces this. If a fork ever
becomes necessary, it lives behind a new ADR (in `vendor/` as patched third-party source, or as a
maintained package — the ADR picks one) with license, patches, upstream-sync, and compatibility tests.

## Sequencing to keep builds green

A Cargo workspace can't be partially moved and still compile, so the unit of "green" is the
PR/branch, verified before commit, not each intermediate `git mv`. Ordered steps: move libs → move
assembly → move stub scaffolds → edit root `Cargo.toml` → fix the two cross-level manifests → verify
`.cargo/config.toml` → update CI grep paths → update `.gitignore` + add release workflow → update
docs/AGENTS.md/README → run the full local verification gauntlet mirroring CI.
</content>
