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
- Do not keep authority in session state. Per-participant limits are engine
  risk admission; reports reach every affected participant through
  `distributor.rs`, which publishes after each durable commit. Every mutating
  path must commit through `PublishingOrigin` under the authoritative writer so
  subscribers see batches in commit order; never publish before commit or
  send reports inline from a command path.
- `wake.rs` + `admission.rs` + `session_host.rs` implement ADR 0035: a
  reader thread per connection stamps `t_rx` the moment bytes arrive; the
  session admits without blocking; the sequencer releases at
  `t_rx + L(p, d)` (per-destination FIFO per connection) and journals the
  `AdmissionRecord`; committed batches carry where they were applied
  (`Committed.source`, the admitted destination) and leave `L(s, p)` after
  commit (never heartbeats or probes). Never derive a batch's source from
  the acting team: its path to the destination was already crossed. One live session per participant;
  connections wait briefly for a slot when a reconnect races the old
  session's close. `tcp_rtt.rs` reads kernel RTT through netlink
  `sock_diag` without `unsafe`; it is published, never used for ordering.
  `writer.rs` is only the commit gate shared with the agent runtime; route
  built-in agents through the sequencer next (Step 4).
- The origin owns the live runs; `storage.rs` appends one journal-format-3
  record per committed input (`commit_journal.rs`) before acknowledging and
  writes state-only checkpoints every `storage.checkpoint_interval` commands.
  Restart re-executes the journal after the checkpoint and must reproduce it
  exactly. Do not add per-command `RunState` clones or full-state
  serialization, and do not write sockets inside `read_run` closures. Every
  acknowledged input must be recorded and replayable.
