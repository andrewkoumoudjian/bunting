# ADR 0024: Discrete matching intervals for competition fairness

- Status: Accepted
- Date: 2026-07-29
- Depends on: ADR 0022, ADR 0023

## Context

A remotely accessible competition cannot make network distance equal. Processing
orders immediately on socket arrival would turn host and route latency into an
unpublished scoring input, while sleeping independently in each session would
still let task scheduling determine priority.

The venue already has one authoritative writer and a replayable logical clock.
That boundary can assign an arrival sequence and release a bounded batch at a
documented cadence without changing the sans-I/O FIX codec or session packages.

## Decision

Competition runs use discrete matching intervals. Every authenticated
participant command receives a monotonic arrival sequence at the venue
boundary, enters a bounded shared interval queue, and is released to the single
authoritative writer in arrival-sequence order when the interval closes.

The default interval is 100 milliseconds and the configured value is published
before a round. All participants share one market, interval schedule, public
depth and publication cadence. The venue enforces per-session message-rate,
open-order and queue limits before admission; every rejection names the field
and configured limit.

Resting orders survive a transport disconnect. A reconnect restores the
participant's FIX sequence state and application mapping state, while the
authoritative book remains unchanged. Disconnect never implies cancel-on-behalf.

## Consequences

Small network-latency differences inside an interval do not change the batch
release time, but arrival order remains deterministic inside the batch. The
interval adds bounded latency and must be included in practice and production
profiles so agents can use the same timing contract.

The queue and interval clock become replay inputs. Operators must halt rather
than silently change the interval during an active round.

## Rejected alternatives

### Immediate matching on socket arrival

Rejected because network distance and operating-system scheduling would affect
priority without being part of the published competition rules.

### Per-session artificial delay

Rejected because independent timers do not establish one deterministic global
order and are vulnerable to scheduler jitter.

### Co-location as the only fairness control

Rejected because it excludes remote teams and still leaves host scheduling and
connection placement as hidden variables.

## Validation

- concurrent clients submitting within one interval are committed in recorded
  arrival-sequence order;
- all clients observe the same depth after an interval closes;
- the interval queue rejects overflow with `max_interval_queue` and its limit;
- replay reproduces interval boundaries and canonical event bytes;
- reconnect preserves FIX sequence state and resting orders.

## Operational impact

Profiles declare the interval duration and queue bound. Venue health exposes
the current interval, queue depth and rejected-message counts, and the run
archive records the effective values.

## Security impact

Bounded queues and per-session admission limits contain floods before they
reach market authority. Credentials remain participant-scoped, and no client
can select its own interval or publication cadence.

## References

- [`0022-native-competition-venue-and-publication-worker.md`](0022-native-competition-venue-and-publication-worker.md)
- [`0023-concurrent-participant-fix-sessions.md`](0023-concurrent-participant-fix-sessions.md)
- [`../hackathon-base-plan.md`](../hackathon-base-plan.md)
- [`../plans/competition-platform-reorganization.md`](../plans/competition-platform-reorganization.md)
