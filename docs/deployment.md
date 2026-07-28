# Bunting deployment guide

The native server is the primary competition venue. This guide covers its
implemented single-session baseline and the transition to the concurrent venue
defined by ADR 0023. Cloudflare is being narrowed to read-only publication
under ADR 0022; the existing Worker mutation and outbound-FIX paths remain
transitional until that cutover is complete.

## Zero-configuration local server

Run an ephemeral loopback server with the canonical one-listing scenario:

```bash
cargo run --locked -p bunting-cli -- server
```

It binds FIX to `127.0.0.1:9880`, administration to `127.0.0.1:8080`, permits
one FIX connection, bounds messages/journals/events, and uses in-memory origin
state. The matching TUI local profile uses the same endpoint and a loopback-only
development credential when `BUNTING_LOCAL_PASSWORD` is absent:

```bash
cargo run --locked -p bunting-cli -- tui
```

Use the checked-in `local.json` through `bunting server <path>` when local
state must survive restart. The zero-configuration profile is intentionally
ephemeral and must never be exposed outside loopback.

The CI smoke gate launches the exact zero-configuration command and requires:

```bash
curl --fail http://127.0.0.1:8080/health
curl --fail -H 'Authorization: Bearer bunting-local-admin-token' \
  http://127.0.0.1:8080/admin/runs/1
```

## Isolated hosted-native sessions

Initialize templates, then create one configuration per hosted session:

```bash
bunting init
cp ~/.config/bunting/server/hosted-native.json session-42.json
bunting server session-42.json
```

Each process has one immutable participant/run binding, accepts at most one FIX
connection, and must use a distinct durable origin path, FIX/admin ports and
scenario. Validation requires file storage, an immutable scenario, loopback
administration, and mutual TLS at the trusted terminator. Put no second server
process on the same origin file because the native store is intentionally
single-writer; use the Worker/D1 deployment for multi-instance authority.

The hosted smoke gate is complete only after the terminator presents a valid
client certificate, Bunting accepts the matching FIX Logon, an order is
committed, and a restart returns the acknowledged run sequence from the same
origin file. A plaintext public bind or shared origin file fails the deployment
contract.

## Cloudflare publication wrapper

Cloudflare supports Rust Workers through `workers-rs` and `worker-build`, with
Wrangler deploying the generated bundle. Under ADR 0022 it publishes immutable
public snapshots, run archives and leaderboards committed by the native venue;
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
