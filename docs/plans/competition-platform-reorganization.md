# Competition-platform reorganization plan

Status: accepted execution plan. Phases 0–2 implemented on 2026-07-28; Phases
3–5 remain gated by the organizer decisions below, and Phase 6 remains last in
the declared sequence.

Inputs:

- [`../streamlining-audit.md`](../streamlining-audit.md) — what the repository is today (findings F1–F5)
- [`../hackathon-base-plan.md`](../hackathon-base-plan.md) — the live-trading-competition target (§1–§13)
- [`../repository-reorganization.md`](../repository-reorganization.md) — the binding execution contract for path changes
- `AGENTS.md` and every scoped `AGENTS.md` under a touched path
- `docs/adr/AGENTS.md` — required ADR sections

This plan converts those two proposal documents into ordered, reviewable pull
requests. It follows the execution idiom already established in
`docs/repository-reorganization.md`: preflight capture, `git mv` with preserved
package names, atomic repair of every path consumer, a validation battery, and a
declared commit shape.

---

## 0. Governing constraints

These come from the repository's own instructions and shape every phase.

1. **Mechanical, semantic, and feature changes never share a pull request.**
   `AGENTS.md` §Package discipline. Each phase below is exactly one of the three.
2. **Accepted ADRs are binding and are amended only by a new ADR.**
   `docs/adr/AGENTS.md`. Superseded decisions are never rewritten in place.
3. **Every ADR needs all nine sections**: status, context, decision,
   consequences, rejected alternatives, validation, operational impact,
   security impact, references.
4. **Documentation must not describe planned behaviour as implemented.**
   `docs/AGENTS.md`.
5. **`docs/reference-functionality-audit.md` is updated *before* a reference's
   role changes.** `AGENTS.md` §Source and license rules.
6. **`git mv` preserves Cargo package names during reorganization.** Renames are
   separate, later, and optional.

### Decisions that block work

| Decision | Blocks | Where discussed |
| --- | --- | --- |
| Fairness / latency model | ADR 0024, `PROTOCOL.md`, Phase 3 acceptor design | hackathon-base-plan §2, §13 |
| Market topology (one shared market vs parallel markets) | Phase 3 session cap, Phase 4 round model | hackathon-base-plan §13 |
| FIX codec: keep the hand-rolled 2,171 lines or adopt a library | Phase 5 `PROTOCOL.md`, conformance harness | audit §6, hackathon-base-plan §13 |
| Reconnect policy: do resting orders survive a session drop | Phase 3 session host, `RULES.md` | hackathon-base-plan §13 |

Phases 0–2 do not depend on any of these. Start there regardless.

### Explicitly *not* in this plan

- **No Cargo package renames.** `simfix-wire`/`simfix-session`/`simfix-mapping`
  keep their names. The audit §4.1 sketched `fix-*` names; on reflection that is
  pure churn with no benefit to a competition and it would collide with rule 6
  above. Revisit only if the FIX codec decision forces a rewrite anyway.
- **No `targets/` top-level directory.** `AGENTS.md` already defines `apps/` as
  the home for deployable entrypoints. Introducing a second concept costs a
  documentation rewrite and buys nothing — Cargo's `default-members` achieves
  the actual isolation goal (Phase 2).
- **No engine module split** until the architecture is settled. Splitting
  `bunting-engine/src/lib.rs` (2,220 lines) is real work with no bearing on
  whether a competition can run. It lands after Phase 4, or never.

---

## Phase 0 — Governance (documentation and CI only)

**Type:** documentation. **Touches no Rust source.** **Blocks: Phases 2, 3, 4.**

Audit finding F1 established that Cloudflare coupling is enforced by CI and
mandated by binding documents. Any code change in Phase 2 fails CI and gets
reverted by the next reader of `AGENTS.md` unless this lands first.

### 0.1 New ADRs

Numbering continues from 0021.

