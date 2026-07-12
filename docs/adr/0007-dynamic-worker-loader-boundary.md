# ADR 0007: Minimal TypeScript Dynamic Worker loader boundary

- Status: Accepted
- Date: 2026-07-11
- Superseded in part by: ADR 0012 for asynchronous dispatch, state and replay

## Context

Users must be able to submit Python strategies and run them against a live simulation. Untrusted strategy code must not execute inside the authoritative `MarketRun` Durable Object.

Cloudflare Dynamic Workers provide isolated runtime instances and Python Worker support. The Worker Loader API is documented through JavaScript/TypeScript bindings rather than a stable native `workers-rs` interface. Forcing an unsupported Rust binding would increase risk and couple Bunting to undocumented runtime behavior.

Current platform behavior relevant to Bunting:

- `load(code)` creates a fresh Dynamic Worker;
- `get(id, callback)` may reuse immutable code identities across requests but does not guarantee the same isolate;
- Python requires the `python_workers` compatibility flag;
- Python Workers have materially slower cold starts than JavaScript Workers;
- omitting `globalOutbound` normally inherits parent Internet access;
- `globalOutbound: null` blocks both `fetch()` and `connect()`;
- custom limits currently cover CPU milliseconds and subrequests;
- Tail Workers can capture logs, exceptions and request metadata;
- custom bindings are capabilities and must be granted explicitly.

## Decision

Keep the market kernel, Edge API, run coordinator and validation path in Rust. Add one minimal TypeScript service, `services/strategy-loader`, whose only responsibilities are:

1. resolve immutable strategy and wrapper modules by content hash;
2. derive/verify an immutable worker ID;
3. call `env.LOADER.get(id, callback)` for recurring strategies;
4. configure the pinned compatibility date and `python_workers` flag;
5. set `globalOutbound` to `null` explicitly;
6. expose no storage, secret, Queue, Durable Object or credential bindings;
7. apply versioned CPU/subrequest limits;
8. enforce source/module/input/output/state/action/log bounds owned by Bunting;
9. invoke a fixed strategy wrapper contract;
10. attach a Tail Worker for operational logs and exceptions;
11. return a bounded typed result to the trusted dispatcher.

The callback passed to `get()` must return byte-identical `WorkerCode` for one ID. The ID includes user source hash, wrapper/SDK versions, compatibility date/flags, limits profile and module manifest hash. Any change creates a new ID.

The loader does not assume warm-isolate persistence. Python globals are non-authoritative.

## Invocation boundary

The Python wrapper receives sanitized market batches and explicit serializable state. It returns proposed state, actions and bounded user logs.

Callbacks include:

- `on_start(context, state)`;
- `on_market(context, state, events)`;
- `on_fill(context, state, fill)`;
- `on_timer(context, state, timer)`.

The wrapper may be exposed through the default Python `WorkerEntrypoint.fetch()` for the initial implementation. The transport is internal and versioned.

The strategy never receives direct order-submission, Durable Object, D1, R2, KV, Queue or credential capabilities. Every returned action is schema validated by trusted code and submitted through normal authorization, idempotency, risk, matching and persistence.

## Dispatch

The loader is not called inside the authoritative command transaction. ADR 0012 defines the asynchronous path:

```text
MarketRun committed request -> Queue -> strategy-dispatch-consumer
-> TypeScript loader -> authenticated result command -> MarketRun
```

This keeps untrusted cold starts, CPU and failures away from the sequencer transaction.

## State and persistence

Strategy state is explicit input/output owned by the run aggregate. Dynamic Worker Durable Object Facets are not used initially. Isolate-global state cannot affect correctness.

Source hash, wrapper/SDK/runtime/limits versions, input sequence range/hash, state revision/hash, output, accepted actions, failures and next-state hash are recorded. Replay does not execute Python.

## Consequences

Positive:

- untrusted code is isolated from authoritative state;
- the unsupported Rust Loader gap is contained in one replaceable service;
- network egress is denied explicitly;
- Python remains user-facing without infecting matching logic;
- stable worker IDs permit reuse and reduce avoidable creation cost;
- Tail Worker observability is available;
- strategy execution can evolve independently.

Negative:

- one small non-Rust service and one dispatch consumer are required;
- strategy invocation introduces latency and cost;
- user state must be explicitly serialized;
- exact reproducibility requires pinning and recording runtime/source/wrapper behavior;
- Python cold starts remain slower than JavaScript.

## Rejected alternatives

### Execute Python in the Durable Object

Rejected because untrusted CPU, imports and failures could block the market sequencer.

### Give strategy Workers direct order or storage bindings

Rejected because it bypasses validation and weakens tenant isolation.

### Omit `globalOutbound`

Rejected because the Dynamic Worker would normally inherit public network access from the parent.

### Use `load()` for recurring strategy calls

Rejected because it creates a fresh Worker per invocation and prevents stable identity reuse.

### Depend on isolate globals

Rejected because `get()` does not guarantee reuse of the same isolate.

### Store initial strategy state in Dynamic Worker Durable Object Facets

Rejected because it creates a second state authority with separate migrations and replay semantics.

### Invent a Worker Loader binding in Rust immediately

Rejected until Cloudflare provides a stable supported API or the project justifies a narrowly reviewed generated binding.

### Run user Python in the browser

Rejected as the sole mechanism because results would be easier to tamper with and could not be scored authoritatively.

## Validation

- same ID always returns identical WorkerCode;
- code/config changes create a new ID;
- tests do not rely on isolate reuse;
- attempted `fetch()` and raw socket `connect()` fail;
- no secrets or storage bindings are visible;
- CPU/subrequest and Bunting byte/count limits terminate abusive code;
- malformed actions are rejected by trusted validation;
- duplicate dispatch/result delivery is idempotent;
- strategy failures do not stop or corrupt a run;
- replay succeeds with the loader disabled;
- Tail Worker logs are correlated but non-authoritative.

## References

- Cloudflare Dynamic Workers getting started
- Cloudflare Dynamic Workers API reference
- Cloudflare Dynamic Workers egress control
- Cloudflare Dynamic Workers bindings
- Cloudflare Dynamic Workers custom limits
- Cloudflare Dynamic Workers observability
- Cloudflare Dynamic Workers pricing
- Cloudflare Python Workers
- `docs/adr/0012-asynchronous-strategy-dispatch.md`
