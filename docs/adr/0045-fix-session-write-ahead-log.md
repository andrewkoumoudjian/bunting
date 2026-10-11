# ADR 0045: FIX session state is a write-ahead log, not a per-message snapshot

- Status: Accepted (2026-10-11). The owner decided on 2026-10-10 that the
  venue must never add delay a real network would not; rewriting a session
  snapshot on every message added 3–4 ms to each acknowledgement. Implemented
  in slice 42.
- Date: 2026-10-11
- Supersedes: the in-memory delivery cursor of slice 34
  (`DeliveryCursor`, `ParticipantClaim::{take_cursor, store_cursor}`) and the
  whole-snapshot session file `<origin>.fix-session.fix-session-<p>.json`.
- Depends on: ADR 0022 (single native venue), ADR 0028 (single ledger, full
  replay), ADR 0035 (latency), ADR 0044 (native host).

## Context

1. **Observed: the session snapshot dominated the acknowledgement.** Before
   this change `session_host.rs` serialized each FIX session's whole
   `NativeFixSnapshot` (session sequences, the resend journal and the FIX
   application's client-order identity map) to a temporary file, fsynced
   and renamed it on every message. ADR 0044 measured 3–4 ms per order round
   trip with the file origin against 0.3 ms with the memory origin and
   attributed the difference to this rewrite (*inferred* there). On the base
   of this change (`8fbb537`, release, 4 cores, loopback, 2,000 orders one at
   a time, `tools/host_load.py`) the file origin measured p50 7.3 / 7.0 ms,
   p99 27.6 / 28.1 ms: the snapshot grows with the journal, so the cost grows
   through a run.
2. **Observed: the snapshot was written after the frames it described.** A
   kill between the socket write and the rename lost the outgoing sequence
   number of frames the participant had already received; the next logon
   failed with a sequence error (`tests/fix_session_crash.rs` reproduced it
   on the base).
3. **Observed: commands in flight at a crash were lost or left unanswered.**
   A command admitted but not yet journaled was lost with the admission
   queue; a command journaled but not yet reported was never reported,
   because reports reach sessions only from the live distributor.
4. **Observed: slice 34's delivery cursor lived in memory.** Reports still
   on their virtual path when the venue stopped, or committed while the
   participant had no session, were lost over a venue restart.
5. **Observed: a reconnect could deadlock.** When both sides had a gap, each
   queued the other's `ResendRequest` behind its own gap and neither
   answered (`simfix-session`).

## Decision

1. Each participant's FIX session state is an append-only log
   `<origin stem>.fix-session-<p>.log` next to the origin, framed like the
   origin journal: magic `BUNTFIX1`, length, SHA-256, JSON payload. It holds
   one checkpoint (session scalars and resend journal, the FIX
   application's state, the delivery position, unwritten reports and
   commands recorded but not yet journaled) followed by deltas.
2. A delta holds what changed since the previous record: session scalars,
   resend-journal entries added and removed, the application inputs that
   changed state (an inbound message with its actor, run and correlation;
   a committed batch that produced reports or registered an order), the
   delivery position `seen_through` and the keys of reports still
   unwritten. The application state is rebuilt by replaying those inputs
   through the same deterministic `map_message` and `committed_messages`.
3. **Write ahead.** The log is appended and `sync_data`'d once, immediately
   before frames are written to the socket, and covers those frames: their
   sequence numbers, their resend entries, and their reports marked written.
   Changes that write nothing ride along with the next write, and a final
   record is written when the session closes.
4. Compaction rewrites the log as one checkpoint (temporary file, sync,
   rename, directory sync) at logon and when deltas exceed
   max(8 MB, checkpoint size). A torn final record is dropped; any other
   damage refuses the session. The previous whole-snapshot file is refused,
   not migrated.
5. **Recovery.** At logon the venue replays the log, then:
   - readmits recorded commands the origin journal does not hold, and the
     sequencer skips any command whose identifier the journal already holds,
     so a command is committed exactly once whichever side of the crash it
     was on;
   - sends unwritten reports at once (their path was already crossed);
   - delivers every batch committed after `seen_through`, from the
     distributor's retention window when it reaches back, otherwise from a
     lock-free scan of the origin journal; this starts only once the session
     is established with no resend or inbound gap pending, so reports follow
     the participant's own recovery.
6. `SessionAction::Persist` carries no snapshot; hosts read
   `FixSession::state`. `simfix-session` answers a `ResendRequest` at once
   even while it waits on its own gap.

## Consequences

- Release build, 4 cores, loopback, file origin, 2,000 orders one at a
  time, interleaved runs on the same machine: base `8fbb537` p50 7.32 /
  6.98 ms, p99 27.6 / 28.1 ms, max 58 / 119 ms; this change p50 1.29 /
  1.31 ms, p99 2.50 / 3.12 ms, max 6.8 / 8.0 ms. The memory origin
  measured p50 0.28 ms, p99 0.60 ms. The remaining file-origin cost is two
  data syncs per order: the origin journal (fsync before acknowledgement)
  and the session log (write ahead). Both are what any durable venue pays.
- The session log is about 3.4 MB per participant after 2,000 orders, until
  compaction.
- Sequence numbers, resend, client-order identities and unsent reports now
  survive `kill -9`, and orders in flight at the kill are answered exactly
  once.
- Debug builds check after every session that replaying the log reproduces
  the live session, application and delivery state.

## Rejected alternatives

- **Keep the snapshot and write it less often.** Any acknowledgement sent
  before its state is durable can be lost by a crash; a write-behind
  snapshot reintroduces the sequence error of Context 2.
- **Persist FIX state inside the origin journal.** It would make the
  session's private transport state part of the market record and add a
  journal record per outbound message; the origin journal records only
  market inputs.
- **A database (SQLite or similar).** A new dependency for an append-only
  log the server already knows how to frame, hash and recover.
- **Persist the application map as a snapshot in each delta.** Still
  O(map) per message; replaying the inputs keeps each delta proportional to
  the change.

## Validation

Recorded in the implementation log (slice 42):

- `cargo test -p bunting-server --test fix_session_crash`: kill -9 at three
  points with sequence numbers, resend and order identities surviving; 40
  orders with kills before, during and after them, each answered with
  `0`/`8` and journaled exactly once; a fill committed while the
  participant was away (without and with a kill) delivered exactly once.
- `cargo test -p simfix-session`
  (`resend_request_ahead_of_a_gap_is_answered_at_once`) and the delivery
  and missed-report tests.
- Latency with `tools/host_load.py` as above.
- The seven required gates.

## Operational impact

Session logs sit next to the origin file and must be kept with it; deleting
one resets that participant's FIX sequence numbers and identities.
Existing `*.fix-session.fix-session-<p>.json` files must be removed before
upgrading (the server refuses to start a session that has one).

## Security impact

None to authentication or authorization. Session logs hold the
participant's FIX messages and reports; they need the same file
permissions as the origin. Records are length-bounded and hash-checked,
and a damaged log refuses the session rather than guessing.

## References

- [ADR 0044](0044-native-server-binary-host.md) (measurement that
  motivated this), [ADR 0035](0035-latency.md),
  [ADR 0028](0028-proposed-headless-run-authority.md)
- `apps/bunting-server/src/session_store.rs`, `delivery.rs`,
  `session_host.rs`; `tests/fix_session_crash.rs`; `tools/host_load.py`
