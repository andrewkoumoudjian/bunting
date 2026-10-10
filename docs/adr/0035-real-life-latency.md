# ADR 0035: Real-life latency — real connectivity counts, venues are distant

- Status: **Accepted** (2026-10-10) by owner direction; implemented in
  slice 16 (see `docs/implementation-log/`).
- Supersedes: **ADR 0034** in full, and ADR 0030's admission modes
  (`physical | equalized | geographic`), `D_max` and outbound equalization.
  ADR 0030's continuous price-time matching, `(release, arrival)` sequencer
  (§3) and journaled admission inputs (§5) stand.
- Depends on: ADR 0022, ADR 0029, slices 14–15 (journal, sequencer).

## Context

The owner's direction for competitions, in order, on 2026-10-10:

1. Teams choose the venue for every order. Each venue is a different
   virtual distance from each trader, so teams must choose carefully,
   account for information arriving late from distant venues, and protect
   resting orders from being filled by someone who sees another venue's
   price first.
2. In a hackathon where everyone is on the same network, real distance —
   measured from handshake and response delay — should count, so a slow
   TCP implementation or connection method is penalized, just as for
   traders in New York reaching TSX, BATS and NYSE.
3. "This shouldn't be a mode; this should be a real-life simulation":
   supersede any other setting with what most resembles real life.

ADR 0034 did the opposite of (2) and (3): it cancelled real network delay
and offered several modes. In real markets nobody cancels a trader's
latency: their network stack, connection method, uplink and physical
distance all count, and each exchange sits somewhere else, so the distance
from the desk to each venue adds to it in both directions.

## Decision

One latency model, no modes and no equalization.

### 1. What reaches a venue, when

```text
inbound:   release(x)  = t_rx(x) + L(p, v)
outbound:  send_at(m)  = t_venue(m) + L(v, p)
```

- `t_rx` is when the team's bytes reach the server, stamped by the
  connection's reader thread the instant they arrive. It already contains
  the team's **real** delay: distance, ISP, Wi-Fi or wired, VPN or proxy
  hops, TCP stack, Nagle, client processing.
- `L(p, v)` is the **virtual** distance between the team's location and
  venue `v`, from the run's published latency table (fixed part plus seeded
  jitter), used in both directions. An empty table means every venue is in
  one data centre with every team.
- Outbound venue messages (execution reports, market data responses) leave
  the server `L(v, p)` after the venue produced them; the team's real delay
  then applies on the wire. Session messages (heartbeats, probes) are never
  delayed.
- Ordering among orders to the same venue from one connection is preserved
  (one path is FIFO); orders to different venues follow their own paths, so
  an order to a near venue can overtake an earlier one to a far venue.

### 2. Measurement is published, never compensated

The venue still measures each connection's real delay — the kernel's TCP
minimum RTT (handshake and ACK timing, read through netlink `sock_diag` on
Linux) and FIX TestRequest round trips — and publishes it as the team's
access latency (`/admin/admission`, journaled with every admission). It
never changes ordering. Because nothing is compensated, inflating a
measurement can only hurt the team that does it; the whole class of
latency-gaming attacks in ADR 0034 §6 disappears.

### 3. What rewards a good implementation

Everything a real trading firm optimizes now matters, and nothing else:
fewer network hops, a wired connection, `TCP_NODELAY` (a client that leaves
Nagle on was measured waiting ~40 ms for the server's delayed ACK in slice
16's tests), a client that reads its socket continuously, pipelining, fast
parsing and decisions, and choosing the right venue for each order given
its virtual distance.

### 4. Server obligations (unchanged from ADR 0034 §4)

`TCP_NODELAY` on server sockets; stamp on read in a dedicated reader;
admission never blocks a session; one live FIX session per participant; the
FIX dictionaries load once per process.

## Consequences

- Teams far from the server (physically or by connection method) are slower,
  as in real life. Organizers who want a level field for remote teams should
  host close to them or run the event on one network; the venue does not
  hide distance.
- Multi-venue scenarios become strategic: venue choice, stale quotes from
  distant venues, and resting orders exposed to faster traders elsewhere.
- Simpler configuration: the latency table plus probe/queue bounds.
- The rules state the model plainly (`RULES.md`).

## Rejected alternatives

- **Equalization (ADR 0034)**: hides the connectivity differences the owner
  wants to count, and invites latency inflation.
- **Selectable modes**: the owner asked for one real-life model.
- **Adding measured delay to `L` explicitly**: real delay is already inside
  `t_rx` and the wire; adding it again would double-count.

## Validation

`packages/admission-sequencer` tests (real delay and venue distance add,
measurement never changes order, nearer team wins at its venue, jitter
determinism, sequencer bounds) and end-to-end `tests/real_latency.rs` (a
team behind a 30 ms-each-way connection loses although it sent 5 ms
earlier; 30 ms real + 0 virtual beats ~0 real + 50 ms virtual) and
`tests/venue_distance.rs` (with two venues, each team wins at the venue it
is near although it sent 10 ms later; a far venue's data arrives later by
the virtual round trip).
