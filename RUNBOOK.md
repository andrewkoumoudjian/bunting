# Organizer runbook

1. Install a release binary, run `bunting init`, replace every example secret,
   add the event roster, and run `bunting doctor <config>`.
2. Publish `RULES.md`, `SCORING.md`, the matching interval, and all configured
   limits unchanged before credentials are distributed.
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

## Known limitations (verified 2026-10-09 at `1d857d1`)

Plan events around these until the fixes listed in
`docs/research/2026-10-09-exploration-and-next-steps.md` §8 land:

- **Passive fills are not reported to the resting participant.** Execution
  reports go only to the connection that sent the aggressing order. Teams
  must poll account/discovery to learn about fills on their resting orders.
- **`max_open_orders` counts every accepted order on a connection** and is
  released only by an explicit cancel, not by fills or expiry. Active teams
  can be falsely rejected; set the limit generously.
- **The archive does not contain ordinary orders.** `bunting replay`/`judge`
  verify simulation/control commands only; they cannot recompute fills or
  P&L from orders. Keep the origin journal and event files for disputes.
- **Default storage holds 10,000 commands across all runs.** After that every
  command fails. Raise `storage.max_commands` and `max_events_per_run` for
  long or busy rounds.
- **Built-in agent state restarts from scratch** when the server restarts, so a
  restarted round is not identical to an uninterrupted one.
