# ADR 0002: One authoritative Durable Object per market run

- Status: Accepted
- Date: 2026-07-11

## Context

A simulated exchange needs a total order across participant commands, background agents, matching, risk, positions, scenario events and the simulation clock. Partitioning books too early creates distributed ordering, cross-instrument risk and replay problems.

Cloudflare Durable Objects provide single-object coordination, persistent storage and WebSocket management. The official Rust bindings expose named object lookup, SQLite storage and hibernatable WebSocket APIs.

## Decision

Create one `MarketRun` Durable Object for each `(tenant_id, run_id)`.

The object is the sole authority for:

- run lifecycle;
- event sequence;
- logical clock;
- instrument books;
- participant accounts, positions and risk state;
- open orders;
- scenario agents and deterministic PRNG state;
- scheduled scenario events;
- native WebSocket subscribers;
- FIX sessions and resend journals;
- scoring state.

All mutating public requests are authenticated and validated by the edge Worker, then routed to the named run object. The object serializes commands through a single command pipeline.

Instrument-level sharding is deferred until measured limits prove a single run object is insufficient.

## Consequences

Positive:

- deterministic total ordering;
- atomic cross-instrument risk within a run;
- simple event sequencing and replay;
- no distributed matching races;
- straightforward lockstep barriers;
- a natural WebSocket fan-out owner.

Negative:

- one run has a finite per-object CPU and storage throughput envelope;
- long-running hot runs will not hibernate;
- multi-object scaling requires a future protocol if one object becomes insufficient.

## Operational rules

- Object names are stable and derived from tenant and run identifiers.
- Location hints may be selected at run creation but are not correctness inputs.
- All buffers are bounded.
- CPU-heavy accelerated simulations process bounded batches and schedule continuation.
- Alarms pace work but never define logical ordering.
- Hibernation attachments store only connection metadata, not canonical market state.

## Rejected alternatives

### One Durable Object per instrument

Rejected for the first architecture because participant risk, multi-instrument agents, scoring, clocks and deterministic replay would require distributed coordination.

### One global exchange object

Rejected because unrelated runs would contend and tenant isolation would weaken.

### Stateless Workers plus D1

Rejected because D1 is not a per-run in-memory sequencer and WebSocket coordinator.

## Validation

- concurrent commands receive one monotonic event sequence;
- object reconstruction restores the same state checksum;
- cross-instrument risk changes are atomic within one command;
- slow WebSocket clients cannot block matching;
- load tests establish a documented safe run envelope before public launch.

## References

- `ref/workers-rs/worker/src/durable.rs`
- `ref/workers-rs/worker/src/websocket.rs`
- Cloudflare Durable Objects WebSocket guidance: https://developers.cloudflare.com/durable-objects/best-practices/websockets/
- Cloudflare SQLite-backed Durable Object storage: https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/
