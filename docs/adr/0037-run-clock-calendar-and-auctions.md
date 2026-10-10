# ADR 0037: One run clock, a trading calendar, and opening and closing auctions

- Status: **Accepted** (2026-10-10) as the design for Step 6 (exploration
  note §8, roadmap Slice 4). Implementation is staged; each stage is
  **Target** until `docs/implementation-log/` records it:
  1. one run clock and a venue timer (**implemented**, slice 26);
  2. the calendar and session phases, with DAY expiry at the close;
  3. opening and closing call auctions;
  4. end-of-day marks and multi-day runs.
- Date: 2026-10-10
- Depends on: ADR 0029 (engine-owned book), ADR 0030 §3 (the sequencer is
  the only committer), ADR 0035 (latency), ADR 0028 (single ledger, full
  replay).
- Supersedes: nothing. It replaces the "two clocks" known trap in
  `AGENTS.md` once stage 1 lands.

## Context

The owner asked for the most realistic venues for both the QUARCC
competition and the classroom. Real venues trade on a calendar: a
pre-open in which orders collect without matching, an opening auction, a
continuous session, a closing auction, then a close at which day orders
expire and the official closing price becomes the day's mark. Runs of
several days carry positions and good-till-cancel orders overnight.

Bunting has none of this (gap G9). A 2026-10-10 survey of the code found:

- **Three clocks.** Order and cancel commands are stamped with venue
  epoch nanoseconds at release (`VenueClock::logical_time`); simulation
  and competition commands use the run clock `simulation.clock.now`, which
  starts at 0 and moves only on an operator `Advance`; the agent runtime
  keeps a private counter that the server overwrites. All three feed one
  GTD expiry index, and the engine checks no order's time against the run
  clock.
- **No timer.** Nothing applies a due scheduled action, tender expiry or
  GTD expiry unless some input happens to arrive.
- **No session.** `TimeInForcePolicy::Day` is accepted but never expires;
  halts are per instrument and do not cancel resting orders; there is no
  auction code. Marks are the last trade price.

## Decision

### 1. One run clock (stage 1)

- `simulation.clock.now` is the run's only time. Every committed input
  carries the run time at which the venue applied it, and the engine moves
  the clock to it. A command whose time is behind the clock is rejected
  (`LogicalTimeRegression`). Before applying any command the engine
  applies everything due by its time: scheduled actions, tender and OTC
  expiries, GTD expiries and, from stage 2, session-phase changes.
- The clock's mode decides how run time follows the venue's real time:
  - `Paced { step_interval_ns }`: run time advances `step_ns` for every
    `step_interval_ns` of venue time (equal values are real time; a
    smaller interval compresses a day for a classroom). The venue stamps
    each input at release as `anchor_run + (t - anchor_venue) × step_ns /
    step_interval_ns`, re-anchoring whenever the clock was moved another
    way (an operator `Advance`) and freezing while the run is not active.
  - `Lockstep` and `Accelerated`: run time moves only by operator
    `Advance`; inputs are stamped with the current run time.
- Run time is nanoseconds since the run began, not a wall-clock date. A
  calendar may name the date and time zone that run time zero represents,
  for display only.
- **The venue timer.** Under `Paced`, a venue timer submits a `ClockTick`
  input through the admission sequencer, the only committer, at the next
  instant something is due. The engine reports that instant
  (`RunState::next_due`). A tick is an ordinary journaled input, so replay
  needs no timer.
- Built-in agents keep deciding on their own schedule, but their commands
  are stamped by the same rule as every other input.
- The mapping also re-anchors when the pace changes (`SetPacing`). The
  anchor lives in the venue process, so after a restart run time resumes
  from the journal's clock: a venue outage is not traded through.
- Tender and OTC decisions check expiry themselves, so an order's
  catch-up skips the scan that marks them expired; the venue timer's tick
  and an operator `Advance` perform it. A command whose catch-up applies a
  scheduled action is staged on a copy, because the action's own rules can
  refuse it; every other catch-up runs in place.

### 2. Calendar and session phases (stage 2)

