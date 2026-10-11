# ADR 0043: Tenders and OTC negotiations over BNP

- Status: **Accepted** (2026-10-11), implemented in slice 46. Extends
  [ADR 0040](0040-bunting-native-protocol-v1.md) and
  [ADR 0042](0042-bnp-operators-and-run-stream.md); implements the deal part
  of [ADR 0031](0031-proposed-bunting-native-client-protocol.md).
- Wire contract: [`docs/specs/bnp-v1.md`](../specs/bnp-v1.md).

## Context

The engine has tenders (an instructor offers one participant a block at a
price until a deadline; the participant accepts or declines) and OTC
negotiations (a participant proposes a bilateral trade, the counterparty
counters, either side accepts, rejects or breaks it) as
`SimulationCommand`s (observed, `packages/bunting-engine/src/simulation.rs`).
Before this ADR:

- FIX carried `open_tender` (operator `UA`) and tender accept, decline and
  list; it carried no OTC command at all (observed, `session_host.rs`).
- BNP carried neither, although BNP is meant to cover what FIX does for
  teams (ADR 0031) and the desktop app is being redesigned with a RIT-style
  tenders pane.
- `authorize_simulation_command` required an instructor to open an OTC
  negotiation while the engine requires the proposer to be a participant,
  so no one could open one (observed: the two checks contradict).
- Expiry changed a tender's or negotiation's status inside the clock sweep
  without an event, so no stream could report it (observed).

## Decision

1. **Typed deal commands.** `TenderDecision { request_id, tender_id,
   accept }`, `OtcPropose { request_id, negotiation_id, counterparty_id,
   instrument_id, side, quantity, price, expires_at_ns }`, `OtcCounter
   { request_id, negotiation_id, quantity, price }` and `OtcDecision
   { request_id, negotiation_id, decision }` (accept, reject, break) map to
   the engine's commands, are authorized by `authorize_simulation_command`
   and are applied at the hub through the admission sequencer from the
   team's location, like operator commands. The answer is `DealDone
   { request_id, run_sequence }` after commit or `Reject`. Operators may not
   send them (they do not trade).
2. **Operators open tenders over BNP** with operator action 11
   (`open_tender`).
3. **Participants open OTC negotiations.** The authorization check now
   treats `OpenOtc` as a participant action, matching the engine. The
   client names its own `negotiation_id` (u64); the venue's ID is that value
   in the participant's `negotiation` namespace, so two teams cannot collide.
4. **Command request IDs are u64** and scoped per participant (namespace
   `deal`), so a reused ID is an idempotent duplicate. `OperatorCommand` and
   `OperatorDone` request IDs widen from u32 to u64 for the same reason (no
   deployed clients depend on the old width; old frames are refused, as for
   every breaking change).
5. **Deal state in the private stream.** Every committed change to a tender
   or negotiation is sent as `Tender` or `Negotiation` with its full state
   (status, quantity, price, whose turn it is) to the parties and to
   operators, stamped with its event sequence, after `L(hub, team)`. The
   state is read right after the commit, so a counter or an expiry reports
   the values of that commit. `DealsRequest` returns every visible tender
   and negotiation for rebuilding a view after reconnecting.
6. **Expiry is an event.** The engine's sweep now emits `TenderChanged` and
   `OtcChanged` with status `expired`, so expiry reaches every interface and
   the journal like any other change.

## Consequences

- Teams can trade tenders and negotiate OTC entirely over BNP; the desktop
  app's tenders pane has typed data to render.
- FIX still has tender decisions and no OTC; adding OTC to FIX would need a
  custom message layout and no one has asked for it.
- `bunting-trader` gains `tender`, `otc` and `deals` subcommands and the
  `operator open-tender` action.
- Cashflow and facility commands remain without a BNP layout.

## Rejected alternatives

- **Generic JSON payload command:** untyped, unlike the rest of BNP.
- **Venue-assigned negotiation IDs returned in the answer:** the client
  could not correlate the first `Negotiation` update before the answer
  arrives; client-chosen IDs in a namespace match how orders work.
- **Deal updates through a public feed:** tenders and negotiations are
  private to their parties.
- **Polling only (`DealsRequest`):** counters and expiry would be seen late
  and lost on reconnect; stamped updates are resumable.

## Validation

`apps/bunting-server/tests/bnp.rs`: an instructor opens two tenders over
BNP; the team accepts one (position and cash change accordingly), the other
expires with an `Expired` update, deciding it is rejected, and `Deals`
lists both. Two teams negotiate: one proposes, the other sees it and
counters, the first accepts, and both accounts settle. Wire round trips
cover every new message and enumeration.

## Operational impact

None beyond new client capability; tenders still come from the instructor
(BNP or FIX) or the scenario.

## Security impact

Authority still comes only from the certificate's roster role; a team can
decide only its own tenders and negotiations (engine checks), and sees only
deals it is party to. Operators see all deals but cannot decide them.

## References

ADR 0031, ADR 0035, ADR 0040, ADR 0042,
`packages/bunting-application/src/bnp.rs` (`deal_command`, `deal_messages`),
`packages/bunting-application/src/competition.rs` (`deal_changes`),
`apps/bunting-server/src/bnp_host.rs`.
