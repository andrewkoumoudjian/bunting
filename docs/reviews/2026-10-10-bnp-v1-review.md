# BNP v1 independent review (main at 123e588, slice 25)

- Status: **Open findings.** Line numbers refer to `123e588`. Re-checked on
  `cfe35b2` (slice 29): F1, F2 and F3 still reproduce with the ignored tests in
  `apps/bunting-server/tests/bnp_review_regressions.rs`
  (`cargo test -p bunting-server --test bnp_review_regressions -- --ignored`).
  Slice 27 (ADR 0041) has since added live CRL reload, so F5's "read at
  start-up only" no longer holds.

Reviewed 2026-10-10. The repro tests for F1–F3 are committed as ignored regression tests
(`apps/bunting-server/tests/bnp_review_regressions.rs`); un-ignore each with its fix.

## Verdict

The build is clean and the identity and TLS handling hold up: every certificate attack I
tried was refused. But three claims in the report don't hold:

1. The kill switch is a **run-wide** switch. Any one team can stop new orders for every
   team, and it cancels none of that team's own orders.
2. "A team that reconnects gets back exactly the reports it missed" is false when reports
   come from places at different distances. A fill can be lost while the venue reports a
   complete replay.
3. Anyone who can reach the BNP port can lock every registered team out without a
   certificate, by holding a dozen idle TCP connections.

## Checks I ran

| Check | Command | Result |
|---|---|---|
| 1 metadata | `cargo metadata --locked --format-version 1 --no-deps` | pass |
| 2 fmt | `cargo fmt --all --check` | pass |
| 3 clippy | `cargo clippy --locked --workspace --all-targets -- -D warnings` | pass |
| 4 tests | `cargo test --locked --workspace` | pass: 236 passed, 0 failed, 1 ignored, 73 test binaries |
| 5 orderbook-rs | `! cargo tree --locked -p bunting-engine -e normal \| grep -q orderbook-rs` | pass |
| 6 wasm32 | `cargo check --locked --workspace --target wasm32-unknown-unknown` | pass |
| 7 diff | `git diff --check` | pass |
| BNP e2e x5 | `cargo test --locked -p bunting-server --test bnp` | 5/5 runs: 4 passed, 0 failed (5.03–5.04 s each) |

The report's numbers (236 tests, 7 gates, 5 e2e runs) are confirmed.

## Hand run

I made certificates with the runbook's OpenSSL commands (SAN `localhost`). I also made
extra certificates to attack the venue with: one expired (`openssl ca -startdate/-enddate`),
one revoked plus a CRL, one with a serverAuth-only EKU, one CA-signed but not on the roster,
and an "impostor" signed by the CA with subject `CN=team1` and its own key. Then I started a
native debug `bunting-server`, using `local.json` plus a `bnp` block (with the agent
liquidity provider on), and ran `bunting-trader`:

| Action | Result |
|---|---|
| `fingerprint team1.pem` | matches `openssl x509 -outform der \| sha256sum` |
| `account` as team1 | connected as participant 1, Live |
| unregistered cert | `certificate is not registered for this run` |
| impostor (`CN=team1`, other key) | `certificate is not registered for this run` (the subject grants nothing) |
| expired cert (registered) | TLS alert `CertificateExpired` |
| revoked cert (registered, CRL loaded) | TLS alert `CertificateRevoked` |
| serverAuth-only cert (registered) | TLS alert `UnsupportedCertificate` (EKU enforced) |
| `buy 1 1 2 --price 90 --id 101` | accepted, resting, round trip 1.8 ms (debug build) |
| same `--id 101` again | `Reject`: `IdempotencyConflict`; nothing new committed |
| `cancel 101` / `cancel 999` | canceled / `cancel rejected 999: UnknownOrder` |
| `buy 1 1 3 --market` | filled 3 @ 102 against the agent (Taker), done; account shows the position |
| kill switch | no CLI command exists; tested through the client library (see F1) |

## Findings, most severe first

### F1. Critical: one team's kill switch stops new orders for every team, and cancels nothing

