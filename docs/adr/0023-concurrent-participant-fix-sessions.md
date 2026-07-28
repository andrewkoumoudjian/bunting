# ADR 0023: Concurrent participant FIX sessions

- Status: Accepted
- Date: 2026-07-28
- Depends on: ADR 0022

## Context

The implemented native server accepts one FIX connection bound to one participant and one run. A trading competition needs several credentialed participants to interact with the same authoritative market without allowing one slow, malformed or disconnected session to stop the run.

The existing FIX wire and session packages are sans-I/O and already expose bounded parsing, snapshots and restore. The application path is single-writer, so concurrency belongs at the session boundary rather than inside the market engine.

## Decision

One native venue process serves one run with a bounded roster of credentialed participants and concurrent inbound FIX sessions. Each connection owns isolated `simfix-session` state and application mapping state. A bounded asynchronous acceptor supervises connections, while one authoritative writer serializes accepted participant commands through the existing commit-before-acknowledgement transaction.

The configured session cap, message-rate cap, open-order cap and wire bounds are enforced per participant. Rejections name the violated field and configured limit. Disconnects, malformed messages and backpressure are contained to the affected session.

Session snapshots support authenticated reconnect and sequence recovery. Whether resting orders survive disconnect is a separately versioned competition rule and must be decided before the reconnect path is enabled for an event.

## Consequences

Participants share one committed market and see the same public market-data depth and cadence. Command arrival receives a recorded venue sequence before authoritative serialization, so replay does not depend on task scheduling.

The acceptor and session tasks may run concurrently, but matching, ledger mutation and origin commits remain single-writer. Capacity must be explicit because unbounded tasks or queues would turn one participant's flood into a venue outage.

## Rejected alternatives

### One process per participant

Rejected because separate processes cannot share one in-memory market authority and would require an unplanned multi-writer origin protocol.

### One writer per session

Rejected because concurrent writers would make command order and replay depend on host scheduling.

### Put socket lifecycle in `simfix-session`

Rejected because the protocol package is intentionally sans-I/O and reusable across native and hosted transports.

## Validation

- multiple independent FIX clients trade against one run and observe each other's committed liquidity;
- the configured session cap rejects excess connections with the limit named;
- killing or flooding one session leaves other sessions and the run sequence available;
- every accepted command has one monotonic arrival and commit sequence;
- reconnect restores FIX sequence state under the configured reconnect rule;
- no session can mutate the engine except through the authoritative writer.

## Operational impact

Venue configuration gains a roster and explicit per-session capacity limits. Operators can revoke one participant, inspect session state and restart a connection without restarting the run.

## Security impact

Logon credentials bind a connection to exactly one roster entry. Per-session rate, size and queue limits constrain denial-of-service impact, and private reports remain scoped to the authenticated participant.

## References

- [`0022-native-competition-venue-and-publication-worker.md`](0022-native-competition-venue-and-publication-worker.md)
- [`../deployment.md`](../deployment.md)
- [`../../packages/simfix-session`](../../packages/simfix-session)
- [`../../packages/simfix-wire`](../../packages/simfix-wire)
