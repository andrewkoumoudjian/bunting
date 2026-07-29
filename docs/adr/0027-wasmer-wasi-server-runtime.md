# ADR 0027: Wasmer-hosted WASI competition server

- Status: Accepted
- Date: 2026-07-29
- Depends on: ADR 0022 and ADR 0023
- Supersedes: ADR 0023's asynchronous acceptor mechanism only

## Context

The competition server needs inbound FIX/TCP, an administration listener,
bounded concurrent sessions, threads, clocks, and explicitly mounted durable
files. Plain WASI preview 1 does not provide the socket creation and listen
operations needed to host that process. Wasmer's WASIX target preserves the
WASI ABI and adds the socket and thread calls required by a long-running server.

The server's FIX application loop is already synchronous. Tokio was used only
to supervise native listeners and blocking tasks, while `orderbook-rs 0.10.3`
independently requires Tokio 1.52. Wasmer's published Tokio fork is 1.47, so
making it the workspace Tokio would violate the required OrderBook-rs
dependency contract.

## Decision

The production competition server is a WASI module built with
`cargo-wasix 0.1.28`, toolchain `v2026-07-07.3+rust-1.96`, and target
`wasm32-wasmer-wasi-dl`. Wasmer `7.2.1` validates and compiles that portable
module with Cranelift, then runs it with WASIX networking enabled.

The server host uses bounded standard-library threads and blocking sockets.
Market authority, FIX sequencing, session recovery, the single writer, and
commit-before-acknowledgement behavior are unchanged. OrderBook-rs retains its
released `0.10.3` and Tokio 1.52 requirements; the server does not adopt or
mislabel the older WASIX Tokio fork.

Wasmer receives host networking and only the configuration, scenario, origin,
and session directories resolved by the launcher. Release archives contain the
portable `.wasm` module. A local build may also produce a host-specific
`.wasmu` compiled artifact, but that file is ignored and is never a portable
release input.

The TUI, bindings, and non-server CLI commands remain ordinary native
artifacts. The native server binary remains buildable for focused tests, but
the supported competition runtime and release entrypoint use Wasmer.

## Consequences

One portable server module runs on every platform supported by the selected
Wasmer release. Native and WASI tests share the same server library and
configuration contract, while CI proves the actual Wasmer-hosted listener
instead of treating a cross-compile as runtime evidence.

Operators must install Wasmer and grant explicit network and directory access.
WASIX is a Wasmer extension rather than a portable plain-WASI socket contract,
so changing runtimes requires a new compatibility decision and live
interoperability gate.

## Rejected alternatives

### Plain `wasm32-wasip1`

Rejected because preview 1 can accept inherited sockets but cannot create and
bind both Bunting listeners through Rust's standard networking API.

### Replace workspace Tokio with the WASIX Tokio fork

Rejected because the current fork is older than the version required by
OrderBook-rs 0.10.3. Two runtime versions would also add scheduler complexity
where synchronous bounded host threads are sufficient.

### Keep the production server as a native executable

Rejected because the selected deployment contract requires Wasmer and a WASI
artifact. Native builds remain useful test evidence but are not the supported
competition runtime.

### Move competition authority to the Cloudflare Worker

Rejected by ADR 0022 because the Worker is a read-only publication wrapper and
cannot accept inbound raw TCP.

## Validation

- `tools/build_wasi_server.sh` builds the exact locked server module, validates
  it, and creates a Wasmer Cranelift artifact;
- Wasmer starts the module with bounded filesystem grants and both FIX/admin
  listeners;
- the health and authenticated run endpoints answer through the Wasmer-hosted
  process;
- two independent QuickFIX-Go clients log on and observe shared committed
  liquidity through that process;
- native focused tests, workspace tests, Clippy, formatting, the
  `orderbook-rs v0.10.3` pin, and the existing Worker Wasm gate remain green.

## Operational impact

Build hosts install the pinned cargo-wasix toolchain and Wasmer runtime.
Operators run `bunting-server <config>` after installation or
`tools/run_wasi_server.py <config>` from a checkout. The launcher resolves
relative paths against the configuration directory and grants Wasmer only the
required parent directories.

Release automation publishes one portable WASI server archive in addition to
native TUI and binding artifacts. The installer verifies both archives before
installing the launcher and module.

## Security impact

Wasmer starts without filesystem access beyond explicitly resolved Bunting
directories. Network access is required for inbound FIX and administration,
so roster authentication, loopback administration, TLS termination, and every
existing per-session bound remain mandatory. Host-specific compiled artifacts
are never accepted as cross-platform release inputs.

## References

- [`0022-native-competition-venue-and-publication-worker.md`](0022-native-competition-venue-and-publication-worker.md)
- [`0023-concurrent-participant-fix-sessions.md`](0023-concurrent-participant-fix-sessions.md)
- [`../deployment.md`](../deployment.md)
- [Wasmer Rust server guide](https://docs.wasmer.io/edge/guides/rust-http-server/)
- [Wasmer runtime CLI](https://docs.wasmer.io/runtime/cli/)
- [WASIX Rust installation](https://wasix.org/docs/language-guide/rust/installation)