- `apps/bunting-server/src/bnp_host.rs:869-884` maps `KillSwitch` to `CommandPayload::ActivateKillSwitch`.
- `packages/bunting-engine/src/lib.rs:1230-1233` sets the **run-level** `candidate.kill_switch = true`. At `lib.rs:1177`, every later order from any participant is rejected with `KillSwitchActive`. No orders are canceled.
- `packages/bunting-application/src/lib.rs:140` (`authorize_command`) lets any participant send this command.
- FIX never exposes this command: `simfix-mapping` maps only D/F/G/H. So BNP is the first participant interface that reaches it. The report's line that BNP does "everything FIX does ... the kill switch" is wrong on both counts.
- **Confirmed** with a review-only test using the standard bnp.rs harness. Team 1 rests an order, then sends `KillSwitch`. Its `OpenOrders` still lists the order. Then team 2 sends a new order and gets `OrderRejected { reason: KillSwitchActive }`.
- The fix needs Bunting semantics, so it may need an engine change plus an ADR note: a per-participant kill switch that cancels that participant's live orders (`CancelReason::KillSwitch` already exists) and blocks only that participant. Until then, BNP should refuse `KillSwitch`.

```rust
a.send(&limit(1, Side::Buy, 1, 91))?;            // team 1 rests
a.send(&ClientMessage::KillSwitch { request_id: 1 })?;
// team 1 OpenOrders -> still [client_order_id 1, remaining 1]
b.send(&limit(5, Side::Sell, 1, 120))?;          // team 2
// -> OrderRejected { client_order_id: 5, reason: KillSwitchActive }
```

### F2. High: resume can silently lose a report (out-of-order delivery plus a max cursor)

- Reports leave at `durable_us + L(source, team)` (`bnp_host.rs:496-501`). The outbound hold sends them in send-time order (`outbound.rs`), not sequence order. So a report committed earlier at a far venue can arrive after a later report from a nearer venue or the hub.
- The client cursor is `fetch_max` of the stamps it has seen (`packages/bunting-client/src/connection.rs:358-359`). The spec says the same: "the highest sequence it processed" (`docs/specs/bnp-v1.md:158-159`).
- On resume the server replays only `sequence > cursor` (`bnp_host.rs:348`) and answers `Replaying`. The earlier report is never sent, and the client is never told about the gap.
- **Confirmed** with a review-only test. The map puts the hub with team 2 and venue 1 30 ms away. Team 2 rests a bid, the FIX maker fills it, and team 2 then sends a kill switch. The kill switch (seq 9) arrives before the fill (seq 8). The client disconnects with cursor 9 and reconnects: `resume = Replaying`, and the replay is only `[ReplayComplete { through_sequence: 9 }]`. The fill is never delivered.
- The same cause also lets live messages overtake replayed ones right after a resume.
- Possible fixes: make the private stream sequence-ordered per participant, holding each report until all earlier ones are due. Or make the cursor "highest contiguous sequence" and have the server replay everything above the oldest undelivered one. Either way, the e2e test needs a two-distance case.

### F3. High: unauthenticated connection exhaustion locks out every registered team

- `max_connections` must not exceed the roster size (`config.rs:629-630`). The accept loop admits up to `2 x max_connections` sockets into the slot queue before any TLS (`acceptor.rs:96-121`, `bnp_host.rs:74-89`). Each one holds its place for up to `handshake_timeout_ms` (up to 60 s) without presenting a certificate.
- **Confirmed** by hand with `max_connections: 5`. Twelve idle `socket.create_connection` calls from Python, with no TLS at all, made a registered team's `bunting-trader account` fail at once with `TLS error: unexpected end of file`. After the idle sockets closed, the same command connected. An attacker who just keeps reconnecting keeps every team out.
- Possible fixes: put no limit on pre-authentication sockets beyond a separate per-IP or global handshake budget that doesn't share capacity with sessions. Count `max_connections` only after the roster check. Use a much shorter pre-Hello timeout.

### F4. Medium (inferred from the code, not reproduced): a stalled client can lock itself out of reconnecting

- The session thread writes TLS on a blocking socket with no write timeout (`bnp_host.rs:602-608`; no `set_write_timeout` anywhere in `apps/bunting-server/src`).
- Suppose a client vanishes without a FIN while a feed is flowing. Once the send buffer fills, `channel.send` blocks. The heartbeat-timeout check (`bnp_host.rs:443`) never runs, and the participant claim stays held. The team's reconnect is refused after `SESSION_HANDOVER` (5 s) until the kernel gives up on the TCP connection, which can take many minutes.
- FIX shares this pattern. A write timeout of a few heartbeats, or non-blocking writes behind the outbound hold, would fix it.

### F5. Low: the runbook's revocation step doesn't work as written

