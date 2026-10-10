# ADR 0044: The venue ships as a native binary; WASIX is retired

- Status: Accepted (2026-10-10). The owner answered on 2026-10-09 that
  Wasmer/WASIX is "not necessarily" the primary host "if there's better ways to
  run the binary anywhere", and on 2026-10-10 authorized superseding legacy ADRs
  when that is the right call. Implemented in slice 27.
- Date: 2026-10-10
- Supersedes: ADR 0027 (Wasmer-hosted WASI server) in whole; the host clause
  of ADR 0033 (decision 3), which deferred this choice to a measured ADR.
- Depends on: ADR 0022 (single native venue), ADR 0033.

## Context

ADR 0027 made a WASIX module run by Wasmer the only supported server runtime.
ADR 0033 kept that as packaging only and asked for a host ADR based on
measured native-versus-WASIX data. That data now exists (implementation log,
slice 27; `tools/host_parity.sh`), measured on Linux x86_64 with Wasmer
`7.2.1`, cargo-wasix `0.1.28` and WASIX toolchain `v2026-07-07.3+rust-1.96`:

1. **Observed: the released WASIX server cannot run a competition.** The
   durable file origin needs an OS single-writer lease. Under WASIX
   `cfg(unix)` is false, so `FileOriginStore::open` returned
   `OriginError::Unavailable`. The WASIX module exits with
   `origin store error: Unavailable` on the shipped `local.json` and
   `hosted-native.json`, both of which use file storage. Only the memory
   origin starts, and it loses every committed input on exit.
2. **Observed: the engine is deterministic across hosts.** A 2,070-command
   journal recorded by the native server (built-in agents plus 2,000 FIX
   orders) replays to the same command and event counts, journal chain and
   state hash under a native build and a WASIX build of
   `bunting-rs/examples/replay_archive.rs`.
3. **Observed: WASIX is slower.** Order-to-first-ExecutionReport round trip
   over loopback, 2,000 orders one at a time, memory origin, two runs: native
   p50 271 / 295 µs, p99 1.15 ms / 0.95 ms; WASIX p50 694 / 968 µs, p99
   2.04 / 4.53 ms (2.6–3.3× at p50).
4. **Observed: WASIX adds host behavior a real venue would not.** A WASIX
   server started right after another server on the same ports failed with
   `Address in use`, where the native server binds (*inferred*: the WASIX
   listener does not set `SO_REUSEADDR`, so `TIME_WAIT` connections block a
   restart).
   Kernel RTT (`tcp_rtt.rs`, ADR 0035's published measured delay) is
   Linux-native only.
5. **Observed: the WASIX gates had stopped protecting anything.** They ran
   only in CI (the toolchain installer resolves releases through
   `api.github.com`), and the latest completed CI run on `main` (`b33db28`)
   failed in the QuickFIX-Go step because the test's configuration had fallen
   behind the server schema; the Wasmer smoke step after it never ran.
6. **Observed: the native `bunting` executable already linked the server**
   but refused `bunting server` and pointed to Wasmer, and release archives
   for Linux, macOS and Windows already existed for the CLI and TUI.
7. **Observed: the Bunting Native Protocol is native-only.** BNP v1 with
   TLS 1.3 mutual authentication (ADR 0040, slice 25) runs in the native
   server; nothing was ever packaged or tested for it under WASIX, so a WASIX
   venue would have served FIX only or needed BNP and TLS ported and tested
   there too.
8. **Observed: a static musl Linux build is slower than glibc.** p50 743 µs
   (memory origin) and 9.0–10.9 ms (file origin) against glibc 271 µs and
   3.3 ms. Swapping in mimalloc fixed the memory-origin case (212–241 µs) but
   not the file-origin case (9.2 ms); the remaining cost is in the per-message
   FIX session snapshot rewrite (*inferred*, see Consequences).

## Decision

1. **The venue is a native executable.** `bunting server <config>` runs the
   venue in-process (`apps/bunting-cli` calls `bunting_server::runtime::run`).
   Release archives ship one `bunting` executable with `bunting-server` and
   `bunting-tui` as aliases (symlinks on Unix, copies on Windows), plus the
   `bunting-trader` participant app (ADR 0040). The standalone
   `bunting-server` binary target remains for tests.
2. **Release targets:** `x86_64-unknown-linux-gnu` and
   `aarch64-unknown-linux-gnu` built on Ubuntu 22.04 (glibc 2.35 floor),
   `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`.
   Linux uses glibc, not musl, because of the measurement in context item 8.
3. **Container image:** `apps/bunting-server/Dockerfile` builds the same
   executable on `rust:1.88.0-bookworm` and runs it on
   `gcr.io/distroless/cc-debian12:nonroot` (uid 65532), with the configuration
   at `/etc/bunting/server.json` and state on the `/var/lib/bunting` volume.
   Releases publish it to `ghcr.io/<owner>/bunting:<tag>` for `linux/amd64`.
   It covers hosts below the glibc floor. The security model is unchanged:
   listeners stay on loopback unless a mutual-TLS terminator is configured, so
   the image runs beside its terminator (same pod) or with host networking.
4. **Durable mode on every release platform.** The single-writer lease is
   `flock` on Unix and an exclusive (no-sharing) open of `<origin>.lock` on
   Windows; both are released when the handle closes. Hosts with neither
   still refuse durable mode.
5. **WASIX is retired.** No WASIX artifact is built, released or installed;
   `tools/build_wasi_server.sh` and `tools/run_wasi_server.py` are removed.
   `tools/host_parity.sh` and the `wasm32-wasmer-wasi*` entries in
   `.cargo/config.toml` remain only to re-run this measurement; they are not a
   supported target and no code path may depend on them. The engine and
   protocol packages keep the `wasm32-unknown-unknown` gate.
6. **Every release platform is exercised in CI.** `tools/smoke_server.py`
   starts the server with the durable origin and built-in agents, trades over
   FIX, restarts on the same journal and requires the committed sequence to
   survive. CI runs it on Linux, macOS and Windows and against the container
   image, and runs the QuickFIX-Go interop test against the native server.

## Consequences

- Operators download one archive or pull one image; no runtime to install,
  no capability flags to grant, no separate toolchain to build.
- Windows hosts can run a durable competition venue for the first time.
- The latency participants see is the native host's: about 0.3 ms per order
  round trip on loopback with the memory origin, 3–4 ms with the file origin
  (fsync before acknowledgement). The file-origin figure is dominated
  by rewriting and fsyncing each FIX session's full snapshot (~275 KB after
  2,000 orders) on every message (`session_host.rs` `persist_session`);
  making that incremental is follow-up work and would also reduce the
  musl penalty.
