# Bunting — repository reorganization + codex handoff plan

## Context

Bunting is a Rust stock-market simulation / exchange-testing platform that runs as a
plain Cloudflare Worker with the released `OrderBook-rs` crate as its matching kernel.
The user wants the repo reorganized so that *reusable engine packages* are visibly
separated from *the deployable Rust assembly*, plus a place for build output — and wants
a codex agent given precise instructions to execute the reorg and the sequenced next
implementation tasks, with the repo's own instruction files updated to match.

**What exploration established (important — it changes the shape of the work):**

- The repo is *already* a single, healthy Cargo workspace (`resolver=2`, 10 members,
  `exclude=["ref","vendor"]`), and the target skeleton already exists: ~15 of the
  directories under `crates/` are `AGENTS.md`-only scaffolds (matching, FIX, scenarios,
  market-making, scoring, nbc, …), and `clients/ services/ web/ scenarios/ tests/` are
  stubs too. Only the core kernel path is implemented:
  `market-types → market-events → {ledger, orderbook} → risk-engine → origin-store →
  command-transaction → workers/edge-api`.
- So this is a **re-layering of an existing skeleton**, not a from-scratch restructure.
- `orderbook-rs=0.10.3` / `pricelevel=0.8.4` / `worker=0.8.5` come from crates.io — there
  is **no fork**. `crates/orderbook` is a thin wasm-shim wrapper, not a fork.
- CI (`.github/workflows/ci.yml`) hard-codes several structural paths that must move in lock-step.
- Docs are extensive (48 AGENTS.md, 13 ADRs, architecture/pathway/port specs) and carry a
  few stale references that should be fixed while we're touching them.

**Decisions locked with the user:**

1. **Single Cargo workspace at root**, members grouped into `packages/` (compose-libs) and
   `bunting-rs/` (the deployable assembly). *Not* a two-workspace split.
2. **No orderbook-rs fork** — keep the crates.io released dependency. A fork would only ever
   live in `vendor/` behind a new ADR if an unfixable WASM break forced it.
3. **`/out` gitignored + GitHub Releases** for the WASM artifact. Never commit the binary.
4. Codex instructions sequence **all** next tasks, in documented order:
   reorg + doc-consistency cleanup → streaming → broader upstream order types → FIX/native adapters.

This is a planning task. The steps below are what the **codex agent** (and the follow-up
execution on branch `claude/bunting-repo-reorganization-jvydby`) will carry out. Nothing is
executed during planning.

---

## Target directory layout

```
/
├── Cargo.toml                 # single workspace, resolver=2 (stays at root)
├── Cargo.lock                 # single lockfile — unchanged by moves (see Migration §note)
├── .cargo/config.toml         # unchanged (target-global getrandom flag, path-independent)
├── rust-toolchain.toml
│
├── packages/                  # reusable libs that COMPOSE the engine (platform-neutral)
│   ├── market-types/          # bunting-market-types      (primitive)
│   ├── market-events/         # bunting-market-events
│   ├── orderbook/             # bunting-orderbook         (wasm-shim wrapper over orderbook-rs)
│   ├── ledger/                # bunting-ledger
│   ├── risk-engine/           # bunting-risk-engine
│   ├── origin-store/          # bunting-origin-store
│   ├── command-transaction/   # bunting-command-transaction
│   ├── quarcc-trading-engine/ # quarcc-trading-engine
│   └── <stub scaffolds, bucketed by role — see Migration step 3>
│
├── bunting-rs/                # the deployable assembly (imports packages/)
│   ├── Cargo.toml             # the cdylib worker package (was workers/edge-api)
│   ├── src/                   # lib.rs, d1_origin.rs
│   ├── migrations/            # 0001_origin_store.sql
│   ├── wrangler.toml
│   └── crates/
│       └── worker-cache/      # bunting-worker-cache (Cloudflare Cache API — runtime-only)
│
├── out/                       # release staging for built wasm (GITIGNORED)
├── ref/                       # excluded from workspace (submodules + checked-in port sources)
├── vendor/                    # excluded; policy-gated fork home (unused today)
├── clients/  services/  web/  scenarios/  tests/  docs/   # unchanged homes
└── workers/                   # remaining CONSUMER-worker stubs stay here
    ├── analytics-consumer/  export-consumer/  strategy-dispatch-consumer/
```

Rationale: `worker-cache` depends on the `worker` crate, so it is runtime infrastructure and
belongs under `bunting-rs/crates/`, not `packages/`. The other 8 crates are platform-neutral
engine libraries → `packages/`. `workers/` keeps the *other* deployable consumer workers; only
the edge assembly moves to `bunting-rs/`.

