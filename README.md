# Bunting

Bunting is a Rust market-simulation and exchange-testing platform designed to run in a plain Cloudflare Worker.

## Install

Release archives contain native `bunting-server` and `bunting-tui` executables
for macOS Apple Silicon, macOS Intel, Linux x86_64, and Windows x86_64. macOS
and Linux users can install the latest release into `~/.local/bin`:

```bash
curl -fsSL https://raw.githubusercontent.com/andrewkoumoudjian/bunting/main/install.sh | sh
```

Pin a release or choose other binary and configuration directories when
reproducibility or a system-wide path matters:

```bash
curl -fsSL https://raw.githubusercontent.com/andrewkoumoudjian/bunting/main/install.sh |
  BUNTING_VERSION=v0.1.0 BUNTING_INSTALL_DIR="$HOME/bin" \
  BUNTING_CONFIG_DIR="$HOME/.config/bunting/server" sh
```

The installer detects supported macOS/Linux platforms, downloads the matching
GitHub release archive, verifies it against `SHA256SUMS`, and installs both
commands plus server configuration templates. It preserves configuration files
that already exist. Windows users can download and extract the matching
`.tar.gz` archive from
[GitHub Releases](https://github.com/andrewkoumoudjian/bunting/releases).

Start a self-contained terminal fixture:

```bash
bunting-tui --fixture
```

To run the native FIX server, review the installed credentials and network
settings, then start it with:

```bash
bunting-server "${BUNTING_CONFIG_DIR:-$HOME/.config/bunting/server}/local.json"
```

The server and TUI are native applications because they use TCP, filesystem,
TLS, and terminal APIs. Each release separately includes
`bunting-worker-vX.Y.Z.tar.gz`, containing the optimized Worker Wasm module,
JavaScript shim, Wrangler configuration, and D1 migrations for Cloudflare
deployment.

## Engine model

Bunting distinguishes venue-side market engines from participant-side execution engines.

- The current default market path uses released [`OrderBook-rs`](https://github.com/joaquinbejar/OrderBook-rs) `0.10.3` for matching and order-book behavior.
- Bunting adds venue identity, canonical events, participant ledger/risk, origin persistence, recovery, browser transport, and outbound FIX/TCP around that kernel.
- NBC configuration, scheduling, synchronization, and provenance live inside `bunting-engine`; its translated matcher is retained only as a differential test oracle.
- QUARCC is a portable Rust participant execution engine with Bunting and Rust/WASM adapters. Humans and FIX sessions may bypass it, while built-in agents always use it.

See:

- [`docs/adr/0013-worker-orderbook-rs-kernel.md`](docs/adr/0013-worker-orderbook-rs-kernel.md)
- [`docs/adr/0014-market-and-execution-engine-boundaries.md`](docs/adr/0014-market-and-execution-engine-boundaries.md)
- [`docs/adr/0016-native-rust-trpc-worker.md`](docs/adr/0016-native-rust-trpc-worker.md)
- [`docs/adr/0017-authorized-nbc-jar-port.md`](docs/adr/0017-authorized-nbc-jar-port.md)
- [`docs/adr/0020-transport-neutral-engine-and-outbound-fix-tcp.md`](docs/adr/0020-transport-neutral-engine-and-outbound-fix-tcp.md)
- [`docs/reference-functionality-audit.md`](docs/reference-functionality-audit.md)
- [`docs/reference-adoption.md`](docs/reference-adoption.md)
- [`docs/architecture.md`](docs/architecture.md)

## Current architecture

- `OrderBook-rs` snapshots are checksum-protected and stored through the Cloudflare Workers Cache API under immutable, content-addressed keys.
- The origin event/version store remains authoritative for accepted commands, canonical events, idempotency, projections and optimistic concurrency.
- Cache misses or evictions are normal recovery events.
- Browser clients use the bounded `/api` fetch/stream contract; internal Worker components call Rust application functions directly.
- FIX session Durable Objects initiate outbound raw TCP and persist session state, but never own market authority or accept inbound raw TCP.
- User strategy outputs enter through the normal authenticated command/risk/persistence path.

## Reference policy

`ref/` is read-only evidence. It contains 25 Git submodules and three checked-in source/asset trees. It is never a production path dependency.

`vendor/` currently contains no implementation. It is reserved for explicitly approved copied/patched third-party source with licenses, notices, upstream metadata and patch records.

Do not classify a reference by its name. The source-backed inventory is in [`docs/reference-functionality-audit.md`](docs/reference-functionality-audit.md).

## Repository organization

The workspace is rooted at the repository `Cargo.toml`. Reusable first-party Rust crates live under `packages/`, the curated composition crate lives under `bunting-rs/`, and deployable applications live under `apps/`.

Cargo-less future scaffolds remain under `crates/` until a roadmap phase introduces real source, tests and a reviewed package boundary. Generated release assembly belongs under ignored `out/` paths.

Read the complete move map and Codex execution contract in [`docs/repository-reorganization.md`](docs/repository-reorganization.md).

## Current workspace

- `market-types`: checked Bunting identifiers and fixed-point values;
- `market-events`: protocol-neutral commands and canonical event envelopes;
- `bunting-engine`: the sole authoritative engine and private version-pinned adapter around `OrderBook-rs`;
- `ledger`: participant cash, position and reservation projections;
- `risk-engine`: participant/account controls not supplied by the upstream book;
- `origin-store`: authoritative projections, idempotency, expected-version commits and recovery metadata;
- `command-transaction`: recovery, risk, matching, accounting and commit orchestration;
- `quarcc-execution-engine`, `quarcc-bunting-adapter`, and `quarcc-execution-wasm`: portable participant execution, venue mapping, and browser bindings;
- `bunting-agents`: deterministic built-in policies composed with mandatory QUARCC execution;
- `simfix-wire`, `simfix-session`, and `simfix-mapping`: FIX framing, session recovery, and application mapping;
- `worker-cache`: immutable Workers Cache snapshot adapter;
- `bunting-rs`: thin portable composition crate with curated first-party re-exports and product metadata;
- `apps/bunting-worker`: browser API and outbound FIX-session Worker entrypoint.
- `apps/bunting-server`: native FIX/TCP server, durable local origin, admin health surface, and external Cloudflare FIX relay.
- `apps/bunting-tui`: Longbridge-derived native Ratatui trading workstation and FIX/TCP test harness; run it with `cargo run --locked -p bunting-tui`.

## Native Worker transports

The Worker exposes bounded `GET|POST /api/<procedure>` handlers for browser clients. Authenticated FIX-session control requests address `/fix-sessions/<id>/...`; each object opens outbound TCP to an external acceptor.

Actor identity comes from the server-configured participant claim associated with the verified bearer token. No request header or procedure input can select a participant.

Deployment and migration commands use the Worker config at `apps/bunting-worker/wrangler.toml`:

```bash
npx wrangler d1 create bunting-origin
npx wrangler d1 migrations apply bunting-origin --config apps/bunting-worker/wrangler.toml --remote
npx wrangler secret put BUNTING_API_TOKEN --config apps/bunting-worker/wrangler.toml
npx wrangler secret put BUNTING_API_PARTICIPANT_ID --config apps/bunting-worker/wrangler.toml
```

Set `BUNTING_FIX_DESTINATIONS` in the environment-specific Wrangler configuration to a comma-separated allowlist of exact `host:port` acceptor destinations before enabling FIX sessions.

Scenario/orchestration code provisions runs before order entry. Command procedures return a typed `NOT_FOUND` error instead of creating authoritative state implicitly.

## Checks

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-engine | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```
