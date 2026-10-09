# Bunting server instructions

This app is the single venue host (ADR 0022): sockets, sessions, TLS,
filesystem persistence, admission and delivery. Market authority, matching,
ledger, canonical events, identity authorization and commit preparation stay
in packages. Bound every connection, request, queue, journal and recovery file.

- Keep the server buildable and testable natively; add no WASIX-only
  dependencies or code paths (ADR 0033). It currently ships as a WASIX module
  (ADR 0027).
- Participant interfaces are FIX and the Bunting Native Protocol only
  (ADR 0031, target). Do not add other command surfaces. TLS is moving
  in-process with mutual authentication; certificates map to actor identity.
- Do not keep authority in session state. The connection-local `open_orders`
  set and requester-only report delivery in `session_host.rs` are known
  defects: limits belong to engine risk admission and reports to a committed
  event distributor that reaches every affected participant.
- `writer.rs` implements ADR 0024 intervals (current). The target is the
  ADR 0030 latency-modeled sequencer; do not build new behavior on the
  sleep-to-boundary writer, and route built-in agents through the same
  admission path as FIX.
- Do not add per-command `RunState` clones or full-state serialization; the
  target is a writer-owned live state with a command journal (exploration
  note Step 3). Every acknowledged input must be recorded and replayable.
