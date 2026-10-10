# Bunting

Bunting is a Rust market-simulation and exchange-testing platform: one
deterministic engine that owns matching, orders, the economic ledger, logical
time and replay, served by one venue process to participants over FIX and the
certified Bunting Native Protocol (BNP, with the `bunting-trader` client). It underpins a
RIT-class classroom simulator and a QUARCC competition venue. Cloudflare is an
optional read-only publisher of results.

> **Agents and contributors:** start with [`AGENTS.md`](AGENTS.md) and the
> [documentation status map](docs/README.md). Many older documents are kept as
> history and are marked "do not follow".

## Install

Release archives contain one portable WASI server plus native TUI and binding
artifacts for macOS Apple Silicon, macOS Intel, Linux x86_64, and Windows
x86_64. Install [Wasmer 7.2.1](https://docs.wasmer.io/install/) first. macOS
and Linux users can then install the latest release into `~/.local/bin`:

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
GitHub release archive, verifies it against `SHA256SUMS`, and installs the
command plus server configuration templates. It preserves configuration files
that already exist. Windows users can download and extract the matching
`.tar.gz` archive from
[GitHub Releases](https://github.com/andrewkoumoudjian/bunting/releases).

Start a self-contained terminal fixture:

```bash
bunting tui --fixture
```

To run the Wasmer-hosted FIX server, review the installed credentials and
network settings, then start it with:

```bash
bunting-server "${BUNTING_CONFIG_DIR:-$HOME/.config/bunting/server}/local.json"
```

`bunting-server` grants Wasmer networking and only the directories referenced
by the selected configuration. `bunting` and `bunting-tui` remain native
because terminal and language-binding APIs are platform-specific. No
Cloudflare publication Worker is currently built (removed in `eed8e00`; see
ADR 0022).

The server currently ships as a WASIX module (ADR 0027). WASIX is not a
binding long-term host (ADR 0033); native server packaging is under evaluation.

From a checkout, build the portable module and a host-specific Wasmer artifact,
then run it:

```bash
tools/build_wasi_server.sh
tools/run_wasi_server.py apps/bunting-server/config/local.json
```

The pinned build uses cargo-wasix `0.1.28`, WASIX toolchain
`v2026-07-07.3+rust-1.96`, target `wasm32-wasmer-wasi-dl`, and Wasmer `7.2.1`.

## Engine model

Bunting separates the venue-side market engine from participant-side
execution engines.

- `bunting-engine` is the only market authority. It owns a private,
  deterministic price-time order book keyed by 128-bit order IDs
  ([ADR 0029](docs/adr/0029-bunting-owned-deterministic-order-book.md)), one
  economic ledger (cash, reservations, fees, positions, cost basis, P&L, FX),
  listing-scoped books and market data, the simulation domain (tenders, OTC,
  news, facilities, scoring) and canonical events.
- [OrderBook-rs](https://github.com/joaquinbejar/OrderBook-rs) is a
  development-only differential oracle, not a production dependency.
- QUARCC is a portable Rust participant execution engine with Bunting and
  Rust/WASM adapters. Humans and FIX sessions may bypass it; built-in agents
  always use it.
- NBC is reference evidence only
  ([ADR 0032](docs/adr/0032-proposed-nbc-reference-only.md)); the engine's
  NBC compatibility surface was removed in slice 11.

See [`docs/architecture.md`](docs/architecture.md) for what is implemented
now versus the accepted target.

## Status and priorities

Implemented through slice 21 (see
[`docs/implementation-log/`](docs/implementation-log/2026-10-07-core-slice-0.md)):
single ledger, explicit listing identity and venue market data, owned order
book, append-only native origin journal, removal of the Cloudflare command
Worker, removal of NBC compatibility, and (slice 12) fills delivered to every
affected participant, engine-owned live-order limits and per-session FIX
identity namespacing, a command-path measurement baseline (slice 13) and
(slice 14) a writer-owned live run with a command-sourced journal and
state-only checkpoints; latency-modeled continuous admission with real
connectivity counted and a published location map (slices 15–17,
[ADR 0035](docs/adr/0035-latency.md)); built-in agents admitted through the
same sequencer and recoverable after a crash (slices 18–19); a version 2
archive that replays every journaled input (slice 20); and per-venue public
feeds of anonymous trades, depth changes and order-by-order changes, plus
a consolidated tape from a processor at the hub, with broker identifiers
on venues that publish them (slices 21–24,
[ADR 0036](docs/adr/0036-public-market-data-feeds.md)); and the Bunting
Native Protocol over mutual TLS with a reference client library and CLI
(slice 25, [ADR 0040](docs/adr/0040-bunting-native-protocol-v1.md)).

Current priorities and their order are in the
[October 9 exploration note §8](docs/research/2026-10-09-exploration-and-next-steps.md#8-owner-decisions-2026-10-09-and-revised-plan):
the multi-day calendar with auctions, and moving the app onto the native
protocol ([ADR 0031](docs/adr/0031-proposed-bunting-native-client-protocol.md)).
Accepted Target decisions are not implemented until the implementation log
says so.

Earlier research: [October 7 architecture audit](docs/research/2026-10-07-independent-core-architecture-audit.md),
[October 8 agent/market simulation research](docs/research/2026-10-08-agent-market-simulation-implementations.md),
[October 8 algorithm survey](docs/research/2026-10-08-expanded-market-algorithm-survey.md).

## Reference policy

`ref/` is read-only evidence. It contains 17 Git submodules and three checked-in source/asset trees. It is never a production path dependency.

`vendor/` currently contains no implementation. It is reserved for explicitly approved copied/patched third-party source with licenses, notices, upstream metadata and patch records.

Do not classify a reference by its name. The source-backed inventory is in [`docs/reference-functionality-audit.md`](docs/reference-functionality-audit.md).

## Repository organization

The workspace is rooted at the repository `Cargo.toml`. Reusable first-party
Rust crates live under `packages/`, the curated composition crate under
`bunting-rs/`, deployable applications under `apps/`, and language bindings
under `bindings/`. Future packages appear only with real source, tests and a
reviewed package boundary. Generated release assembly belongs under ignored
`out/` paths.

## Current workspace

- `market-types`: checked identifiers and fixed-point values;
- `market-events`: canonical commands, events and envelopes;
- `bunting-engine`: the sole authoritative engine, owned order book and simulation domain;
- `ledger`: the single economic ledger;
- `risk-engine`: pure order admission over ledger counters;
- `origin-store`: commit contract, idempotency and expected-version checks;
- `command-transaction`: recovery, transition and commit orchestration;
- `bunting-application`: transport-neutral application service and FIX mapping;
- `quarcc-execution-engine`, `quarcc-bunting-adapter`, `quarcc-execution-wasm`: participant execution, venue mapping and Wasm bindings;
- `bunting-agents`, `bunting-runtime`: deterministic built-in participants and scheduling;
- `simfix-wire`, `simfix-session`, `simfix-mapping`: FIX framing, session recovery and application mapping;
- `bunting-api-contract`: shared identity/role types (its browser procedures and `browser-wire` are retired under ADR 0031);
- `bunting-rs`: composition crate and competition archive replay;
- `apps/bunting-server`: venue host (FIX acceptor, admin, writer, durable origin, built-in agents);
- `apps/bunting-cli`: native CLI (TUI, init, replay, scoring);
- `apps/bunting-tui`: Longbridge-derived Ratatui trading terminal and FIX test harness;
- `bindings/*`: C ABI, Python and C++ bindings over `bunting-rs`.

## Checks

```bash
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
! cargo tree --locked -p bunting-engine -e normal | grep -q 'orderbook-rs'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```