| ADR | Decision | Supersedes / resolves |
| --- | --- | --- |
| **0022** Native competition venue is the primary deployment target | The native server is the venue; Cloudflare becomes a read-only publication wrapper (leaderboard, run archives, snapshots). The engine is host-neutral: no `packages/` crate carries Worker bindings or a browser-JS backend. | `architecture.md` §2 principle 2 ("Plain Worker authority") and principle 3 ("Workers Cache required"); resolves the ADR 0020 ambiguity below |
| **0023** Concurrent multi-participant FIX sessions | One run, N credentialed participants, N concurrent inbound FIX sessions, one authoritative writer serializing commits. Per-session bounds and isolation. | the single-connection / single-participant constraint recorded in `docs/deployment.md` |
| **0024** Fairness and latency model | *Blocked on the organizer decision.* Co-location, gateway normalization, or discrete matching intervals. | new concern; nothing currently addresses it |
| **0025** Run archive and replay verification | Every round emits an immutable archive (scenario, seeds, engine version, command stream, event stream, checksums). `bunting replay` must reproduce it byte-identically. Scoring is a pure function of the archive. | extends ADR 0010 determinism from a property to an enforced gate |
| **0026** Language bindings and the FFI façade | `bunting-rs` gains a concrete non-generic façade; `bunting-ffi`/`bunting-py`/`bunting-cpp` wrap it. Binding crates declare their own `[lints]` tables. | carves out `AGENTS.md` §Package discipline's blanket "workspace metadata/lints" requirement |

**ADR 0022 must resolve a real contradiction, not paper over it.** ADR 0020's
Decision section says "Bunting does not expose an inbound raw TCP listener",
while its Validation section says "no *Worker* route accepts inbound raw TCP" —
and `apps/bunting-server/src/runtime.rs:164` already binds an inbound FIX
listener. The implementation resolved the ambiguity in practice; ADR 0022 must
resolve it in writing: **the native venue is an inbound acceptor; the Worker
remains outbound-only and, after Phase 2, publication-only.** State this
explicitly in Rejected alternatives so the next reader does not re-litigate it.

**ADR 0026 must state the lint carve-out and why.** `unsafe_code = "forbid"`
cannot be relaxed by an inner `allow`, and both PyO3's `#[pymodule]` and
`#[cxx::bridge]` expand to `unsafe`. Without an ADR authorizing per-crate
`[lints]`, a reviewer following `AGENTS.md` correctly rejects the binding crates.

### 0.2 Mark superseded ADRs

Add a `Superseded by:` line to the Status field of ADRs **0004** (FIX over
WebSocket → 0020), **0013** and **0014** (→ 0018/0019), **0015** (→ 0016).

This is metadata only. Do not touch their Context, Decision, or Consequences
text — `docs/adr/AGENTS.md` forbids rewriting accepted history, and a reader
needs the original reasoning intact to understand why it changed.

### 0.3 Reframe the binding prose

| File | Change |
| --- | --- |
| `AGENTS.md` §Mission | "with a plain Cloudflare Worker deployment target" → the native competition venue is primary; Cloudflare is one publication wrapper |
| `AGENTS.md` §Binding architecture decisions | Replace "The deployment target is one native Rust Cloudflare Worker" with the ADR 0022 role split |
| `README.md` line 3 | Drop "designed to run in a plain Cloudflare Worker" |
| `docs/architecture.md` §1, §2 principles 2–3 | Restate per ADR 0022 |
| `docs/deployment.md` | Native venue first; Cloudflare section becomes publication-only |
| `docs/repository-reorganization.md` | Add an architecture note pointing here — the same pattern that file already uses for ADR 0018/0019 |
| `docs/plans/native-rust-trpc-nbc-sprints.md`, `docs/plans/corrected-bunting-implementation-plan.md` | Architecture notes marking superseded targets; do not delete |

### 0.4 Rewrite the CI architecture policy

`.github/workflows/ci.yml`, step *"Enforce architecture dependency policy"*,
currently greps for the exact JS shim lines in the engine manifest. It therefore
**requires** the coupling that Phase 2 removes.

In this phase, reduce it to the assertions that are true today and remain
architecturally meaningful:

- `Cargo.lock` exists;
- `orderbook-rs = { version = "=0.10.3", default-features = false }` in the root
  manifest (a real invariant per ADR 0013/0019);
- keep `cargo tree --locked -p bunting-engine | grep -F 'orderbook-rs v0.10.3'`.

Drop the four greps for `getrandom`, `uuid`, `getrandom_backend`, and
`Cache::default()`. The positive assertions replacing them (§2.5) can only be
added once Phase 2 makes them true — do not add them here.

### 0.5 Validation and commit shape

