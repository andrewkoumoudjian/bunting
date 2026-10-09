# ADR 0030: Latency-modeled continuous admission

- Status: **Accepted** (2026-10-09) by owner direction, recorded in ADR 0033.
  **Target — not yet implemented**; the implementation log records each slice.
- Would supersede: ADR 0024 (discrete matching intervals) and ADR 0028 item 7's
  open fairness question.
- Depends on: ADR 0022 (single venue authority), ADR 0029 (owned book), the
  command-sourced journal proposed as Step 3 of
  [the October 9 exploration note](../research/2026-10-09-exploration-and-next-steps.md).

## Context

The owner asked for the **most realistic** market for one and for multiple
venues, using an algorithm that accounts for the real connection distance
between the server and each client.

Real exchanges run continuous price-time matching per venue (plus opening and
closing auctions). Distance matters in reality: a trader nearer a venue sees
its data and reaches its book first, and cross-venue arbitrage exists because
venues are far apart. ADR 0024 instead releases 100 ms batches in arrival
order (observed: `apps/bunting-server/src/writer.rs` sleeps to a wall
boundary, then admits by arrival ticket). That hides distance inside a batch
but still lets the real internet path decide order between batches, and it is
not how any real venue matches.

A remote competition has two kinds of distance:

1. **Physical distance** — the contestant's real internet path to the server.
   It is an accident of where they sit, not part of the market being simulated.
2. **Simulated distance** — where the scenario says each participant and each
   venue are. This is part of the market and should shape outcomes, exactly as
   it does in real markets.

Observed starting points: `simfix-session` already sends FIX TestRequest
(`35=1`, tag 112 `test-{millis}`) and matches the Heartbeat reply
(`packages/simfix-session/src/lib.rs:330-344, 490-491`), but only when the
connection is idle. The server already supports TLS with optional mutual
authentication (`apps/bunting-server/src/config.rs:41`).

## Decision

Matching is **continuous price-time per listing**. Admission order is decided
by a deterministic **latency model** that removes measured physical delay and
adds simulated, scenario-defined delay. Three modes are published per run:

| Mode | Use | Effective venue arrival |
|---|---|---|
| `physical` | Local/LAN events; "real" latency counts | server receive time |
| `equalized` | Remote events; nobody gains from their ISP | estimated client send time + constant |
| `geographic` (default for multi-venue) | Most realistic | estimated client send time + simulated path latency to that venue |

### 1. Measure each connection's one-way delay

For every session the gateway keeps RTT samples from:

- FIX TestRequest/Heartbeat sent **periodically** (e.g. every 1 s, not only
  when idle), with a microsecond monotonic send stamp kept server-side keyed by
  TestReqID;
- the native protocol's `Ping`/`Pong` (proposed ADR 0031);
- optionally the kernel's TCP RTT (`TCP_INFO` `tcpi_min_rtt` on Linux), which a
  client cannot inflate without delaying its own TCP ACKs.

The one-way estimate is half the **windowed minimum** RTT (minimum over the
last *W* samples or seconds), the standard robust filter used by NTP and BBR:
queuing spikes raise individual samples but not the minimum.

```text
d̂(c) = clamp( min_rtt_window(c) / 2 , 0 , D_max )
```

`D_max` (e.g. 150 ms) is published. A connection slower than `2 × D_max` RTT is
disadvantaged by the excess, and that is stated in the rules.

### 2. Compute the effective venue arrival

For command *x* from participant *p* on connection *c*, received at server
monotonic time `t_rx`, addressed to listing on venue *v*:

```text
ŝ        = t_rx − d̂(c)                      # estimated send time
L(p, v)  = scenario path latency + seeded jitter  # 0 in `equalized`
release  = ŝ + D_max + L(p, v)              # never earlier than t_rx
```

Because `d̂(c) ≤ D_max` and `L ≥ 0`, `release ≥ t_rx`: the server never needs a
command before it has arrived.

`L(p, v)` comes from the scenario: each venue and each participant (or
participant tier, e.g. `colocated`, `metro`, `remote`) has a location;
`L = distance / fiber_speed (~5 µs per km) + venue gateway latency + jitter`,
where jitter is drawn from a named, seeded RNG stream per (participant, venue).
Built-in agents have locations too and use the same formula with `d̂ = 0`.

### 3. Sequence

A bounded priority queue ordered by `(release, arrival_sequence)` holds
admitted commands. When the server's monotonic clock reaches the head's
`release`, the writer applies it with `logical_time = release` (mapped to the
run clock). Ordering is consistent: a command arriving after time *r* has
`release ≥ t_rx > r`, so it can never need to jump ahead of one already
released.