- A scenario `calendar` gives each venue a session per trading day, as
  offsets from the day's start: `pre_open`, `open`, `closing_call`,
  `close`. It also gives the day length, the number of days, and the day
  indices that are holidays. Every listing of a venue follows its venue's
  session, as on real exchanges.
- Phases: `closed` → `pre_open` (orders and cancels accepted, no matching)
  → opening auction at `open` → `continuous` → `closing_call` (orders
  collect, no matching) → closing auction at `close` → `closed` (new
  orders rejected; cancels accepted). Phase changes are engine events
  applied when the run clock passes each boundary. A scenario without a
  calendar trades continuously, as today.
- **DAY orders** expire at their venue's close, after the closing auction.
  GTC orders carry overnight; GTD orders expire at their time.
- Instrument halts become listing halts: `SetListingHalt` names a listing,
  and a halted listing collects orders like a call phase. It reopens
  through an auction, as real venues do after a halt.

### 3. Call auctions (stage 3)

- At an auction the venue uncrosses its book at one price:
  1. the price that maximises executable volume;
  2. then the one leaving the smallest imbalance;
  3. then the one nearest the reference price (the last trade, else the
     previous close, else the opening mark);
  4. then the lower price.
- Market orders take part at any price and have priority. Every match
  prints at the auction price, in price-time priority on both sides.
- During call phases, every public direct feed publishes the indicative
  auction price, matched volume and imbalance side and size, each time
  they change. Like every other feed message, these leave the venue after
  commit and travel `L(v, p)`.
- Iceberg and hidden quantity take part in the uncross but stay hidden in
  the indicative data.

### 4. End of day and multi-day (stage 4)

- At each close the venue records the official closing price for each of
  its listings: the closing auction price, else the day's last trade,
  else the previous mark. A `DayClosed` event carries those marks, and
  NLV and scores use them.
- Positions, cash and GTC orders carry to the next day. Under `Paced`, the
  timer jumps the run clock from one day's close to the next day's
  pre-open (the night is not traded), so a five-day run takes five
  trading sessions of real time.

## Consequences

- One deterministic clock drives expiries, scheduled actions, sessions
  and auctions. A run replays from its journal exactly, ticks included.
- The QUARCC profile's scenario must say `Paced` with equal step and
  interval to keep real-time trading. The scenario format changes, and no
  migration is provided.
- Order commands can now emit events that are due by their time
  (scheduled actions, expiries), so a team's order may be the input that
  applies them. That is deterministic and journaled.
- Auctions and phases add order-entry rejections during `closed` and
  change fill prices at the open and close. RULES.md and PROTOCOL.md must
  describe both before participants see them.

## Rejected alternatives

- **Keep venue epoch time for orders** and convert it for sessions: this
  leaves two clocks, and the run clock and the order clock would drift
  under classroom compression.
- **A wall-clock timer that mutates state directly:** it bypasses the
  sequencer and the journal, so replay could not reproduce it.
- **Continuous trading through the open and close:** this is simpler, but
  it is not how the venues the owner names (TSX, NYSE, Cboe) open or close,
  and it gives no official closing price.

## Validation

Each stage ships with engine unit tests (clock regression, due-item
ordering, phase transitions, auction price rules including ties, DAY
expiry, closing marks) and end-to-end server tests (a paced run applies a
scheduled action with no other input; an opening auction fills at one
price for teams at different distances; a restart mid-session and across
a day boundary replays to the same state hash).

## Operational impact

Organizers set the clock mode and calendar in the scenario. A classroom
can compress a trading day with `Paced`. The competition profile ships
with a real-time `Paced` clock. The admin health view reports the run
time and, from stage 2, each venue's phase.

## Security impact

Timer ticks come only from the venue itself, through the sequencer, and
carry no participant identity. Auction indicative data is aggregated and
anonymous, like L2.

## References

- [Exploration note §8](../research/2026-10-09-exploration-and-next-steps.md)
- [Evidence-led roadmap, Slice 4](../plans/2026-10-07-evidence-led-core-roadmap.md)
- [ADR 0035](0035-latency.md), [ADR 0036](0036-public-market-data-feeds.md)
