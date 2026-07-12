# ADR 0012: Asynchronous Dynamic Worker strategy dispatch

- Status: Accepted
- Date: 2026-07-11

## Context

Bunting allows users to submit Python strategies. The strategy must receive market/fill/timer inputs and return state plus proposed actions, but it is untrusted, may be slow or fail, and cannot be allowed to block or mutate the authoritative market transaction.

Cloudflare Dynamic Workers provide isolated runtime execution through a Worker Loader binding. The API supports `load()` for one-off execution and `get(id, callback)` for reusable immutable code identities. Isolate reuse is an optimization and is not guaranteed. Dynamic Workers inherit network access unless `globalOutbound` is explicitly restricted. Custom limits currently cover CPU milliseconds and subrequests; Bunting must enforce all other bounds itself.

The architecture therefore needs explicit dispatch, idempotency, state ownership, sandboxing, replay and failure behavior.

## Decision

### Asynchronous topology

The `MarketRun` object does not synchronously execute untrusted Python inside command processing.

1. The run commits `StrategyInvocationRequested` with an immutable invocation ID, trigger, input event range/hash, state revision/hash and deadline.
2. It enqueues a bounded invocation request to a Cloudflare Queue after the commit.
3. `workers/strategy-dispatch-consumer` receives the request and invokes `services/strategy-loader` through a service binding.
4. The loader creates/reuses the isolated Dynamic Worker and returns a bounded typed result.
5. The consumer submits an authenticated internal `StrategyInvocationResult` command to the run object.
6. The run validates invocation ID, state revision, deadline, output limits and every proposed action.
7. The accepted state transition and actions are committed through the ordinary event/risk/matching pipeline.

Queue delivery is at-least-once. The invocation ID and state revision make request/result processing idempotent.

### Worker identity

Recurring strategy code uses `LOADER.get(id, callback)`, not `load()`.

The ID is derived from immutable content/configuration:

```text
user source hash
wrapper version
SDK version
compatibility date
compatibility flags
limits profile version
module manifest hash
```

The callback returns byte-identical `WorkerCode` for the same ID. Any source, wrapper, SDK, flag or limits-profile change creates a new ID.

The implementation never assumes two calls use the same isolate.

### Python runtime

The loaded Worker includes the `python_workers` compatibility flag and a fixed outer wrapper. The wrapper accepts bounded serialized requests and invokes one of:

- `on_start`;
- `on_market`;
- `on_fill`;
- `on_timer`.

The wrapper validates returned shape before sending it to the parent. User code cannot replace the parent-side Rust validation or submit directly to a market endpoint.

### Explicit state

Dynamic Worker global memory is non-authoritative. Strategy state is explicit, versioned input/output owned by the run aggregate.

Initially:

- one invocation per strategy state revision may be in flight;
- new market updates are coalesced for the next invocation while one is pending;
- stale, duplicate or expired results cannot mutate state;
- only the first valid result for an invocation ID is accepted;
- state size and nesting are bounded;
- state hashes are recorded.

Dynamic Worker Durable Object Facets are not used for initial strategy state. Their independent SQLite storage would create a second authority and complicate replay, migrations and participant fairness.

### Sandbox

Every loaded strategy has:

- `globalOutbound: null`;
- no direct D1, R2, KV, Queue, Durable Object, secret, token or credential binding;
- no reusable order-submission capability;
- explicit `cpuMs` and `subRequests` limits;
- source/module/input/output/state/action/log byte and count limits enforced by Bunting;
- a fixed compatibility date and flags;
- bounded structured errors.

If future strategies need a capability, it must be a narrow custom RPC binding with a new security review and ADR. No generic storage or network binding is implied by this decision.

### Observability

The loader attaches a Tail Worker to capture console output, exceptions and request metadata. Tail records include worker ID, invocation ID, tenant/run/strategy identifiers where permitted and the limits profile.

Tail output is operational telemetry and may arrive after the result. It cannot change canonical market state. Bounded user-visible logs returned by the wrapper are separately validated and streamed on `strategy.logs`.

### Replay

Dynamic strategy execution is treated as external input.

Canonical records include:

- invocation request and trigger;
- source/wrapper/SDK/runtime/limits versions;
- input range/hash;
- prior state revision/hash;
- deadline and result status;
- accepted result and next-state hash;
- generated action commands and their canonical outcomes.

Replay never calls the loader. It applies the recorded accepted result/actions. A replay environment must succeed with Dynamic Workers disabled.

### Clock-mode support

Initial live Dynamic Worker support is limited to lockstep and paced modes.

- Lockstep invokes all eligible strategies from the same published decision state, applies a deterministic deadline and orders accepted participant actions according to the run's documented fairness rule.
- Paced mode treats accepted results as external commands at their canonical arrival order.
- New unrestricted accelerated runs use built-in pure Rust agents or recorded external actions until asynchronous strategy checkpoints receive a separate fairness/load review.

### Failure policy

Load error, exception, timeout, CPU/subrequest limit, malformed output, oversized output or stale result creates a typed failure event. Strategy state remains unchanged and no proposed action is accepted.

A versioned policy may warn, pause or disable a repeatedly failing strategy. The market run continues.

## Consequences

Positive:

- untrusted Python does not block the authoritative transaction;
- duplicate Queue deliveries are safe;
- state ownership and replay remain in the market run;
- stable `get()` identities improve reuse and avoid one creation per invocation;
- denied egress and no bindings create a narrow capability surface;
- operational logs are available without making them authoritative.

Negative:

- invocation results arrive asynchronously;
- another Queue consumer Worker is required;
- lockstep needs explicit deadlines and fairness ordering;
- Python startup/runtime cost remains material;
- the initial accelerated mode does not execute new live Dynamic Worker strategies.

## Rejected alternatives

### Execute the loader synchronously inside the market transaction

Rejected because untrusted CPU, cold starts and failures would delay the authoritative sequencer and persistence.

### Call `load()` for every invocation

Rejected because it creates a fresh Worker each time and prevents stable code reuse. It also increases Dynamic Worker creation billing.

### Rely on isolate-global Python state

Rejected because `get()` does not guarantee that later requests use the same isolate.

### Store strategy state in a Dynamic Worker Facet

Rejected initially because it creates a second independently migrated and replayed state authority.

### Give a strategy a Durable Object or order-service binding

Rejected because it could bypass validation, risk, idempotency and canonical ordering.

### Re-execute Python during replay

Rejected because runtime/package behavior and external execution timing are not part of the pure deterministic kernel.

## Validation

- the same worker ID always resolves to identical code/configuration;
- any code/configuration change changes the ID;
- tests pass when isolate reuse never occurs;
- `fetch()` and `connect()` from user code fail;
- no secret or storage capability is visible;
- CPU/subrequest and Bunting-owned limits terminate abusive inputs;
- duplicate request/result delivery is idempotent;
- stale state revisions and expired results reject;
- failure leaves strategy state, book, risk and ledger unchanged;
- accepted actions go through ordinary authorization/risk/matching;
- Tail logs are correlated but non-authoritative;
- full run replay succeeds without invoking Dynamic Workers.

## References

- Cloudflare Dynamic Workers getting started and API reference
- Cloudflare Dynamic Workers bindings, egress control, custom limits, observability and pricing documentation
- `docs/adr/0007-dynamic-worker-loader-boundary.md`
- `docs/core-implementation-questions.md`