### 4. Apply the same model outbound

Execution reports and market data for venue *v* are delivered to participant
*p* so that they arrive at `commit_time + D_max + L(v, p)` in the simulated
world: the server holds each outbound message until
`commit_time + D_max + L(v, p) − d̂(c)`, so every participant sees the same
delay regardless of their real path (in `physical` mode messages are sent
immediately). A
participant far from venue B therefore sees B's book later than a participant
near B — this is what makes cross-venue arbitrage and stale consolidated quotes
realistic. Consolidated NBBO publication uses venue-to-publisher latencies from
the same scenario table.

### 5. Record everything

Each admitted command's journal record carries `t_rx`, `d̂(c)`, mode,
`L(p, v)`, the jitter stream position, `release` and arrival sequence.
Replay uses the recorded `release` order and never re-measures the network, so
replay stays deterministic while live admission uses real measurements.

## Consequences

- Positive: the matching rule is the one real venues use; distance between
  venues and participants is a designed, visible market feature; remote teams
  are not penalized by their ISP in `equalized`/`geographic` modes; the
  ordering policy and every input to it is in the archive.
- Every command waits up to `D_max + L` before matching. With `D_max = 150 ms`
  this is comparable to ADR 0024's 100 ms interval, but fixed per participant
  instead of depending on where in the interval a message lands.
- With outbound equalization, every participant's fastest see-and-react loop
  is about `2 × D_max + L(p, v) + L(v, p)`. Fairness is relative, not absolute:
  choose `D_max` as small as the slowest admitted connection allows (e.g. a
  regional event can use 25 ms), and use `physical` mode for LAN events where
  microsecond realism matters.
- The gateway needs a monotonic microsecond clock, periodic RTT probes, and a
  per-connection outbound hold queue. Both queues are bounded.
- Scenario schema gains venue/participant locations or tiers and a latency
  table. Single-venue runs can use `equalized` with no geography.

## Rejected alternatives

- **Keep discrete 100 ms intervals (ADR 0024):** not how real venues match;
  physical latency still decides order across interval edges.
- **Pure arrival FIFO with no compensation:** realistic only when everyone is
  on the same network; kept as the `physical` mode for that case.
- **Frequent batch auctions:** a respected market-design proposal, but not
  the dominant real mechanism; can be added later as a per-venue policy.
- **Trust client-reported timestamps:** trivially forged.
- **Mean RTT instead of windowed minimum:** inflated by queuing and congestion;
  the minimum is the accepted estimator of propagation delay.

## Validation

1. Unit: the sequencer releases in `(release, arrival_sequence)` order and
   never releases a command before `t_rx`; queue overflow rejects with the
   named limit.
2. Determinism: a recorded live session replayed from the journal produces
   identical events and hash without network access.
3. Equalization: two simulated clients with injected RTTs of 5 ms and 120 ms
   sending at the same true send time reach the book in arrival-sequence order
   in `equalized` mode, and the near client wins in `physical` mode.
4. Geography: with venues A and B 1,000 km apart, a participant colocated at A
   observes A's trade before B's, and the measured A→B arbitrage window equals
   the configured path latency.
5. Anti-gaming: a client that delays its `Pong`/Heartbeat replies gains at most
   `D_max − true one-way delay`; with kernel RTT enabled it gains nothing
   measurable. Record the result.

## Operational impact

Publish the mode, `D_max`, the RTT window, the latency table and jitter
policy before a run. Health output shows each connection's `d̂`, sample count,
queue depths and hold times. An operator must not change the model mid-round.

## Security impact

The main risk is RTT inflation to buy priority; it is bounded by `D_max` and
by preferring kernel-measured RTT where the host exposes it (*unresolved*
whether WASIX does — another input to the host decision). Probes are
rate-limited and bounded; a client that never answers probes uses
`d̂ = 0` (no compensation), never `D_max`.

## References

- [ADR 0024](0024-discrete-matching-interval-fairness.md), [ADR 0028](0028-proposed-headless-run-authority.md) item 7
- [Exploration note, 2026-10-09](../research/2026-10-09-exploration-and-next-steps.md) — G4 and Step 5
- [Expanded market algorithm survey](../research/2026-10-08-expanded-market-algorithm-survey.md) — auction and allocation alternatives
- RFC 5905 (NTP v4) §10 clock filter — minimum-delay sample selection
- Cardwell et al., "BBR: Congestion-Based Congestion Control", ACM Queue 14(5), 2016 — windowed min-RTT estimation
