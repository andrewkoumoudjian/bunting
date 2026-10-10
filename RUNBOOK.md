# Organizer runbook

1. Install a release binary, run `bunting init`, replace every example secret,
   add the event roster, and run `bunting doctor <config>`.
2. Publish `RULES.md`, `SCORING.md`, the latency map (`fix.admission.map`:
   the location of each team, venue and the hub, and the latency between
   every pair of locations in use, in microseconds, with optional seeded
   jitter; it fixes team-to-venue and team-to-team distance) and all configured limits
   unchanged before credentials are distributed. Latency is real (ADR 0035):
   teams' own connectivity counts and is not equalized, so host the server
   close to the participants (or run everyone on one network) when that
   matters, and say so in the rules. `GET /admin/admission` (bearer token)
   lists each connection's measured access latency for publication. Never
   change the map during a round.
3. Start `bunting-server <config>` through Wasmer, verify `/health`, export
   credentials through a protected channel, and let every team complete Logon
   plus discovery.
4. Arm and start the round through an administrator FIX session. Pause/resume,
   news, tenders, fines, scoring, and termination are committed simulation
   commands and therefore appear in replay.
5. On a participant disconnect, leave resting orders live and let that identity
   reconnect. On venue integrity risk, issue the halt/terminate control once,
   preserve origin and session files, and do not restart against a different
   scenario.
6. Produce the archive, run `bunting replay`, then `bunting judge`. Publish only
   the verified archive, leaderboard, and immutable public snapshots.
7. Resolve disputes from the archive's ordered accepted commands, canonical
   events, policy values, and checksums. Live UI state is never settlement
   evidence.

## Configuration change (slice 12)

`fix.max_open_orders` was removed from the server configuration; a config that
still sets it fails at startup with an explanation. Set
`participants.<id>.limits.max_live_orders` in the scenario instead (the
checked-in local scenario uses 256 for each human participant). Omitting it
leaves that participant without a per-participant cap.

## Known limitations (verified 2026-10-09; updated after slice 12)

Plan events around these until the fixes listed in
`docs/research/2026-10-09-exploration-and-next-steps.md` §8 land:

- **Reports missed while disconnected are not replayed.** Fills are delivered
  to every connected participant, but a team that is offline when its order
  fills (or that falls more than 4,096 committed batches behind and is
  disconnected) must request account/discovery after reconnecting to
  resynchronize.
- **The archive does not contain ordinary orders.** `bunting replay`/`judge`
  verify simulation/control commands only; they cannot recompute fills or
  P&L from orders. Keep the origin journal and event files for disputes.
- **Storage bounds are per run.** `storage.max_commands_per_run` and
  `max_events_per_run` cap one run (the shipped profiles allow 1–2 million
  commands); each committed command keeps one small index entry in memory.
  A stale `storage.max_commands` key stops the server with an explanation.
- **The journal is never compacted.** `<path>.wal` holds every committed
  command of every run (it is the run's history); plan disk for its growth.
  `<path>` is a state-only checkpoint that only speeds up restarts; deleting
  it forces a full re-execution from genesis, never data loss. Stores written
  before journal format 3 (slice 15b; format 2 shipped in slice 14) are
  refused at startup; archive them and start a new run.
- **Built-in agent state restarts from scratch** when the server restarts, so a
  restarted round is not identical to an uninterrupted one.
