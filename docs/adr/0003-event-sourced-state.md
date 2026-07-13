# ADR 0003: Event-sourced authoritative run state

- Status: Accepted for event sourcing; Durable Object and FIX ownership superseded by ADR 0013 and ADR 0016
- Date: 2026-07-11

## Context

Bunting must support deterministic replay, auditability, score verification, recovery after Durable Object reconstruction and multiple external protocols. The C++ reference separates a journal and an order store, but it performs several state writes around a gateway call and includes race-handling for fills arriving before local/broker identifier mapping.

Inside one Durable Object, commands can be sequenced before effects are exposed. A canonical event log can therefore be the single source from which books, ledgers, private state and public streams are derived.

## Decision

Use event sourcing for every market run.

Command processing order:

1. authenticate actor context;
2. validate schema, run status and idempotency;
3. execute deterministic risk and matching logic against current state;
4. derive canonical events;
5. persist the accepted event batch and idempotency result atomically in Durable Object SQLite;
6. apply events to in-memory state;
7. acknowledge the command;
8. emit private and public stream projections;
9. enqueue asynchronous exports and analytics.

No accepted order is acknowledged before its canonical events are durable.

Persist:

- monotonic event sequence;
- typed, versioned event payloads;
- correlation and causation identifiers;
- logical time;
- actor identity;
- periodic checksummed snapshots;
- idempotency records;
- scheduled-event status;
- FIX session and outbound resend journals.

## Snapshot and recovery

A snapshot contains all deterministic state required to continue a run, including books, ledgers, agents, logical clock, run configuration hash and PRNG state. Recovery loads the newest valid snapshot and replays later events through the same event-application functions used during normal processing.

Snapshots are accelerators, not independent truth. Corrupt or incompatible snapshots are rejected and recovery falls back to an earlier snapshot or full replay.

## Consequences

Positive:

- exact replay and audit trails;
- protocol-independent facts;
- resilient reconstruction;
- reproducible scoring;
- easier debugging of scenario behavior.

Negative:

- schema evolution requires discipline;
- event volume can be large;
- snapshot cadence and archive compaction require tuning;
- event application code must remain backward compatible or support migrations.

## Rejected alternatives

### Mutable tables only

Rejected because reconstructing causality, replaying scenarios and verifying scores would be difficult.

### Queue as the event log

Rejected because Queues are asynchronous and at-least-once; they are suitable for derived work, not authoritative acceptance.

### Emit before persist

Rejected because clients could observe an accepted order that disappears after reconstruction.

## Validation

- direct execution and snapshot-plus-replay produce identical state hashes;
- duplicate commands return the original result without creating events;
- duplicate Queue delivery does not duplicate exports or scores;
- event schema compatibility tests cover every released version;
- injected persistence failure produces no acceptance acknowledgement.

## References

- `ref/quarcc-trading-engine/engine-cpp/include/trading/interfaces/i_journal.h`
- `ref/quarcc-trading-engine/engine-cpp/src/core/order_manager.cpp`
- Cloudflare Queues delivery guarantees: https://developers.cloudflare.com/queues/reference/delivery-guarantees/