Documentation-only, so the existing battery must pass unchanged:

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
git diff --check
```

Commits:

1. `docs: add ADRs 0022, 0023, 0025, 0026 for the competition venue`
2. `docs: mark superseded ADRs 0004, 0013, 0014, 0015`
3. `docs: reframe Cloudflare as a publication wrapper`
4. `ci: replace manifest greps with architectural assertions`

**Exit criteria:** a contributor reading `AGENTS.md` cold concludes that the
native venue is primary, and CI no longer requires browser-JS shims in
`packages/bunting-engine/Cargo.toml`.

---

## Phase 1 — Pure deletions

**Type:** mechanical. **No moves, no behaviour change.** Independent of Phase 0;
can run in parallel.

Audit finding F4. Ordered cheapest-first so early commits are trivially
reviewable.

### 1.1 Untracked working-tree cruft

Confirmed untracked (`git ls-files` returns nothing for either):

- `apps/edge-api/` — 1.1 MB of stale Worker build output, no `Cargo.toml`, not a
  workspace member, superseded by `apps/bunting-worker`. Delete.
- `temp/RIT.User.Application-1.8.456.msi`, `temp/RIT2.RTD.API.Link.x64-0.0.15.msi`
  — 9.2 MB of third-party proprietary installers behind
  `docs/research/rit-binary-audit/`. Relocate outside the working tree; record
  where in `docs/research/rit-binary-audit/source-manifest.md`.
- Add `.DS_Store` and `apps/*/build/` to `.gitignore`.

### 1.2 The tRPC oracle

`tests/oracles/trpc/` (Node package pinning `@trpc/client|server` 11.18.0) and
`tests/fixtures/reference/trpc/11.18.0/` (10 fixtures).

Two facts make this clean: `docs/architecture.md` §6 already states tRPC "is no
longer an architecture or runtime dependency", and **it is not referenced by
`.github/workflows/`, `tools/`, or `install.sh`** — the oracle is unexercised.

Delete both, then update `architecture.md` §6 to drop the "development-only
differential record against pinned tRPC fixtures" sentence in the same commit.
This removes Node from the toolchain entirely.

### 1.3 Placeholder source

`packages/bunting-engine/src/compatibility/nbc/translation.rs` is 2 lines.
Delete it and its `mod` declaration in `compatibility/nbc/mod.rs`.

### 1.4 Reference submodules that only mirror published crates

`rand`, `slotmap`, `intrusive-rs`, `postcard`, `proptest`, `wirefilter`, `cqrs`,
`nexosim` — readable on docs.rs, no unique evidence value.

Order matters here, per `AGENTS.md` §Source and license rules:

1. **First** update `docs/reference-functionality-audit.md`,
   `docs/reference-adoption.md`, and `docs/reference-inventory.md` to record the
   disposition change, keeping each upstream URL and pinned commit in the text
   so the evidence trail survives the removal.
2. Then `git submodule deinit <path>`, `git rm <path>`, and remove the stanza
   from `.gitmodules`.
3. Retain the remaining references and add the track mapping from
   hackathon-base-plan §9 to the audit document.

Do not touch `orderbook-rs`, `pricelevel`, `nbc_engine`, `nbc-hft-simulation`,
`quarcc-trading-engine`, `workers-rs`, `quickfixj`, `liquibook`,
`exchange-core`, `abides`, `market-maker-rs`, `ritc_mm`, `ferrumfix`, or
`nautilus-trader` in this phase. The four-way FIX-reference duplication is
resolved by the codec decision, not here.

### 1.5 Validation and commit shape

Full battery, plus:

```bash
git submodule status          # only intended submodules remain
git grep -n 'oracles/trpc'    # no active references
git grep -n 'edge-api'        # only historical ADR/plan text
```

Commits:

1. `chore: remove stale edge-api build output and ignore generated paths`
2. `chore: remove unexercised tRPC conformance oracle`
3. `chore: remove empty NBC translation placeholder`
4. `docs: record reference disposition changes before submodule removal`
5. `chore: deregister dependency-mirror submodules`

**Exit criteria:** clone size drops materially; Node leaves the toolchain; every
`git grep` for a removed path returns only intentionally historical text.

---

## Phase 2 — Cloudflare becomes a wrapper

**Type:** mechanical moves plus manifest surgery. **Requires Phase 0.**

Audit finding F1, hackathon-base-plan §9. No runtime behaviour changes; the
Worker must still build and serve.

### 2.1 Isolate Cloudflare from default workspace operations

The idiomatic fix, and the one that actually solves the stated problem: add
`default-members` to the root `[workspace]`, listing every member **except** the
Cloudflare crates.

```toml
[workspace]
members = [ ... ]                    # unchanged, everything stays a member
default-members = [ ... ]            # everything except the Worker crates
```

Bare `cargo build`, `cargo test`, and `cargo clippy` then skip Cloudflare
entirely, while `cargo build -p bunting-worker` and the Worker CI job continue to
work. This removes the `worker` crate from the native development loop without
moving a single file or introducing a second workspace.

### 2.2 Relocate the platform adapter

```bash
git mv packages/worker-cache apps/bunting-worker/worker-cache
```

Preserve the Cargo package name `bunting-worker-cache` (rule 6). Then update:

- root `Cargo.toml` `members` and `default-members`;
- `apps/bunting-worker/Cargo.toml` path dependency;
- **`packages/AGENTS.md`** — delete the sentence "`packages/worker-cache` is a
  platform adapter and may depend on `workers-rs`." It is the scoped
  authorization for exactly the coupling being removed, and leaving it invites
  the next contributor to reintroduce a Worker-dependent package;
- `packages/worker-cache/AGENTS.md` if present, moved with the directory.

### 2.3 De-contaminate the engine manifest

From `packages/bunting-engine/Cargo.toml`, delete:

```toml
getrandom = { version = "=0.3.4", features = ["wasm_js"] }
uuid = { version = "=1.23.4", features = ["js"] }
```

Neither identifier appears in `packages/bunting-engine/src` (F1). They exist to
feature-unify browser-JS backends for transitive dependencies of `orderbook-rs`
and `pricelevel`. Move both declarations to `apps/bunting-worker/Cargo.toml`,
which is the only crate that genuinely needs a JS backend.

The pinned `orderbook-rs` graph enables UUID v4 without selecting UUID's
portable adapter, so the engine retains a host-neutral direct
`uuid = { version = "=1.23.4", features = ["rng-getrandom"] }` feature
unification. This replaces the `js` feature; it does not restore a browser
backend.

### 2.4 Scope the Wasm rustflag

Replace the root browser-specific cfg with the explicit host-neutral
`getrandom_backend="unsupported"` backend for `wasm32-unknown-unknown`, and
move `rustflags = ["--cfg", 'getrandom_backend="wasm_js"']` into
`apps/bunting-worker/.cargo/config.toml`. The compile-only host-neutral target
has no entropy API; deployments that need entropy must select one explicitly.

Cargo reads `.cargo/config.toml` from the invocation directory upward, and
`worker-build` runs from `apps/bunting-worker` (per `wrangler.toml` `[build]`),
so the Worker build keeps the flag while every other `wasm32-unknown-unknown`
consumer is freed from a browser-JS assumption.

Verify explicitly — this is the step most likely to break quietly:

```bash
cd apps/bunting-worker && worker-build --release --no-panic-recovery
```

### 2.5 Delete the relay

`apps/bunting-server/src/relay.rs` (9.5 KB) exists solely because Workers cannot
accept inbound TCP. Under ADR 0022 the Worker no longer pretends to be the
venue, so the workaround is dead code.

Remove, atomically:

- `apps/bunting-server/src/relay.rs` and its `pub mod relay;` in `lib.rs`;
- the `DeploymentProfile::Cloudflare` branch in `apps/bunting-server/src/main.rs`;
- the `relay` field and `DeploymentProfile::Cloudflare` variant in `config.rs`;
- `apps/bunting-server/config/cloudflare.json`;
- the `bunting relay` subcommand in `apps/bunting-cli`;
- `bunting relay` from `README.md`, `docs/deployment.md`, and the CLI's scoped
  `AGENTS.md`.

Note `.github/workflows/release.yml` copies `apps/bunting-server/config/*.json`
into the release archive — removing `cloudflare.json` silently changes the
release payload. Confirm the archive contents in the same PR.

### 2.6 Add the real CI assertions

Now that the invariant holds, replace what §0.4 deleted with assertions that
express it:

```bash
# the engine must not reach Cloudflare on any target
! cargo tree --locked -p bunting-engine | grep -q '^worker '
# the engine must build without a browser-JS backend
cargo check --locked -p bunting-engine --target wasm32-unknown-unknown
# native default workspace must not compile Cloudflare bindings
! cargo tree --locked | grep -q '^worker '
```

Update the `dorny/paths-filter` `worker` filter to the new paths
(`apps/bunting-worker/**` now includes `worker-cache`).

### 2.7 Validation and commit shape

Full battery, plus the Workerd gate already in CI, plus:

```bash
cargo metadata --locked --format-version 1 --no-deps   # members: paths only
cargo tree --locked -p bunting-engine                  # no `worker`
cargo check --locked --workspace --target wasm32-unknown-unknown
git grep -n 'packages/worker-cache'                    # no active hits
git grep -n 'relay'                                    # no active hits
```

Commits:

1. `chore: exclude Cloudflare crates from default workspace members`
2. `chore: move worker-cache under the Worker app`
3. `refactor: remove browser-JS shims from the engine manifest`
4. `chore: scope the wasm_js rustflag to the Worker`
5. `refactor: remove the Cloudflare FIX relay`
6. `ci: assert engine host-neutrality`

**Exit criteria:** `cargo tree -p bunting-engine` contains no Cloudflare crate;
bare `cargo test` compiles no Cloudflare code; the Workerd smoke gate still
passes.

---

## Phase 3 — The concurrent venue

**Type:** feature. **Requires Phase 0 (ADR 0023) and the latency decision (ADR
0024).** Largest phase; hackathon-base-plan §1–§2.

This is the phase that decides whether a competition can happen. Split it into
its own sequence of PRs, not one.

### 3.1 Consolidate on one I/O model

`apps/bunting-tui/src/local_market.rs` (1,052 lines) is a working async `tokio`
FIX acceptor calling `bunting-engine` in-process — `TcpListener::bind` at 116,
`serve_connections` at 120, `serve` at 131. `apps/bunting-server/src/runtime.rs`
uses blocking `std::net` at 164 and 819 with no `tokio` in `[dependencies]`.

Promote the async acceptor into `bunting-server`; delete the blocking one. This
is primarily relocation of working code, which is why it goes first.

**Do not touch `simfix-wire` or `simfix-session`.** Their sans-I/O design
(`receive_bytes`, `poll`, `SessionAction`, `SessionSnapshot`, `restore`) is
exactly what makes this a transport change rather than a protocol rewrite.

### 3.2 Split `runtime.rs`

937 lines currently holding the FIX acceptor, connection handling, scenario
bootstrap, operator commands, the `RuntimeHost` impl, a hand-written HTTP/1
admin server, and hand-rolled calendar arithmetic. Split into:

| Module | Owns |
| --- | --- |
| `acceptor.rs` | listener, connection lifecycle, per-session bounds |
| `session_host.rs` | one participant session: `FixSession` + `FixApplicationState` |
| `writer.rs` | the single authoritative committer serializing commands |
| `scenario.rs` | run bootstrap |
| `admin.rs` | the operator/admin surface |

Delete `civil_from_days` (923) and `fix_timestamp` (908) in favour of `time 0.3`,
already in the lockfile via the TUI.

### 3.3 Multi-session and roster

- one run, N credentialed participants, each with starting cash and inventory;
- bounded session cap with an explicit rejection naming the limit;
- per-session isolation: one team's disconnect, flood, or malformed message
  must not affect another session or the run;
- reconnect and resume via the existing `SessionSnapshot`/`restore`;
- one authoritative writer; participants' commands serialize through it.

### 3.4 Fairness enforcement

Per ADR 0024. Regardless of the model chosen:

- per-session message-rate and open-order caps, enforced at the session
  boundary, rejecting with the field and the limit named;
- `simfix-wire::WireLimits` (line 91) and the `orderbook-rs` per-book notional /
  open-order / price-band controls (`architecture.md` §5, §12) are the existing
  primitives — wire them per participant rather than per book;
- symmetric market data: identical depth and cadence for every session.

### 3.5 Feature-gate the TUI

`apps/bunting-cli` depends on `bunting-tui` unconditionally, so `bunting server`
links `ratatui`, `crossterm`, `rustls`, `ring`, and the `windows-*` families —
roughly half the 243-crate graph (audit §2.3). Add a default-off `tui` feature.

Keep the TUI itself: under hackathon-base-plan §5 it is the operator and
spectator console, which is a promotion, not a deprecation. Only the *venue
binary's* dependency on it goes away.

### 3.6 Validation

Beyond the standard battery:

- N concurrent QuickFIX-Go clients trade against one run and see each other's
  liquidity — extend `tests/interop/quickfixgo`, which already proves the
  single-client path;
- killing one session mid-order leaves other sessions and the run sequence
  intact;
- a reconnecting session resumes its sequence and open orders per the chosen
  reconnect policy;
- rate-limit rejects name the limit;
- `cargo tree -p bunting-cli --no-default-features` shows no `ratatui`.

**Exit criteria:** N teams trade concurrently on one market with published,
enforced limits.

---

## Phase 4 — Audit and operator surface

**Type:** feature. **Requires Phase 3.** hackathon-base-plan §3–§4.

### 4.1 Run archive and replay

Generalize `tests/goldens/competition-full-run.v1.json` into the archive format
rather than inventing one: scenario id and version, engine version, seed set,
the ordered accepted-command stream with arrival sequence, the canonical event
stream, and final checksums.

```bash
bunting replay <archive>   # exit 0 = reproduced byte-identically
bunting score  <archive>   # -> per-participant scores
bunting judge  <archives>  # -> leaderboard.json + leaderboard.html
```

`origin-store` and `command-transaction` already record everything required;
this phase is largely serialization plus a CLI surface.

Add `bunting replay` over the golden archive to CI. It becomes a permanent
determinism gate protecting ADR 0010 — any wall-clock leak, unseeded RNG, or
iteration-order dependence fails it.

### 4.2 Operator surface

Per hackathon-base-plan §4: roster management with credential export; round
lifecycle (arm, start, pause, resume, halt, end, reset) built on
`IterationId`; scheduled and on-command news/tender injection extending the
existing `operator_command()` and `OpenTenderPayload`; and one clean panic
button that halts, commits, and leaves a replayable archive.

Resolve the admin-transport question here rather than letting it drift: either
freeze the hand-written HTTP/1 surface (`write_http`, `runtime.rs:882`) and
drive operations from the CLI against the origin store, or accept one small
framework. Decide before event prep, not during it.

### 4.3 Cloudflare publication wrapper

The read-only counterpart to ADR 0022: publish archives, the leaderboard, and
book snapshots from the Worker. `bunting-worker-cache`'s immutable
content-addressed storage is already the right shape. No new authority path.

---

## Phase 5 — Competitor pack

**Type:** feature plus documentation. **Requires Phase 3; needs the FIX codec
decision.** hackathon-base-plan §7–§8, §10–§11.

1. **`PROTOCOL.md` generated from
   `schemas/fix/bunting.fixlatest.competition.v1.orchestra.xml`** — it already
   exists; do not hand-write a second source of truth.
2. **Sample clients.** Python first. `tests/interop/quickfixgo/interop_test.go`
   is already a working sample client in disguise — publish it as one.
3. **Practice venue from prebuilt binaries.** `install.sh` and
   `release.yml` already ship four platforms. Document `git clone` *without*
   `--recursive` as the supported path — CI already runs `submodules: false`,
   but no document says so, and a recursive clone still pulls the remaining
   reference tree.
4. **`SCORING.md`** with the scoring function published up front and
   `bunting score` runnable locally.
5. **`RULES.md`** covering limits, the fairness model, and the reconnect policy.
6. **`RUNBOOK.md`** for organizers: venue setup, roster creation, running a
   round, mid-round failure recovery, publishing results, settling a dispute by
   replay.
7. **Baseline agents and round scenarios** — hackathon-base-plan §6, §10. Mostly
   data plus existing `bunting-agents` policies.
8. **Documentation split by audience.** Competitor-facing and organizer-facing
   documents at the top level; everything currently in `docs/` moves to
   `docs/internals/`. `AGENTS.md` stays where it is and keeps full authority
   over the engine tree.
9. **`bunting doctor` and `bunting conformance --agent <cmd>`** —
   hackathon-base-plan §8. Buy this before setup day.

### 5.1 Onboarding SLA as a CI gate

Extend the existing zero-configuration smoke gate into a timed contract: fresh
clone → venue running → order filled → score printed, in three commands, under
a declared budget, failing CI on regression. An untested onboarding promise
decays within a month.

---

## Phase 6 — Language bindings

**Type:** feature. **Requires Phase 0 (ADR 0026).** Audit §4.4.

Deliberately last. FIX over TCP is already a binding for every language a
competitor might use, and the competition runs on it. These matter for practice
tooling, scoring, and embedding — not for letting teams compete.

1. **Concrete façade in `bunting-rs`.** Today it is 23 lines re-exporting
   `ApplicationService<'a, O, C>`, which is generic over `OriginStore` and
   `SnapshotCache` and cannot cross an FFI boundary. Add a concrete handle
   owning a chosen store and cache. This is the prerequisite for all three
   bindings and it finally makes `bunting-rs` the real Rust binding rather than
   name-only indirection.
2. **`bindings/bunting-ffi`** — C ABI, `crate-type = ["cdylib", "staticlib"]`,
   opaque handle, error out-parameters, explicit frees, `bunting.h` via
   cbindgen. One unsafe boundary.
3. **`bindings/bunting-py`** — PyO3 with `crate-type = ["cdylib", "rlib"]` and
   the `abi3` feature so a single wheel covers supported Python versions; built
   and published with maturin. Wrap the façade directly rather than the C
   header, so exceptions map properly and the GIL can be released around engine
   calls.
4. **`bindings/bunting-cpp`** — `#[cxx::bridge]` with the handle as an opaque
   `extern "Rust"` type and orders/fills as shared structs; `cxx-build` in
   `build.rs`; a CMake package config so `ref/quarcc-trading-engine/engine-cpp`
   can link it unmodified.
5. **Cross-binding contract test** — replay one golden archive through Rust,
   Python, and C++ and assert byte-identical canonical event streams. This is
   what keeps three bindings honest without tripling the test surface.

Each binding crate declares its own `[lints]` table per ADR 0026.

---

## Cross-cutting best practices to adopt

Not phase-specific; fold each into whichever phase touches the same files.

| Practice | Why here | Phase |
| --- | --- | --- |
| `default-members` to isolate deployment targets | Removes Cloudflare from the native loop without a second workspace | 2 |
| Default-off `tui` feature | Halves the venue binary's dependency graph | 3 |
| Graph-based CI assertions instead of manifest greps | The current greps enforce the coupling being removed; a `cargo tree` assertion states the actual invariant | 0, 2 |
| `cargo deny` for advisories and licences | 243 external crates and a repository that already takes licence provenance seriously; `AGENTS.md` §Source and license rules is currently enforced by review alone | 1 or 2 |
| Fully pinned dev environment (devcontainer or Nix) | `rust-toolchain.toml` pins the compiler; nothing pins Go, `workerd`, `worker-build`, or `wrangler` | 5 |
| Keep workspace lints strict; carve out only bindings | `unsafe_code = "forbid"`, `unwrap_used`/`expect_used`/`panic` denied, clippy pedantic is a genuine asset for a venue whose scores must be defensible | 6 |
| Replay in CI as the determinism gate | Converts ADR 0010 from a stated property into an enforced one | 4 |
| Timed onboarding gate | The only way an onboarding promise stays true | 5 |

---

## Summary sequence

| Phase | Type | Gated on | Outcome |
| --- | --- | --- | --- |
| 0 Governance | docs + CI | — | Native venue is the documented primary target; CI stops requiring JS shims in the engine |
| 1 Deletions | mechanical | — | Node leaves the toolchain; clone shrinks; dead paths gone |
| 2 Cloudflare wrapper | mechanical | 0 | Engine is host-neutral and provably so |
| 3 Concurrent venue | feature | 0, latency decision | N teams trade on one market |
| 4 Audit + operator | feature | 3 | Rounds can be run, scored, and defended by replay |
| 5 Competitor pack | feature + docs | 3, codec decision | Teams can practise locally and self-verify |
| 6 Bindings | feature | 0 | Python, C++, and a real Rust embedding API |

Phases 0 and 1 are independent and can start immediately; neither is blocked on
any open decision. Phases 0–2 are the reorganization proper. Phases 3–4 are the
difference between an engine and a venue. Phases 5–6 are the difference between
a venue and a competition someone else can run.
