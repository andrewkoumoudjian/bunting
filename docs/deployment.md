# Bunting deployment guide

This guide covers the implemented native server and Cloudflare Worker/relay
topologies. It does not claim a live staging deployment: the local gate is
automated in CI, while the hosted gates require operator-owned DNS, D1,
certificates, secrets and a publicly reachable FIX acceptor.

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

## Cloudflare Worker and external FIX relay

Cloudflare supports Rust Workers through `workers-rs` and `worker-build`, with
Wrangler deploying the generated bundle. Workers can initiate outbound TCP,
but cannot accept an inbound raw TCP connection, so Bunting uses the external
relay topology recorded in ADR 0020. See the official
[Rust Worker guide](https://developers.cloudflare.com/workers/languages/rust/)
and [TCP sockets contract](https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/).

1. Replace the D1 database ID and `BUNTING_FIX_DESTINATIONS` placeholder in an
   environment-specific copy of `apps/bunting-worker/wrangler.toml`.
2. Install `BUNTING_API_TOKEN`, `BUNTING_API_PARTICIPANT_ID`, and any FIX
   credentials through Wrangler secrets; never place them in TOML or JSON.
3. Apply the checked-in D1 migrations before deploy. Cloudflare records applied
   migrations and applies pending files in order; the authoritative command
   path must not run against an older schema. See the official
   [D1 migrations guide](https://developers.cloudflare.com/d1/reference/migrations/).
4. Deploy the Rust Worker, then run `bunting relay <cloudflare.json>` on a
   public host behind a mutually authenticated TLS terminator. The Worker
   destination must be the relay's public Worker listener, never localhost or a
   private address.

```bash
npx wrangler d1 migrations apply bunting-origin \
  --config apps/bunting-worker/wrangler.toml --remote
npx wrangler deploy --config apps/bunting-worker/wrangler.toml
bunting relay ~/.config/bunting/server/cloudflare.json
```

The Cloudflare smoke gate requires the deployed `/api/system.health` response, an
authenticated browser projection, a Worker-initiated outbound FIX Logon at the
relay, one committed order/execution report, duplicate-command idempotency, and
recovery after FIX-session-object restart. A local relay handshake proves only
configuration and byte forwarding; it is not evidence of Worker network
interoperability.

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
