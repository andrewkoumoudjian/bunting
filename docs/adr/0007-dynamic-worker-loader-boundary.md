# ADR 0007: Minimal TypeScript Dynamic Worker loader boundary

- Status: Accepted
- Date: 2026-07-11

## Context

Users must be able to paste Python strategies and run them against a live simulation. Untrusted strategy code must not execute inside the authoritative `MarketRun` Durable Object. Cloudflare Dynamic Workers provide isolated runtime instances and Python Worker support, but the current Loader API is documented through JavaScript/TypeScript bindings rather than a stable native `workers-rs` interface.

Forcing an unsupported Rust FFI layer would increase risk and couple the project to undocumented runtime behavior.

## Decision

Keep the market kernel, edge API, run coordinator and validation path in Rust. Add one minimal TypeScript service, `services/strategy-loader`, whose only responsibilities are:

1. resolve immutable strategy source by content hash;
2. call the Cloudflare Worker Loader API;
3. configure a Python Worker compatibility date and flags;
4. set `globalOutbound` to `null`;
5. provide only narrow, non-secret bindings;
6. apply CPU, subrequest, source-size, result-size and invocation limits;
7. invoke a fixed strategy wrapper contract;
8. return a bounded typed result to the Rust caller.

The Python strategy never receives direct Durable Object, D1, R2, KV, Queue or credential bindings. It receives sanitized market batches and explicit serializable state, and returns proposed actions. The Rust parent validates every action and routes it through normal authorization, risk, matching and persistence.

Strategies are invoked on bounded coalesced batches, fills and timers rather than every book mutation.

## Strategy contract

The wrapper exposes callbacks such as:

- `on_start(context)`;
- `on_market(context, state, events)`;
- `on_fill(context, state, fill)`;
- `on_timer(context, state, timer)`.

Each callback returns bounded `state`, `actions` and `logs` fields. Source hash, wrapper version, SDK version, input sequence range, output actions, failures and runtime version are recorded for reproducibility.

## Consequences

Positive:

- untrusted code is isolated from authoritative state;
- the unsupported Rust Loader gap is contained in one replaceable service;
- network egress can be denied;
- Python remains a user-facing language without infecting matching logic;
- strategy execution can evolve independently.

Negative:

- one small non-Rust service is required;
- strategy invocation introduces latency and cost;
- user state must be explicitly serialized;
- exact reproducibility depends on pinning Python/runtime/package behavior.

## Rejected alternatives

### Execute Python in the Durable Object

Rejected because untrusted CPU, imports and failures could block the market sequencer.

### Give strategy Workers direct order or storage bindings

Rejected because it bypasses validation and weakens tenant isolation.

### Invent a Worker Loader binding in Rust immediately

Rejected until Cloudflare provides a stable supported API or the project can justify a narrowly reviewed generated binding.

### Run user Python in the browser

Rejected as the sole mechanism because results would be easier to tamper with and could not be scored authoritatively.

## Validation

- attempted `fetch()` and raw socket egress fail;
- no secrets or storage bindings are visible;
- CPU and output limits terminate abusive code;
- malformed actions are rejected by Rust;
- identical source, input batch, explicit state and runtime version produce recorded comparable outputs;
- strategy failures do not stop or corrupt a run.

## References

- Cloudflare Dynamic Workers: https://developers.cloudflare.com/dynamic-workers/getting-started/
- Egress control: https://developers.cloudflare.com/dynamic-workers/usage/egress-control/
- Python Workers: https://developers.cloudflare.com/workers/languages/python/
