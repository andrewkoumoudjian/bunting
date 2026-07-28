# Bunting streamlining audit

Status: exploration report, not an accepted ADR. Produced by reading the
workspace manifests, `cargo metadata --locked`, source under `packages/`,
`apps/`, `bunting-rs/`, the CI workflow, and every binding document under
`docs/`. Evidence markers follow `AGENTS.md`: **observed** is proved by a file,
manifest, resolved dependency graph, or CI step; **inferred** is reasoning on
top of that; **Bunting-added** is a new proposal.

Goal being served (owner input): shed unnecessary features, keep one
host-neutral engine, make the inbound FIX/TCP server the primary product
surface, provide Rust / Python / C++ bindings, and reduce Cloudflare to one
deployment wrapper that does not modify the engine.

---

## 1. What the repository is today

**observed** — 27,855 lines of first-party Rust across 24 workspace members,
one `Cargo.lock`, edition 2024, toolchain pinned to 1.88.0.

| Layer | Members | LOC |
| --- | --- | --- |
| Core domain | `market-types`, `market-events`, `ledger`, `risk-engine` | 1,702 |
| Engine | `bunting-engine` (incl. `compatibility/nbc`) | 4,955 |
| Orchestration | `origin-store`, `command-transaction`, `bunting-application` | 1,959 |
| Participant side | `quarcc-execution-engine`, `quarcc-bunting-adapter`, `quarcc-execution-wasm`, `bunting-agents`, `bunting-runtime` | 3,950 |
| FIX | `simfix-wire`, `simfix-session`, `simfix-mapping` | 2,171 |
| Contract / transport | `bunting-api-contract`, `browser-wire`, `worker-cache` | 1,097 |
| Composition | `bunting-rs` | 23 |
| Apps | `bunting-tui`, `bunting-worker`, `bunting-server`, `bunting-cli` | 10,912 |

Two facts stand out immediately. The terminal UI (`apps/bunting-tui`, 6,251
lines) is the largest single component in the repository — larger than the
engine. The declared composition boundary (`bunting-rs`, 23 lines) is the
smallest.

---

## 2. Complete dependency inventory

### 2.1 Resolved graph

**observed** — `cargo metadata --locked` resolves **243 distinct external
crates** for the host target.

### 2.2 Direct external dependencies

Workspace-level (`Cargo.toml` `[workspace.dependencies]`):

| Crate | Version | Role |
| --- | --- | --- |
| `orderbook-rs` | `=0.10.3`, `default-features = false` | CLOB matching (ADR 0013/0019) |
| `pricelevel` | `=0.8.4` | price-level primitives under `orderbook-rs` |
| `rustyfix-dictionary` | `=0.7.4` (`fix50sp2`, `fixt11`) | FIX dictionary metadata only — **not** a codec |
| `serde` | `1` (derive) | serialization |
| `serde_json` | `1` | JSON |
| `sha2` | `0.11` | checksums, id namespacing |
| `worker` | `=0.8.5` | Cloudflare Workers runtime |
| `wasm-bindgen` | `0.2` | JS/Wasm boundary |
| `serde-wasm-bindgen` | `0.6` | JS value conversion |
| `percent-encoding` | `2` | browser query decoding |
| `tokio` | `1` (`io-util`) | async I/O |

Crate-local direct dependencies not declared at workspace level:

