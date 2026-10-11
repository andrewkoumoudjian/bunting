# ADR 0042: Instructor and administrator control and the run stream over BNP

- Status: **Accepted** (2026-10-11), implemented in slice 39. Implements the
  instructor/admin and run-stream parts of
  [ADR 0031](0031-proposed-bunting-native-client-protocol.md) on
  [ADR 0040](0040-bunting-native-protocol-v1.md)'s protocol.
- Wire contract: [`docs/specs/bnp-v1.md`](../specs/bnp-v1.md).

## Context

BNP v1 accepted participant certificates only: config validation refused
any other roster role (observed, `config.rs` before this slice). Instructors
and administrators ran a round over FIX, sending `UA`/`UB` messages whose
action and JSON payload `session_host.rs` maps to engine
`SimulationCommand`s (`start`, `pause`, `resume`, `advance`, `terminate`,
`publish_news`, `open_tender`, `score`, `fine`). Authority is the
application layer's `authorize_simulation_command`: Instructor or
Administrator for everything except the participant's own tender and OTC
decisions. No interface delivered run lifecycle, halts or news as they
happened: FIX clients poll (`x`, `BE`), and BNP had nothing. The owner wants
an app on a hosted server over Bunting's own protocol (2026-10-09) and
classroom use beside the competition.

## Decision

1. **Operator certificates.** A `bnp.roster` entry may have role
   `instructor` or `administrator`. Its `participant_id` field is the
   operator's actor ID (as on the FIX roster); the session's actor carries no
   participant, so it cannot trade and receives no participant's reports.
   IDs stay unique across the roster.
2. **Typed operator commands.** `OperatorCommand { request_id, action }`
   carries one of: start, pause, resume, advance (steps), terminate
   (reason), halt or resume a listing's instrument (reason), mass cancel
   (optional participant, optional instrument), publish news (ID,
   audience, headline, body), score the iteration, fine a participant.
   Fields are typed binary, not JSON. Each is an engine `SimulationCommand`
   authorized by `authorize_simulation_command` and applied at the hub
   through the admission sequencer from the operator's location, so it is
   journaled with its admission record like every other input. The answer
   is `OperatorDone { request_id, run_sequence }` after commit, or `Reject`
   naming why (refused, or not committed). A participant certificate's
   `OperatorCommand` is refused by the same check.
3. **Run stream.** Every session receives the run's public facts as they
   commit, stamped with their event sequence in the private stream (so they
   are ordered with reports and replayed on resume): `RunStatus` (lifecycle
   changes), `ListingHalt` (an instrument halted or resumed) and `News`
   (headline and body) for news whose audience includes the session's actor,
   under the same rule as FIX's news query. They leave the hub after commit
   and cross `L(hub, team)` like any committed message.
4. Operator sessions send no orders: `NewOrder`, `CancelOrder` and the kill
   switch are refused before admission with a reason.

## Consequences

- A round can be run entirely from BNP clients; FIX keeps its `UA`/`UB`
  operator messages.
- Teams learn of halts, lifecycle changes and news without polling, at the
  same virtual distance as their reports.
- `open_tender`, OTC, cashflow and facility commands remain FIX-only (their
  payloads need their own typed layouts) and `SetPacing` has no interface.

## Rejected alternatives

- **JSON action plus payload, as FIX `UA`:** untyped and harder for
  non-Rust clients; BNP's layouts are fixed binary.
- **A separate operator protocol or HTTP endpoint:** ADR 0031 allows FIX and
  BNP only; the admin HTTP stays read-only and loopback.
- **Run facts as unstamped session messages:** they would race reports and
  be lost on reconnect; stamping them puts them in the resumable stream.
- **Operators receive every participant's reports:** not needed for control
  and it would leak private data to a certificate that only runs the round.

## Validation

`apps/bunting-server/tests/bnp.rs`: an instructor certificate pauses and
resumes the run, halts the instrument, publishes public news and fines a
team; a team receives `RunStatus`, `ListingHalt` and `News` in sequence
order; a team's `OperatorCommand` and an instructor's `NewOrder` are refused.
Wire round trips cover every action.

## Operational impact

Operators register their certificates' fingerprints with role
`instructor` or `administrator` and a unique ID. `bunting-trader` gains
operator subcommands.

## Security impact

Authority still comes only from the registered certificate fingerprint and
its roster role; the message never names an actor. Operator commands pass
the same application authorization as FIX's. News visibility follows the
existing audience rule.

## References

ADR 0031, ADR 0035, ADR 0040, `packages/bunting-application/src/lib.rs`
(`authorize_simulation_command`), `competition.rs` (`news_visible`).
