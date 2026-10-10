# Bunting deployment guide

The venue server is the native `bunting` executable: `bunting server <config>`
runs it in-process ([ADR 0044](adr/0044-native-server-binary-host.md)).
Release archives cover Linux x86_64 and aarch64 (glibc 2.35 or newer), macOS
Apple Silicon and Intel, and Windows x86_64; a container image covers
everything else. The server serves the concurrent rostered market defined by
ADR 0023, while Cloudflare is a read-only publication wrapper under ADR 0022.

## Install or build

Install a release with `install.sh` (see the README) or download an archive.
Each archive's `bin/` holds `bunting` plus `bunting-server` and `bunting-tui`
aliases that route to `bunting server` and `bunting tui`, and the
`bunting-trader` participant app. From a checkout:

```bash
cargo build --locked --release -p bunting-cli
target/release/bunting server apps/bunting-server/config/local.json
```

The durable file origin works on every release platform: the single-writer
lease is `flock` on Unix and an exclusive open of `<origin>.lock` on Windows.

## Run locally

The checked-in local profile binds FIX to `127.0.0.1:9880` and administration
to `127.0.0.1:8080`, serves two rostered participants against one shared
market, persists origin and per-participant FIX recovery beside the
configuration (relative paths resolve against the configuration's directory),
and enforces every configured queue, message, journal and rate bound:

```bash
bunting server apps/bunting-server/config/local.json
```

Verify the running process with:

```bash
curl --fail http://127.0.0.1:8080/health
curl --fail -H 'Authorization: Bearer replace-admin-token' \
  http://127.0.0.1:8080/admin/runs/1
```

`tools/smoke_server.py --state-dir <dir> -- bunting server` runs the same
check CI runs on Linux, macOS, Windows and the container: durable origin,
built-in agents, FIX order flow, then a restart that must keep every
committed command. Run the terminal separately with `bunting tui`.

## Container image

```bash
docker build -f apps/bunting-server/Dockerfile -t bunting .   # or pull ghcr.io/andrewkoumoudjian/bunting:<tag>
docker run --init --network host \
  -v "$PWD/etc:/etc/bunting:ro" -v bunting-state:/var/lib/bunting bunting
```

The image runs `bunting server /etc/bunting/server.json` as uid 65532 on a
distroless glibc base; put the storage path under `/var/lib/bunting` and the
scenario next to the configuration. Listeners stay on loopback unless a
mutual-TLS terminator is configured, so run the image beside its terminator
(same pod) or with host networking. The server installs no signal handler:
stopping it is crash-equivalent and safe because every acknowledged input is
already journaled, and `--init` makes `docker stop` take effect promptly.
Templates are in `/usr/share/bunting/config`.

## Hosted competition

Initialize and review the templates, then run one authoritative process for the
shared event:

```bash
bunting init
bunting server ~/.config/bunting/server/hosted-native.json
```

The hosted profile requires durable file storage, an immutable scenario,
loopback administration, and mutual TLS at the trusted terminator. Do not run a
second process against the same origin file because the store is
single-writer; a second process on the same origin fails to take the lease.

The hosted smoke gate is complete only after the terminator presents a valid
client certificate, two rostered clients complete FIX Logon, one participant's
order is visible to the other, and a restart returns the acknowledged run and
session sequences from the same files. A plaintext public bind or shared origin file fails the deployment contract.

## Cloudflare publication wrapper

Cloudflare supports Rust Workers through `workers-rs` and `worker-build`, with
Wrangler deploying the generated bundle. Under ADR 0022 it publishes immutable
public snapshots, run archives and leaderboards committed by the native venue;
it does not accept participant commands or own origin truth. See the official
[Rust Worker guide](https://developers.cloudflare.com/workers/languages/rust/)
and [TCP sockets contract](https://developers.cloudflare.com/workers/runtime-apis/tcp-sockets/).

The transitional Worker (`apps/bunting-worker`: D1 origin, Worker command
routes, outbound FIX Durable Objects and its Workers Cache crate) was removed on
2026-10-08 because it was a second market authority that ADR 0022 prohibits.
No Cloudflare artifact is currently built. A replacement publisher must only read
immutable, checksum-addressed archives, leaderboards and snapshots exported by the
native venue after commit; it must not accept participant commands.

Organizers should also read the known limitations at the end of
[`RUNBOOK.md`](../RUNBOOK.md) before a live event.