- Termination is crash-equivalent: the server installs no signal handler and
  every acknowledged input is already journaled. Containers should run with
  `--init` (or an equivalent) so `SIGTERM` stops PID 1 promptly.
- The sandbox WASIX offered (explicit directory grants) is gone. Operators
  who want it use the container image, OS service sandboxing or a dedicated
  user; the server still touches only its configured paths.
- glibc older than 2.35 (for example RHEL 9) needs the container image.

## Rejected alternatives

- **Keep WASIX as a secondary target.** It cannot run a durable competition
  without a lease primitive the server could trust, it is 2.6–3.3× slower,
  it would serve FIX only unless BNP and mutual TLS were ported and tested
  there, and its gates had already rotted. Keeping it "cheap" was not true.
- **Static musl binaries as the Linux release** (the exploration note's first
  candidate). Measured 2.7× slower acknowledgements; mimalloc recovers the
  memory-origin case but would add a C dependency and still leave the
  file-origin path slower than glibc.
- **Build Linux on an older glibc (Debian 11 container) for a 2.31 floor.**
  Possible later; the container image already covers older hosts, and the
  wheel and FFI jobs run on stock runners.
- **Signal handling for graceful shutdown.** Not needed for correctness and
  would need a new dependency or `unsafe`; `--init` covers containers.
- **A Linux-arm64 container image now.** Emulated builds are slow; the
  aarch64 archive covers ARM hosts until a native-runner image job is added.

## Validation

Recorded in the implementation log (slice 27) with commands and figures:

- `tools/host_parity.sh 2000`: identical native and WASIX replay output;
  loopback latency for native file, native memory and WASIX memory origins;
- QuickFIX-Go interop passes against the native server
  (`BUNTING_SERVER_BIN=target/release/bunting-server go test .`);
- `tools/smoke_server.py` passes against `bunting server` natively and against
  the container image (`docker run --init --network host`), including the
  restart check;
- `cargo check` and `cargo clippy -D warnings` of `bunting-server` and
  `bunting-cli` for `x86_64-pc-windows-msvc` and `x86_64-apple-darwin`;
- the seven required gates.

## Operational impact

Install with `install.sh` (Linux x86_64/aarch64, macOS) or a release archive,
or pull the image. Start with `bunting server <config>` (or `bunting-server
<config>`). `bunting init` writes configuration templates. CI no longer
installs Wasmer, cargo-wasix or a second Rust toolchain; it adds a macOS and
Windows smoke job and a container smoke job. Releases need `packages: write`
to publish the image.

## Security impact

Listeners, roster authentication, loopback administration, TLS termination
rules and per-session bounds are unchanged. The container runs as a non-root
user on a distroless base with no shell. The Wasmer directory sandbox no
longer applies; the server's file access is limited to its configured
storage, scenario and session paths by construction, and operators can add OS
or container confinement.

## References

- [ADR 0027](0027-wasmer-wasi-server-runtime.md) (superseded),
  [ADR 0033](0033-guidance-reconciliation-2026-10-09.md),
  [ADR 0022](0022-native-competition-venue-and-publication-worker.md),
  [ADR 0035](0035-latency.md)
- [Exploration note: running the binary anywhere](../research/2026-10-09-exploration-and-next-steps.md#running-the-binary-anywhere-answer-3)
- [`../deployment.md`](../deployment.md), `tools/host_parity.sh`,
  `tools/host_load.py`, `tools/smoke_server.py`, `apps/bunting-server/Dockerfile`
