# Bunting deployment guide

The Wasmer-hosted WASI server is the primary competition venue under ADR 0027.
It serves the concurrent rostered market defined by ADR 0023, while Cloudflare
is a read-only publication wrapper under ADR 0022.

## Build the WASI server

Install Wasmer `7.2.1`, cargo-wasix `0.1.28`, and WASIX toolchain
`v2026-07-07.3+rust-1.96`, then run:

```bash
tools/build_wasi_server.sh
```

The script builds the locked `bunting-server` binary for
`wasm32-wasmer-wasi-dl`, validates the portable `.wasm`, and asks Wasmer's
Cranelift compiler to produce an ignored host-specific `.wasmu` cache. The
portable module is the release input; the compiled cache is only for the
current host.

Plain `wasm32-wasip1` is insufficient because the venue creates two inbound
listeners. WASIX supplies the WASI-compatible listen and thread extensions
required by the server.

## Run locally

The checked-in local profile binds FIX to `127.0.0.1:9880` and administration
to `127.0.0.1:8080`, serves two rostered participants against one shared
market, persists origin and per-participant FIX recovery beside the
configuration, and enforces every configured queue, message, journal, and
open-order bound:

```bash
tools/run_wasi_server.py apps/bunting-server/config/local.json
```

The launcher enables Wasmer networking and mounts only the parent directories
required by the configuration, scenario, origin, and session files. Verify the
running process with:

```bash
curl --fail http://127.0.0.1:8080/health
curl --fail -H 'Authorization: Bearer replace-admin-token' \
  http://127.0.0.1:8080/admin/runs/1
```

Run the native TUI separately with `cargo run --locked -p bunting-cli -- tui`.

## Hosted competition

Initialize and review the templates, then run one authoritative process for the
shared event:

```bash
bunting init
bunting-server ~/.config/bunting/server/hosted-native.json
```

The hosted profile requires durable file storage, an immutable scenario,
loopback administration, and mutual TLS at the trusted terminator. Do not run a
second process against the same origin file because the store is
single-writer. Wasmer must receive the configuration, scenario, origin, and
session directories; the installed launcher resolves and mounts them.

The hosted smoke gate is complete only after the terminator presents a valid
client certificate, two rostered clients complete FIX Logon, one participant's
order is visible to the other, and a restart returns the acknowledged run and
session sequences from the same files. A plaintext public bind, shared origin
file, native-only smoke, or cross-compile without Wasmer execution fails the
deployment contract.

## Cloudflare publication wrapper

Cloudflare supports Rust Workers through `workers-rs` and `worker-build`, with
Wrangler deploying the generated bundle. Under ADR 0022 it publishes immutable
public snapshots, run archives and leaderboards committed by the WASI venue;
it does not accept participant commands or own origin truth. See the official
[Rust Worker guide](https://developers.cloudflare.com/workers/languages/rust/)
and [TCP sockets contract](https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/).

The publication cutover is not complete while the Worker still exposes command,
D1-origin, or outbound-FIX routes. Those routes are compatibility debt and must
not be used for new competition deployments. The publication smoke gate will
require checksum-addressed reads of a snapshot, archive, and leaderboard after
Phase 4 supplies those artifacts.

### Raw Workerd gate

The repository also carries a raw Workerd configuration under
`apps/bunting-worker/workerd/`. It loads the optimized JS/Wasm bundle, the real
`FixSessionObject` namespace, and the production compatibility date without
Wrangler or a Cloudflare account:

```bash
cd apps/bunting-worker
worker-build --release --no-panic-recovery
npx --yes workerd@1.20260716.1 serve workerd/workerd.capnp
```

The Workerd gate requires `GET /api/system.health` to report contract
compatibility, authenticated `GET /fix-sessions/smoke/snapshot` to instantiate
the Durable Object and return empty state, and `market.snapshot` to fail closed
with `ORIGIN_UNAVAILABLE`. Raw Workerd does not supply the Cloudflare D1
service, so the configuration intentionally binds D1's standard wrapper to a
501 stub; D1 migrations and transactions remain local-Miniflare or remote
staging gates. Workerd is a runtime/deployment validation here, not a hardened
multi-tenant sandbox or a D1 emulator.
