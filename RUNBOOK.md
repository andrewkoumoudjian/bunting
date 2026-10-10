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
3. Start `bunting server <config>` (or the container image), verify `/health`, export
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

## Bunting Native Protocol (slice 25)

Teams may trade over BNP ([ADR 0040](docs/adr/0040-bunting-native-protocol-v1.md),
[wire contract](docs/specs/bnp-v1.md)) with the `bunting-trader` client or
their own `bunting-client` program. BNP goes through the same sequencer and
latency map as FIX. It needs a native server build; the WASIX package has no
BNP listener.

1. Create an event CA and issue a server certificate (with the host name
   teams connect to) and one client certificate per team. With OpenSSL:

   ```bash
   cat > ca.cnf <<'CNF'
   [req]
   distinguished_name = dn
   prompt = no
   [dn]
   CN = Bunting event CA
   [ca]
   basicConstraints = critical, CA:true
   keyUsage = critical, keyCertSign, cRLSign
   [server]
   basicConstraints = CA:false
   keyUsage = critical, digitalSignature
   extendedKeyUsage = serverAuth
   subjectAltName = DNS:venue.example.org
   [client]
   basicConstraints = CA:false
   keyUsage = critical, digitalSignature
   extendedKeyUsage = clientAuth
   CNF
   openssl ecparam -name prime256v1 -genkey -noout -out ca.key
   openssl req -x509 -new -key ca.key -days 30 -config ca.cnf -extensions ca -out ca.pem
   openssl ecparam -name prime256v1 -genkey -noout -out server.key
   openssl req -new -key server.key -subj /CN=venue -out server.csr
   openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
     -days 30 -extfile ca.cnf -extensions server -out server.pem
   # per team:
   openssl ecparam -name prime256v1 -genkey -noout -out team1.key
   openssl req -new -key team1.key -subj /CN=team1 -out team1.csr
   openssl x509 -req -in team1.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
     -days 30 -extfile ca.cnf -extensions client -out team1.pem
   ```

   Keep `ca.key` offline. Teams may instead send a CSR and keep their key.
2. Register each team certificate's fingerprint
   (`bunting-trader fingerprint team1.pem`) in the server config. The
   certificate's subject grants nothing; only the roster does. Paths are
   relative to the config file:

   ```json
   "bnp": {
     "bind": "0.0.0.0:9881",
     "run_id": 1,
     "certificate_chain": "server.pem",
     "private_key": "server.key",
     "client_ca": "ca.pem",
     "revocation_lists": [],
     "roster": [{ "certificate_sha256": "<64 hex digits>", "participant_id": 1 }],
     "heartbeat_ms": 1000,
     "max_connections": 1,
     "max_frame_bytes": 65536,
     "rate_limit_window_ms": 1000,
     "max_messages_per_interval": 100,
     "handshake_timeout_ms": 5000
   }
   ```

   `run_id` must match `fix.run_id` and the runtime run. v1 accepts the
   `participant` role only; instructors and administrators use FIX.
3. Give each team `ca.pem`, its certificate and key, and the host and port.
   They check the connection with
   `bunting-trader --server venue.example.org:9881 --ca ca.pem --cert team1.pem --key team1.key account`
   (or set `BUNTING_SERVER`, `BUNTING_CA`, `BUNTING_CERT`, `BUNTING_KEY`).
4. To revoke a certificate, issue a new CRL from the CA and replace the file
   named in `revocation_lists` atomically (write a new file, then rename it
   over the old one). The venue checks the file every second: the team's live
   session is logged out and its certificate can no longer connect, with no
   restart ([ADR 0041](docs/adr/0041-bnp-live-certificate-revocation.md)).
   A file that does not parse is ignored and the previous list stays in
   force. A CRL file must be listed at start-up to be watched; roster edits
   still need a restart. Any tool that writes a PEM CRL signed by the CA
   works; the OpenSSL commands in step 1 keep no CA index, so CAs that need
   revocation should be run with `openssl ca` (which does), or another CA
   tool. List an initial (empty) CRL at start-up so there is a file to
   replace.

A BNP client that reconnects with its cursor (`--resume-after`) receives the
reports it missed while the venue still retains them (the latest 16,384
committed batches); otherwise it is told there is a gap and rebuilds from
open orders and account.

## Known limitations (verified 2026-10-09; updated after slice 12)

Plan events around these until the fixes listed in
`docs/research/2026-10-09-exploration-and-next-steps.md` §8 land:

- **Reports missed while disconnected are not replayed over FIX.** (BNP
  sessions resume from their cursor; see above.) Fills are delivered
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