- A CRL made with plain `openssl ca -gencrl` (v1, no CRL number) makes the server **refuse to start**: `invalid BNP client verifier: provided CRL could not be parsed: ParseError`. This fails closed, but `RUNBOOK.md` step 4 gives no CRL command.
- A working command needs `crlnumber` and `crl_extensions = authorityKeyIdentifier=keyid:always` in the CA config. With those, revocation worked (see the hand run).
- CRLs are read only at startup. That is documented, and live reload is listed as Target.

### F6. Low: docs and CLI don't match

- `docs/specs/bnp-v1.md:31` says `bunting-trader fingerprint --cert team.pem`. That form fails ("required arguments were not provided: <CERTIFICATE>"). The runbook's `fingerprint team1.pem` is correct.
- `bunting-trader` and `bunting-client` have no kill-switch command (`grep -i kill` finds nothing), although the report lists the kill switch among what the app does.
- There is no end-to-end test of `KillSwitch`. It is only round-tripped in `bnp-wire` tests. A test would have caught F1.

### F7. Low: some private messages depend on where the order was entered

- `OrderAccepted`, `OrderReduced` and `OrderDone` are sent only for orders entered over BNP (`mine()` at `packages/bunting-application/src/bnp.rs:209`). `OrderRested`, `OrderCanceled` and `Fill` go to the participant for any order. So a team's FIX order shows up on its BNP session as fills and cancels with `client_order_id 0` and no accept or done.
- `KillSwitchActivated` is sent only when the team itself was the actor (`bnp.rs:355`).

### F8. Info

- When a reply fails, the `Reject` carries `request_type: 0, reference: 0` (`bnp_host.rs:456-460`), so the client can't match it to its request.
- `bnp_crosses_the_same_virtual_path_as_fix` checks BNP against the map but never runs FIX beside it. "Same path as FIX" holds from reading the code (below), not from a test. The planned FIX/BNP parity test would close this.

## Claims verified as stated

- **Same sequencer and latency map as FIX, no BNP speed advantage.** BNP builds the same `Inbound` and calls the same `AdmissionService::admit` (`bnp_host.rs:1004-1018`; FIX at `session_host.rs:444-457`). Release time is `received_us + path` from `LatencyModel::decide` (`packages/admission-sequencer/src/lib.rs:420-438`). Measured delay is only recorded (`measured_one_way_us`) and never ordered on. The same per-destination release floor applies. Session messages (Ping/Pong, probes, heartbeats) are undelayed, as the spec says.
- **Real delay counts, with no smoothing and no modes.** The reader thread stamps ciphertext on arrival (`wake.rs`, `bnp_host.rs:294-296`), so TLS cost is inside the stamp, and there is no compensation path. Frames pipelined behind `Hello` are stamped at `Welcome` time, which is later than true arrival: a small penalty, never an advantage.
- **Identity only from the registered certificate.** Identity is the SHA-256 of the leaf DER, looked up in the roster (`bnp_host.rs:220-240`). `Hello.client_name` is ignored, and no client message carries a participant. The impostor test above confirms it. Config validation requires 64-character lowercase hex and unique fingerprints and participants (`config.rs:606-625`). TLS 1.3 only. Client certificates are required through `WebPkiClientVerifier` (chain, expiry and clientAuth EKU) with CRLs (rustls 0.23.42, webpki 0.103.13).
- **Gap reported when the window can't cover the cursor.** `subscribe_resuming` (`distributor.rs:222-261`) and `publish` hold the same lock, so resume and the live stream are atomic. An out-of-window cursor gets `Gap`, and `OpenOrders` and `Account` carry `run_sequence`. The existing e2e test covers this. F2 is the exception, where the cursor itself is wrong.
- **ADR 0040 supersedes parts of ADR 0031 correctly.** Both status lines are amended (0031 says "partly implemented ... per ADR 0040, which replaces this ADR's codec candidate ... and identity mapping"). `docs/README.md` rows for 0031, 0040 and `specs/bnp-v1.md` are updated. This is a partial supersession, so no historical banner is needed, and 0040 has the standard eight `##` sections plus its status block. Minor: ADR 0033's supersession table doesn't mention 0040, but 0033 is a dated record.
- **The spec matches the code** apart from F2 (cursor semantics), F6 (fingerprint syntax) and F1. Kill-switch semantics aren't spelled out in the spec, which only says the hub applies it.
