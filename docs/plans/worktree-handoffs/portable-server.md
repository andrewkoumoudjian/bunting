# Portable application and native server handoff

## Branch and base

- Branch: `codex/portable-server`
- Base: `codex/product-contract` at
  `b935000c750ed1944995dc03fe4364971969beb4`
- Product contract: `bunting.product.v1`
- FIX profile: `bunting.fix44.competition.v1`

## Implemented topology

`packages/bunting-application` is the transport-neutral, sans-I/O application
boundary. It verifies the immutable actor/participant binding before recovery,
composes `command-transaction` with engine-owned candidate state and origin
commit, returns only committed events/state, exposes bounded market projections,
and composes `simfix-mapping`, portable QUARCC execution state and the
QUARCC-to-Bunting adapter. It contains no socket, filesystem, Worker, D1, Cache
API or ambient-time type.

```text
native FIX/TCP acceptor -----------------------+
browser-compatible Worker handler ------------+--> bunting-application
Worker outbound FIX Durable Object -----------+      -> bunting-engine
                                                   -> expected-version origin commit
                                                   -> committed result/projection

participant FIX initiator -> external native relay acceptor
Worker FIX initiator ------> external native relay acceptor
                         raw byte-preserving paired session
```

`apps/bunting-server` is a native adapter over that package. Native profiles
load and validate one immutable `ScenarioDefinition`, create the configured run
when it does not exist, accept standard FIX 4.4/TCP, authenticate Logon
credentials/profile/CompIDs, retain bounded FIX and Bunting recovery state,
expose `GET /health` and authenticated `GET /admin/runs/{run_id}`, and execute
commands through the application service. The Cloudflare profile starts the
external relay instead: the relay accepts the participant and Worker-initiated
connections on separate listeners, authenticates the configured bindings,
forwards FIX bytes unchanged with bounded buffers and persists a bounded
directional journal. It never maps application semantics or acknowledges venue
commands.

`apps/bunting-worker` remains authoritative through D1 and uses
`bunting-application` for actor authorization, command preparation and market
projection. Its D1 batch still commits command guard, canonical events,
idempotency result, complete engine recovery state and snapshot metadata before
the response. Workers Cache publication remains best effort after that commit,
and no Worker route accepts inbound raw TCP.

## Configuration profiles

- `apps/bunting-server/config/local.json`: loopback FIX/admin, durable local
  file origin, and immutable scenario bootstrap.
- `apps/bunting-server/config/hosted-native.json`: native hosted service behind
  a mutually authenticated TLS terminator, with loopback plaintext only between
  the terminator and Bunting.
- `apps/bunting-server/config/cloudflare.json`: no inbound Worker FIX listener;
  it configures the external participant/Worker relay.

All configurations are versioned and reject unknown fields. Validation errors
name the invalid field and enforce positive storage/session limits, bounded
message/request sizes, nonzero identities, strong placeholder lengths,
different relay listeners, and the Cloudflare outbound-only constraint. A
hosted non-loopback listener cannot start with TLS disabled.

The native binary does not implement TLS cryptography. In `terminated` mode the
configured trusted proxy must terminate TLS, require client certificates, bind
the Bunting-facing socket to a private or loopback interface, preserve the TCP
byte stream without PROXY/header identity overrides, and pass only peers whose
certificate identity matches the configured FIX binding. FIX Username/Password,
CompIDs and `BuntingProfileVersion(10000)` are still checked by Bunting after TLS
termination.

## Commands

From the repository root:

```bash
export PATH="$HOME/.cargo/bin:$PATH"
export CARGO_TARGET_DIR=/Users/andrewkoumoudjian/Documents/QUARCC/bunting/target

cargo run --locked -p bunting-server -- apps/bunting-server/config/local.json
cargo run --locked -p bunting-server -- apps/bunting-server/config/hosted-native.json
cargo run --locked -p bunting-server -- apps/bunting-server/config/cloudflare.json

curl http://127.0.0.1:8080/health
curl -H 'Authorization: Bearer <configured-token>' \
  http://127.0.0.1:8080/admin/runs/1
```

Replace every credential/path placeholder and provide a serialized, validated
`ScenarioDefinition` before starting a native profile. Configuration and
scenario files are bounded before parsing.

## Storage and restart behavior

`memory` storage is process-local and intended for tests or disposable runs.
`file` storage persists complete engine snapshots, accepted-command
fingerprints/results and canonical events as one bounded document. A mutation
is staged in memory, written to a temporary file, `sync_all` completes, and an
atomic rename installs it before the application service returns. FIX session
sequence/journal state plus portable FIX application/QUARCC state use a separate
atomic snapshot next to the origin file. Therefore process termination after a
returned acknowledgement cannot remove that committed command; restart reloads
both origin and session snapshots before accepting subsequent traffic.

D1 remains the Cloudflare origin. The native file store is not a D1 replacement,
does not coordinate multiple server processes, and must live on one durable
local filesystem. Workers Cache and relay journals are non-authoritative.

## End-to-end evidence

`apps/bunting-server/tests/path_equivalence.rs` maps a FIX NewOrderSingle through
the portable FIX/QUARCC application state and proves it becomes the exact same
canonical command as the Worker application prepare path. Each path commits to
an independent origin and the test compares the complete recovered `RunState`.
The same suite reopens the native file origin after commit and proves the
complete state and canonical event batch survive restart.

The parity test exposed PriceLevel `0.8.4` initializing a derived
`first_arrival_time` statistic from wall time even under OrderBook-rs's
deterministic `StubClock`. `bunting-engine` now pins that diagnostic field to the
deterministic upstream snapshot timestamp and recomputes the standard upstream
snapshot checksum. The new engine regression proves byte-equivalent checksums
for identical books; price/FIFO/matching behavior is unchanged.

Focused verification completed during implementation:

```text
cargo test -p bunting-engine level_statistics_are_canonical_across_identical_books
cargo test -p bunting-server --test path_equivalence
```

Both passed using an isolated focused-test target because another worktree was
actively compiling different package sources into the shared target directory.
Final root checks and Worker release evidence are recorded below before commit.

## Known limitations

- The native FIX slice covers the currently implemented standard order,
  cancellation and market-data mappings; the broader discovery, account, news,
  tender, OTC, asset, reporting, score and administrator `U*` contract remains
  future work already marked incomplete in the corrected implementation plan.
- The native FIX listener and relay currently configure one static identity and
  one active session binding per process. Multi-tenant dynamic pairing and
  certificate-to-tenant registries require a later bounded control-plane design.
- The relay journal proves directional session delivery and remains bounded,
  but authoritative FIX resend journals stay at the participant and Worker FIX
  session endpoints. The relay never synthesizes sequence numbers or gap fills.
- Native file origin is single-process. Run more than one native instance only
  behind a real transactional `OriginStore` adapter.
- The server relies on commit-per-command durability rather than a platform-
  specific signal handler. SIGTERM or process loss closes sockets, while the
  last acknowledged origin/FIX snapshots remain restartable.

## Final verification evidence

The following root-required checks passed from a clean shared target directory:

```text
cargo metadata --locked --format-version 1 --no-deps
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
cargo tree --locked -p bunting-engine | grep -F 'orderbook-rs v0.10.3'
cargo check --locked --workspace --target wasm32-unknown-unknown
git diff --check
```

The workspace test included the native FIX/Worker committed-state equivalence
and durable local restart tests. `worker-build --release` also completed from
`apps/bunting-worker`; it produced the ignored Wrangler shim and optimized Wasm,
and both checked-in D1 migrations remained discoverable under `migrations/`.