| Crate | Consumer | Notes |
| --- | --- | --- |
| `getrandom` `=0.3.4` (`wasm_js`) | `bunting-engine` | **unused in source** — see F1 |
| `uuid` `=1.23.4` (`js`) | `bunting-engine` | **unused in source** — see F1 |
| `clap` `4.6.1` | `bunting-cli`, `bunting-tui` | argument parsing |
| `ratatui` `0.30.2` (crossterm only) | `bunting-tui` | terminal UI |
| `crossterm` `0.29.0` (`event-stream`) | `bunting-tui` | terminal backend |
| `colored` `2` | `bunting-tui` | ANSI colour |
| `unicode-width` `0.2.2` | `bunting-tui` | glyph widths |
| `time` `0.3.47` | `bunting-tui` | timestamps |
| `futures-util` `0.3` | `bunting-tui` | stream combinators |
| `rustls` `0.23` (`ring`,`std`,`tls12`) | `bunting-tui` | TLS |
| `rustls-native-certs` `0.8`, `rustls-pemfile` `2`, `tokio-rustls` `0.26` | `bunting-tui` | TLS support |

### 2.3 Where the 243 crates come from

**inferred, from the resolved graph** — the engine's own closure is small:
`bunting-engine` needs `serde`, `orderbook-rs`, `pricelevel`, `sha2`, `uuid`,
plus `orderbook-rs`'s internals (`crossbeam*`, `dashmap`, `parking_lot`,
`rand`, `tracing*`, `thiserror`, `chrono`, `ulid`, `derive_more`). Roughly half
the graph is attributable to `apps/bunting-tui`: the `ratatui` cluster
(`palette`, `kasuari`, `line-clipping`, `compact_str`, `castaway`,
`unicode-truncate`, `instability`, `strum`, `lru`, `indoc`, `itertools`), the
TLS cluster (`ring`, `rustls*`, `security-framework`, `schannel`,
`openssl-probe`), full `tokio` with `mio`/`socket2`/`signal-hook`, and the
`windows-*` / `winapi` target families.

`apps/bunting-cli` depends on `bunting-tui` unconditionally, so the shipped
`bunting` binary — including `bunting server` — links the entire terminal UI
and TLS stack.

### 2.4 Non-Rust toolchain dependencies

| Tool | Version | Where | Purpose |
| --- | --- | --- | --- |
| Go | 1.24.x | `tests/interop/quickfixgo` | `github.com/quickfixgo/quickfix v0.9.10` FIXT.1.1 / FIX 5.0 SP2 interop gate |
| Node | — | `tests/oracles/trpc` | `@trpc/client`+`@trpc/server` `11.18.0` fixture oracle |
| `worker-build` | `0.8.5` | CI, `wrangler.toml` `[build]` | Worker bundling |
| `workerd` | `1.20260716.1` | CI, `apps/bunting-worker/workerd/` | Worker runtime gate |
| `wrangler` | via `npx` | deployment | D1 migrations, deploy, secrets |

### 2.5 `ref/` evidence tree

**observed** — 25 Git submodules plus three checked-in trees (`nbc_engine`,
`ritc_mm`, `quarcc-trading-engine`), **483 MB** on disk.

Load-bearing: `orderbook-rs`, `pricelevel` (production deps),
`nbc_engine` + `nbc-hft-simulation` (compatibility source),
`quarcc-trading-engine` (port source), `workers-rs` (Cloudflare).

Redundant with crates.io/docs.rs and carrying no unique evidence value:
`rand`, `slotmap`, `intrusive-rs`, `postcard`, `proptest`, `wirefilter`,
`cqrs`, `nexosim`.

Five FIX implementations are held as evidence (`ferrumfix`, `ironfix`,
`fixer`, `quickfixj`, plus `rustyfix` as a dependency) while the repository
hand-rolls 2,171 lines of FIX using only `rustyfix`'s dictionary crate.

---

## 3. Findings

### F1 — Cloudflare is not a wrapper; it is a load-bearing workspace constraint

This is the finding that most directly contradicts the stated goal.

**observed:**

1. `packages/bunting-engine/Cargo.toml` declares
   `getrandom = { version = "=0.3.4", features = ["wasm_js"] }` and
   `uuid = { version = "=1.23.4", features = ["js"] }`. Neither identifier
   appears anywhere in `packages/bunting-engine/src` — grep is clean. Both
   exist purely to feature-unify browser-JS backends for transitive
   dependencies of `orderbook-rs`/`pricelevel`. `cargo tree -p bunting-engine
   -i uuid` confirms `uuid` already arrives through `orderbook-rs` and
   `pricelevel`. The core matching engine's manifest therefore encodes a
   browser-JS host assumption it never uses.