---

## Migration steps (single atomic PR — the workspace can't be half-moved and still build)

**1. Move the 8 compose-libs (history-preserving `git mv`):**
`crates/{market-types,market-events,orderbook,ledger,risk-engine,origin-store,command-transaction,quarcc-trading-engine}` → `packages/*`.
Their intra-`packages/` path deps (`../ledger`, `../market-types`, …) stay siblings → **unchanged**.

**2. Move the assembly:** `git mv workers/edge-api bunting-rs`, then
`git mv crates/worker-cache bunting-rs/crates/worker-cache`.

**3. Move the ~15 stub scaffolds** (no `Cargo.toml` → zero Cargo impact) into `packages/` by role:
- Engine/algorithm/sim: `agent-models, market-making, matching-engine, order-reconciliation, scoring, simulation-clock, scenario-engine, scenario-schema, replay-format` → `packages/`.
- Protocol: `protocol-native, protocol-legacy-nbc, simfix-wire, simfix-session, simfix-mapping` → `packages/` (optionally under `packages/protocols/`).
- `test-fixtures` → `packages/test-fixtures`.

**4. Root `Cargo.toml`** — update `members` (keep it **explicit, not globbed**; a `packages/*`
glob breaks on the `Cargo.toml`-less scaffolds), the two path-pinned `workspace.dependencies`,
and `exclude`:
```toml
members = [
  "packages/market-types", "packages/market-events", "packages/orderbook",
  "packages/ledger", "packages/risk-engine", "packages/origin-store",
  "packages/command-transaction", "packages/quarcc-trading-engine",
  "bunting-rs", "bunting-rs/crates/worker-cache",
]
exclude = ["ref", "vendor", "out"]
# workspace.dependencies:
bunting-market-events = { path = "packages/market-events" }
bunting-market-types  = { path = "packages/market-types" }
```

**5. Fix the only two manifests that crossed directory levels:**
- `bunting-rs/Cargo.toml` (was edge-api): the five `../../crates/*` deps become
  `../packages/command-transaction`, `../packages/market-types`, `../packages/orderbook`,
  `../packages/origin-store`, and `crates/worker-cache`. (`bunting-market-events` and `worker`
  are inheritance-based → untouched.)
- `bunting-rs/crates/worker-cache/Cargo.toml`: `bunting-market-types` path → `../../packages/market-types`.

**Note on `--locked`:** moves change no dependency *versions*; `Cargo.lock` identifies path
members by name, not path, so the lockfile is byte-unchanged and `--locked` keeps passing.

**6. `.cargo/config.toml`** — verify no change needed (it is a target-global rustflag).

**7. CI path updates in `.github/workflows/ci.yml`** (the "Enforce architecture dependency policy" step):
- `crates/orderbook/Cargo.toml` → `packages/orderbook/Cargo.toml` (both the `getrandom` and `uuid` greps).
- `crates/worker-cache/src/lib.rs` → `bunting-rs/crates/worker-cache/src/lib.rs`.
- `test ! -e workers/market-run-do/AGENTS.md` — still passes (path never existed); leave or tidy the comment.
- Unchanged (verify only): root `Cargo.toml` greps, `.cargo/config.toml` grep,
  `cargo tree -p bunting-orderbook` (name-based), all `--locked --workspace` commands.

**8. WASM output / releases:**
- `.gitignore`: add `/out/` and `bunting-rs/build/` (worker-build emits to `bunting-rs/build/`;
  wrangler `main` resolves relative to `bunting-rs/`, so leave that path as-is).
- Add a **tag-driven release workflow** (`.github/workflows/release.yml`, `on: push: tags: v*`)
  that runs `worker-build --release` in `bunting-rs/`, copies `bunting-rs/build/worker/*.wasm`
  (and the shim) into `out/`, and uploads them to a GitHub Release.

---

## Instruction files to write / update in the repo

These are the "instructions in the repo" deliverable. Create/update during execution:

1. **`docs/reorg-execution-prompt.md`** (NEW) — the codex agent's step-by-step contract for the
   reorg itself: the target tree, the exact `git mv`/manifest/CI edits above, the "single atomic
   PR / verify-before-commit" rule, and the local verification gauntlet. Self-contained.
2. **`docs/codex-implementation-prompt.md`** (UPDATE) — refresh all `crates/…` and
   `workers/edge-api` path references to `packages/…` and `bunting-rs/…`; add a short
   "next tasks, in order" section pointing at the four sequenced tracks below.
3. **Root `AGENTS.md` + `README.md`** (UPDATE) — new workspace layout, the `packages/` vs
   `bunting-rs/` convention, the `/out` + Releases policy, and corrected check commands.
