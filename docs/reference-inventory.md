# Reference implementation inventory

This inventory records the role of each source in Bunting. Reference code is evidence, not production code.

## Pinned external repositories

| Local path | Upstream | Pinned commit | Intended use |
|---|---|---:|---|
| `ref/workers-rs` | `cloudflare/workers-rs` | `5f2d6c9192377451d43910098738624474196364` | Worker runtime APIs, Durable Objects, SQLite, WebSockets, Queues, RPC, examples, build conventions |
| `ref/nbc-hft-simulation` | `carterj-c/NBC_HFT_Simulation` | `35b8050546679547dc737198ea13aa0ec8ed7db8` | Legacy REST/WebSocket contract, lockstep `DONE`, participant clients, scenario names, golden compatibility fixtures |
| `ref/ironfix` | `joaquinbejar/IronFix` | `6ac17a37f7c4efbdfe97a06f428809733d88b66b` | Candidate source for FIX core types, dictionary and zero-copy tag-value codec; session algorithms require Worker-specific extraction |
| `ref/fixer` | `fixer-rs/fixer` | `c1c27c3287d6f275a9c33122cc2af063de7c5a08` | Native FIX interoperability oracle and bridge reference; not a Worker dependency because the engine is Tokio/native-I/O coupled |
| `ref/ferrumfix` | `ferrumfix/ferrumfix` | `ca2bbe4c6461108646f35f7cc9245bf1848ec368` | FIX layering and conformance reference; upstream explicitly describes the project as unstable |
| `ref/nautilus-trader` | `nautechsystems/nautilus_trader` | `c28b1335c95abbf1bef2385def9a75a1b3862f76` | Current adapter architecture, normalized domain mappings, reconciliation and test patterns |
| `ref/wirefilter` | `cloudflare/wirefilter` | `61936e5f38523df3f80880bbc662e490b52e7f86` | Conditional scenario-rule engine candidate after Wasm size, latency and security evaluation |

The `workers-rs` submodule already contains its official `examples/` directory; it is not duplicated separately.

## Existing in-repository references

### `ref/quarcc-trading-engine`

C++ trading engine. Useful concepts:

- sequential order-manager event dispatch;
- execution gateway, journal, order-store and market-feed interfaces;
- local/broker order identifier mapping;
- deferred fill handling;
- position keeping;
- SQLite journal and order-store tests;
- kill-switch workflow;
- protobuf contracts.

Do not port directly:

- `std::jthread`, mutex and native event-queue architecture;
- gRPC server;
- native SQLite implementation;
- wall-clock acquisition from domain code;
- floating-point order quantities and prices;
- race-recovery mechanics caused by separate gateway and dispatch threads.

The Durable Object supplies a single authoritative sequencer, so the Rust port should preserve business semantics while removing thread races and native-service assumptions.

### `ref/nbc_engine`

Compiled Java simulator assets and calibrated scenario definitions. Useful concepts:

- normal, stressed, flash-crash, mini-flash-crash and HFT-dominated scenarios;
- fundamental, momentum, noise, market-making, institutional and spiking agents;
- deterministic seeds and step intervals;
- legacy application configuration.

The repository snapshot does not expose a complete reviewed Java source tree. Scenario files are therefore behavioral inputs, not proof of implementation correctness.

### `ref/ritc_mm`

Rust RITC strategy and adapter reference. Useful concepts:

- RIT API models;
- Avellaneda–Stoikov quoting;
- queue imbalance;
- GARCH volatility;
- spectral order-flow analysis;
- market-making configuration and calibration artifacts.

Do not import as engine code without redesign. The current implementation uses blocking `reqwest`, Tokio, native WebSockets, threads, wall-clock sleeps and floating point. These are acceptable for a native strategy client but violate the deterministic Worker kernel boundary.

## Porting rule

Every port must document:

1. upstream file and commit;
2. upstream license;
3. behavior retained;
4. behavior rejected;
5. new Bunting abstraction;
6. Wasm compatibility;
7. equivalence and property tests;
8. local patches and divergence.