2. Root `.cargo/config.toml` applies
   `rustflags = ["--cfg", 'getrandom_backend="wasm_js"']` to **all** of
   `wasm32-unknown-unknown`. Any non-JS Wasm consumer — wasip1/p2, wasmtime,
   a standalone embedding — inherits a JS randomness backend from the
   workspace root.
3. `packages/worker-cache` is a root workspace member that depends on the
   `worker` crate. `cargo clippy --workspace` and `cargo test --workspace`
   therefore compile Cloudflare bindings during an ordinary native loop.
4. `.github/workflows/ci.yml`, step *"Enforce architecture dependency
   policy"*, hard-greps for those exact manifest lines, for
   `worker = "=0.8.5"` in the root manifest, and for `Cache::default()` inside
   `packages/worker-cache/src/lib.rs`. **Decoupling Cloudflare from the engine
   currently fails CI by design.**
5. The documents make it binding, not incidental: `AGENTS.md` line 5 ("with a
   plain Cloudflare Worker deployment target"), `README.md` line 3 ("designed
   to run in a plain Cloudflare Worker"), `docs/architecture.md` §1 (same) and
   §2 principle 2 ("**Plain Worker authority**"), and `AGENTS.md` §Binding
   architecture decisions ("The deployment target is one native Rust
   Cloudflare Worker").

**inferred:** code-only decoupling will be reverted by the next contributor or
agent that reads `AGENTS.md`. The document layer has to move first.

### F2 — The inbound FIX/TCP server is the secondary target, and there are two divergent I/O stacks

**observed:**

- Cloudflare Workers cannot accept inbound raw TCP. ADR 0020 and
  `docs/architecture.md` §6 therefore restrict the Worker to *outbound*
  FIX initiation, and inbound participant connectivity requires either
  `apps/bunting-server` (native) or `bunting relay`
  (`apps/bunting-server/src/relay.rs`) proxying participant ↔ Worker.
- `apps/bunting-server` uses **blocking** `std::net` — `TcpListener::bind` at
  `runtime.rs:164` (FIX) and `runtime.rs:819` (admin), plus `relay.rs:67,69`.
  It has no `tokio` in `[dependencies]`, only `[dev-dependencies]`.
- `apps/bunting-tui` uses **async** `tokio::net::TcpListener`
  (`local_market.rs:116`) with `tokio-rustls`.
- `docs/deployment.md` states each server process "accepts at most one FIX
  connection", binds one immutable participant/run, and that the native origin
  store is "intentionally single-writer" — "use the Worker/D1 deployment for
  multi-instance authority."
- `apps/bunting-server/src/runtime.rs` is 37 KB / ~940 lines and holds the FIX
  acceptor, connection handling, scenario bootstrap, operator commands, the
  `RuntimeHost` impl, a hand-written HTTP/1 admin server (`write_http`, line
  882), and hand-rolled calendar arithmetic (`civil_from_days`, line 923)
  despite `time 0.3` already being in the lockfile.

**inferred:** for "a server that can be connected by TCP for FIX", the single
largest functional gap is concurrent multi-participant sessions. Everything
about the current native server — one connection, one participant, one writer,
blocking sockets — treats it as a test fixture rather than the product.

### F3 — There are no Python or C++ bindings, and no C ABI of any kind

**observed:** grep across `packages/`, `apps/`, `bunting-rs/`, and all
manifests for `extern "C"`, `pyo3`, `maturin`, `cbindgen`, `cxx::bridge`, and
`uniffi` returns **zero hits**. The only non-Rust binding is
`packages/quarcc-execution-wasm` (92 lines, `wasm-bindgen`), and it wraps the
*participant* execution engine, not the market engine.

The nominal Rust binding, `bunting-rs`, is 23 lines of `pub use`. It re-exports
`ApplicationService<'a, O, C>`, which is generic over `OriginStore` and
`SnapshotCache` — not expressible across an FFI boundary without a concrete
façade. `apps/bunting-tui` and `apps/bunting-server` both bypass `bunting-rs`
and depend on `packages/*` directly, so the declared
`packages -> bunting-rs -> apps` flow in `AGENTS.md` §Package discipline is not
actually enforced.

**observed, and a hard blocker:** root `Cargo.toml` sets
`[workspace.lints.rust] unsafe_code = "forbid"`. `forbid` cannot be relaxed by
an inner `allow`, and both PyO3's `#[pymodule]`/`#[pyclass]` and
`#[cxx::bridge]` expand to `unsafe` code. Any binding crate must therefore
declare its own `[lints]` table instead of `workspace = true` — which
`AGENTS.md` §Package discipline currently mandates for every package.

`ref/quarcc-trading-engine` already contains `engine-cpp/` (CMake, `include/`,
`src/`, `tests/`) and `python_client/` (`client.py`, `grpc_interface.py`,
`strategy.py`) — the intended shape of both surfaces exists as reference, with
no Rust-side counterpart.

### F4 — Shedding candidates

| Item | Size / evidence | Assessment |
| --- | --- | --- |
| `apps/edge-api/` | 1.1 MB, only `build/index_bg.wasm` + shim; no `Cargo.toml`; not a workspace member; untracked | Dead build output of a Worker superseded by `apps/bunting-worker`. Referenced only in historical docs. Delete. |
| `temp/*.msi` | 9.2 MB, `RIT.User.Application-1.8.456.msi`, `RIT2.RTD.API.Link.x64-0.0.15.msi`; untracked | Third-party proprietary installers behind `docs/research/rit-binary-audit`. Move out of the working tree. |
| `tests/oracles/trpc/` | Node package pinning `@trpc/*` `11.18.0`, 10 fixture JSONs under `tests/fixtures/reference/trpc/` | `docs/architecture.md` §6 already states tRPC "is no longer an architecture or runtime dependency". An entire Node toolchain is retained for a superseded protocol. |
| `ref/` dependency mirrors | `rand`, `slotmap`, `intrusive-rs`, `postcard`, `proptest`, `wirefilter`, `cqrs`, `nexosim` | Published crates readable via docs.rs. No unique evidence value. |
| `ref/` FIX duplicates | `ferrumfix`, `ironfix`, `fixer`, `quickfixj` | Keep at most one plus `quickfixj` (needed for the Go/Java interop semantics). Four Rust FIX libraries as evidence for hand-rolled code is not a decision, it is a deferred decision. |
| `bunting-rs` | 23 lines, bypassed by both native apps | Either becomes the real embedding API (§4.2) or is deleted. Today it is name-only indirection. |
| `compatibility/nbc/translation.rs` | 2 lines | Empty placeholder; `AGENTS.md` prohibits placeholder structure for future ideas. |
| Docs | 20+ top-level docs, 21 ADRs, plus `docs/plans/`, `docs/prompts/`, `docs/claude/` | ADRs 0013/0014 superseded by 0018/0019, 0015 by 0016, 0004 (FIX over WebSocket) by 0020. Superseded ADRs are not marked as such in their filenames or, in several cases, their headers. `docs/claude/` and `docs/prompts/` are session artifacts. |
| `.DS_Store` | 5 tracked under `ref/` (inside submodules), several untracked at top level | Add to `.gitignore`; remove the untracked ones. |

### F5 — Structural findings inside the kept code

**observed:**

- `packages/bunting-engine/src/lib.rs` is 2,220 lines and `simulation.rs` is
  1,326. The "single production engine" is two god-modules plus a 543-line
  matching adapter. The NBC compatibility split (881 lines across
  `compatibility/nbc/*`) is, by contrast, clean and provenance-linked.
- `packages/bunting-application` describes itself as
  "Transport-neutral Bunting application and transaction service" yet depends
  on `simfix-mapping` and `simfix-wire` and exposes `FixApplicationState`,
  `FixApplicationRequest`, and `map_message`. FIX is embedded in the layer that
  claims protocol neutrality.
- The workspace lint set is genuinely strong and worth preserving:
  `unsafe_code = "forbid"`, `unused_must_use = "deny"`, clippy `all` +
  `pedantic` at warn, `unwrap_used`/`expect_used`/`panic` at deny,
  `float_arithmetic` at warn. Keep it as the default; carve out only the
  binding crates.
- `simfix-wire` / `simfix-session` are correctly sans-I/O (`receive_bytes`,
  `poll`, `SessionAction`, `SessionSnapshot`, `restore`). This is the right
  design and it is what makes a transport rework cheap.

---

## 4. Proposed direction

### 4.1 Target structure

```
packages/                    host-neutral; no crate here may name `worker`,
                             wasm-bindgen, or a JS backend
  market-types  market-events  ledger  risk-engine
  bunting-engine             (split lib.rs / simulation.rs by responsibility)
  origin-store  command-transaction
  bunting-application        (protocol-neutral again)
  fix-wire  fix-session  fix-mapping  fix-application
  quarcc-execution-engine  quarcc-bunting-adapter
  bunting-agents  bunting-runtime
  bunting-api-contract

bunting-rs/                  THE stable embedding API: concrete façade, no
                             generics across the boundary

bindings/
  bunting-ffi/               C ABI, cdylib+staticlib, cbindgen -> bunting.h
  bunting-py/                PyO3 + maturin -> `bunting` wheel
  bunting-cpp/               cxx::bridge + CMake package config
  bunting-wasm/              wasm-bindgen (browser/JS)   [own [lints]]

targets/
  bunting-server/            PRIMARY: inbound FIX/TCP acceptor, multi-session
  bunting-cli/               server | tui | relay | init | version
  bunting-tui/               operator workstation (feature-gated out of `server`)
  cloudflare/                WRAPPER ONLY
    worker/                  worker crate, D1 origin, FixSessionObject, relay
    worker-cache/            moved from packages/
    .cargo/config.toml       wasm_js rustflag scoped here, not at root
```

The invariant to enforce mechanically: **no crate under `packages/` may appear
in the same dependency graph as the `worker` crate, and none may require a JS
backend cfg.**

### 4.2 Making Cloudflare a wrapper (F1)

Ordered, because step 1 gates the rest:

1. **Documents first.** New ADR superseding `architecture.md` §2 principle 2
   ("Plain Worker authority"). Rewrite `AGENTS.md` §Mission, `README.md` §1,
   `architecture.md` §1–2 to: *the engine is host-neutral; deployment targets
   are the native FIX/TCP server (primary), the CLI/TUI, the C/Python/C++
   bindings, and the Cloudflare Worker (one wrapper among several).* Without
   this, the code change is reverted by the next reader of `AGENTS.md`.
2. **Rewrite the CI policy step.** Replace manifest greps with graph
   assertions that express the actual invariant:
   - `worker` must **not** appear in `cargo tree -p bunting-engine` for any
     target;
   - `bunting-engine` must build for native, `wasm32-unknown-unknown`, **and**
     `wasm32-wasip2` with no JS cfg;
   - the pinned `orderbook-rs v0.10.3` assertion stays (it is a real
     architectural invariant, unlike the JS shims).
3. **Delete `getrandom` and `uuid` from `packages/bunting-engine/Cargo.toml`.**
   Move the wasm-JS feature unification into `targets/cloudflare/worker`, the
   only crate that actually needs a JS backend. Verify with
   `cargo tree -p bunting-worker --target wasm32-unknown-unknown`.
4. **Scope the rustflag.** Move `getrandom_backend="wasm_js"` from root
   `.cargo/config.toml` into `targets/cloudflare/worker/.cargo/config.toml`.
   Cargo reads `.cargo/config.toml` from the invocation directory upward, and
   `worker-build` runs from the Worker directory, so this keeps the Worker
   building while freeing every other Wasm consumer.
5. **Move `packages/worker-cache`** into the Cloudflare target (187 lines, one
   consumer). It stops appearing in native `--workspace` builds.
6. **Decide the relay's fate.** `relay.rs` exists only because Workers cannot
   accept inbound TCP. If the native server becomes the primary FIX surface,
   the relay is Cloudflare-specific plumbing and belongs under
   `targets/cloudflare/`, not in the product server binary.

### 4.3 The FIX/TCP server (F2)

- **One I/O model.** Move `bunting-server` to `tokio`, already in the graph.
  `fix-wire`/`fix-session` are sans-I/O, so this touches only the accept loop
  and stream plumbing — the protocol layer is untouched.
- **Concurrent sessions.** Replace the single-connection accept loop with a
  bounded multi-session acceptor: N sessions, per-session `FixSession` +
  `FixApplicationState`, one authoritative writer serializing commits.
  Preserves single-writer origin semantics while serving many participants.
  This is the single highest-value change for the stated goal.
- **Split `runtime.rs`** (940 lines) into `acceptor.rs`, `session_host.rs`,
  `admin.rs`, `scenario.rs`.
- **Delete `civil_from_days` and `fix_timestamp`** in favour of `time 0.3`,
  already resolved.
- **Decide the admin surface.** The hand-written HTTP/1 `write_http` is ~40
  lines of parser that will grow. Either keep it deliberately frozen and
  minimal in `admin.rs`, or drop the HTTP admin entirely and expose
  administration through the CLI against the same origin store. Adding a web
  framework contradicts the shedding goal.
- **Feature-gate the TUI out of the server binary.** `bunting-cli` depends on
  `bunting-tui` unconditionally, so `bunting server` links `ratatui`,
  `crossterm`, `rustls`, `ring`, and the `windows-*` families. A default-off
  `tui` feature removes roughly half the dependency graph from the server
  build.

### 4.4 Rust / Python / C++ bindings (F3)

**Design constraint:** generics must not cross the FFI boundary, so
`bunting-rs` must first grow a concrete façade — a `Bunting` handle owning a
chosen `OriginStore` + `SnapshotCache` (in-memory and file implementations),
exposing submit / cancel / snapshot / recover and typed accessors. Every
binding wraps that one façade. This is also what finally makes `bunting-rs`
the real Rust binding instead of 23 lines of re-exports.

- **`bindings/bunting-ffi` (C ABI).** `crate-type = ["cdylib", "staticlib"]`,
  opaque `BuntingHandle*`, error out-parameters, explicit `bunting_free_*`.
  Generate `bunting.h` with `cbindgen`. One unsafe boundary, reusable by any
  language.
- **`bindings/bunting-py` (PyO3 + maturin).** Per current PyO3 guidance, use
  `crate-type = ["cdylib", "rlib"]` so integration tests can still `use` the
  crate, and opt into the **`abi3`** feature (`abi3t` if free-threaded CPython
  matters) for the limited API and stable ABI — one wheel across Python
  versions instead of one per version. Wrap the façade with
  `#[pyclass]`/`#[pymodule]` directly rather than going through the C header:
  richer types, proper exception mapping, and the GIL can be released around
  engine calls. Build/publish with maturin.
- **`bindings/bunting-cpp` (cxx).** `#[cxx::bridge]` with the engine handle as
  an **opaque `extern "Rust"` type** and orders/fills/snapshots as **shared
  structs** visible to both languages — the exact case cxx is designed for.
  `cxx-build` in `build.rs` emits the C++ side; ship a CMake package config so
  `ref/quarcc-trading-engine/engine-cpp` (already CMake) can link it
  unmodified.
- **Lints.** Each binding crate declares its own `[lints]` table rather than
  `workspace = true`, because `unsafe_code = "forbid"` cannot be relaxed
  locally. `AGENTS.md` §Package discipline needs a matching carve-out.
- **Contract tests.** Replay one golden scenario —
  `tests/goldens/competition-full-run.v1.json` already exists — through the
  Rust, Python, and C++ bindings and assert byte-identical canonical event
  streams. This is what keeps three bindings honest without tripling test
  surface.

### 4.5 Shedding sequence (F4)

1. Delete `apps/edge-api/`; relocate `temp/*.msi` outside the working tree;
   add `.DS_Store` to `.gitignore`.
2. Retire `tests/oracles/trpc/` and `tests/fixtures/reference/trpc/`, or record
   an explicit expiry date for the differential record. Removing it deletes the
   Node toolchain dependency outright.
3. Deregister the eight dependency-mirror submodules; update
   `docs/reference-functionality-audit.md` and
   `docs/reference-adoption.md` in the same change, as `AGENTS.md` requires.
4. Resolve the FIX evidence duplication: pick the codec strategy (keep the
   2,171 hand-rolled lines, or adopt one library), then drop the submodules
   that no longer inform a decision. Keep `quickfixj` — the Go/Java interop
   gate depends on those semantics.
5. Mark superseded ADRs (0004, 0013, 0014, 0015) as superseded in-file; archive
   `docs/claude/` and `docs/prompts/`; fold `docs/plans/` into whatever is
   still active.
6. Delete `compatibility/nbc/translation.rs`.

---

## 5. Sequencing

| Phase | Content | Why here |
| --- | --- | --- |
| 0 | Docs + ADR reframing Cloudflare as one wrapper; rewrite the CI policy step | Gates everything; without it, decoupling fails CI and gets reverted |
| 1 | Pure deletions: `edge-api`, `temp`, tRPC oracle, `.DS_Store`, `translation.rs`, submodule mirrors | Zero-risk, shrinks the surface before restructuring |
| 2 | Cloudflare extraction: engine manifest, rustflag scoping, `worker-cache` move, relay relocation | The "wrapper only" invariant, now enforceable |
| 3 | Concrete `bunting-rs` façade | Prerequisite for all three bindings |
| 4 | FIX server: tokio, multi-session, `runtime.rs` split, TUI feature gate | The primary product surface |
| 5 | `bunting-ffi` → `bunting-py` → `bunting-cpp`, plus the golden cross-binding test | Depends on phase 3; C ABI first so Python/C++ share a proven façade |
| 6 | Engine internals: split `lib.rs`/`simulation.rs`, extract `fix-application` from `bunting-application` | Refactor under a settled architecture, not before |

## 6. Unresolved

- **FIX codec strategy.** Keep 2,171 hand-rolled lines, or adopt `rustyfix`'s
  codec / another library? This decision blocks step 4 of §4.5 and should be
  made against the QuickFIX-Go interop gate, not against source aesthetics.
- **Multi-writer authority natively.** `docs/deployment.md` currently routes
  multi-instance authority to Worker/D1. If the native server becomes primary,
  does it stay single-writer per origin file, or gain a real native
  multi-writer store? This shapes §4.3.
- **Python binding depth.** Full engine embedding, or a participant/strategy
  client mirroring `ref/quarcc-trading-engine/python_client`? Different
  surface, different maintenance cost.
- **Worker parity after decoupling.** Does the Worker remain a full authority
  path (D1 origin, command transaction) or narrow to a read/stream edge with
  the native server holding authority? The second is much cheaper to keep
  correct and is more consistent with "wrapper only".