4. **`docs/architecture.md` + `docs/implementation-pathway.md`** (UPDATE) — repository-ownership
   section and any path references re-pointed to the new layout.
5. **New tree-level `AGENTS.md`** for `packages/` and `bunting-rs/` describing each bucket's rule
   (packages = platform-neutral compose libs, no Worker bindings except `orderbook`; bunting-rs =
   the Worker assembly + runtime-only support crates). Keep the existing per-crate AGENTS.md with
   their crates (they move via `git mv`).

**Doc-consistency fixes to fold in (found during exploration):**
- `crates/matching-engine/AGENTS.md` names a crate that conflicts with the "no second matching
  engine" policy and is absent from README/architecture → either remove the scaffold or rename
  it to a non-matching role (e.g. an event-translation helper) and reconcile the prose.
- `tests/AGENTS.md` still lists "Durable Object recovery" → drop/replace per ADR-0013.
- `docs/ports/nbc-scenario-catalog.md` lists `workers/market-run-do/` as a target while CI asserts
  it never exists and architecture §4 says it's superseded → repoint to the plain-Worker path.
- ADRs 0003/0004/0007/0012 + `workers/strategy-dispatch-consumer/AGENTS.md` still describe an
  authoritative "MarketRun Durable Object" → do **not** rewrite accepted ADR history; instead add
  a one-line superseded-by-0013 banner where prose reads as current (respect the "amend via new
  ADR" rule). Fix the consumer AGENTS.md prose.
- `ref/ritc_mm/AGENTS.md` is effectively empty → leave (it's inside a read-only reference tree).

---

## Branches — safe to delete or archive

Authoritative from PR metadata (`merged_at` set = merged; the list API's `merged:false` is a known quirk):

| Branch | State | Recommendation |
|---|---|---|
| `main` | default | **Keep.** |
| `claude/bunting-repo-reorganization-jvydby` | our working branch | **Keep** (reorg lands here). |
| `feat/bootstrap-architecture` | PR #1 **merged** | **Delete** — fully merged. |
| `feat/orderbook-rs-worker-kernel` | PR #2 **merged** | **Delete** — fully merged. |
| `feat/command-transaction-origin-store` | PR #3 **merged** | Already deleted on remote — nothing to do. |
| `feat/deterministic-kernel-vertical-slice` | **no PR, never merged** | **Archive then delete.** Its custom BTreeMap/arena book was declared obsolete by ADR-0013; useful parts were already salvaged into PR #2. Tag `archive/deterministic-kernel-vertical-slice` for provenance, then delete the branch. |

Deletion/tagging is **not** performed automatically — it needs the user's explicit go-ahead and is
outside the designated working branch's push scope. Listed here as the requested recommendation.

---

## Next-task sequencing (for the codex instructions, documented order)

1. **Reorg + doc-consistency cleanup** (this PR) — the moves above + the doc fixes.
2. **Streaming market data** (ADR-0011/0013) — plain Worker WebSocket endpoint, snapshot + absolute
   L1/L2 deltas, committed event-sequence cursors, `stream.reset`/event-tail recovery, bounded
   subscriptions/frames/backlog. Target: `packages/protocol-native` + `bunting-rs`.
3. **Broader upstream order types** — expose IOC/FOK/post-only/replace/mass-cancel/STP/fees,
   host-driven GTD/DAY expiry, depth/metrics/impact via the existing `packages/orderbook` adapter
   (call upstream APIs; never reimplement).
4. **FIX / native adapters** — build out `packages/simfix-{wire,session,mapping}` and
   `clients/{fix-bridge,ritc-adapter,nautilus-adapter}`; IronFix as codec candidate, QuickFIX/J +
   Fixer as conformance oracles. Translate to Bunting commands — no second matching engine.

Each track is its own follow-up PR; do not stack them onto the reorg PR.

---

## Verification (run before committing the reorg PR — mirrors CI)

```bash
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-orderbook | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
# assembly still builds to the wrangler path:
cd bunting-rs && worker-build --release   # expect bunting-rs/build/worker/shim.mjs
# re-run the CI policy greps against the NEW paths (packages/orderbook, bunting-rs/crates/worker-cache)
```

Expected: `Cargo.lock` unchanged; all five workspace checks green; `worker-build` produces the shim;
CI greps pass at the new paths. If `--locked` complains, it's a manifest typo, not lock drift.

## Delivery

Commit the reorg + updated instruction docs to `claude/bunting-repo-reorganization-jvydby`, push
with `git push -u origin claude/bunting-repo-reorganization-jvydby`. Open a PR only if the user asks.
Report branch-cleanup recommendations for the user to action.
