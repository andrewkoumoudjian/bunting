# Desk feedback on v0.2.0, RIT client audit, hosted venue (2026-10-22)

Status: **Current.**

## Owner requests

- "the traders (simulated agents) on the market doesnt give enough
  liquidity …"
- "the PnL graph is bugged and stops showing evolution at a certain
  point"
- "cost for each stock doesnt show up"
- "the candle chart should be dynamic and not have spaced out candles"
- "decompile [the RIT client] … so we can continue our implementation of
  the bunting desktop app while knowing exactly all the features and
  specs we need to have AT A MINIMA"
- "add the same sounds … as we have in RIT client and the same
  onboarding flow"
- Hosting on the Oracle instance as "a secondary connection option other
  than local".
- "make sure also that PNL of portfolio and cost basis updates all the
  time and doesnt stop".

## What changed

### Cost and P&L stopped updating (`bunting-trading-state`)

- **Cause (observed in the code):**
  - An `Account` answer whose position differed from this client's fills
    was adopted at once, and the cost basis was marked unknown.
  - Fill reports and account answers travel separately. A fill on the
    farther venue can arrive after an answer that already counts it.
  - Cost then showed "—" until the position went flat. The P&L total lost
    its unrealized part, so the chart went flat.
- **Fix:** `PnlBook::account` adopts a difference only when the same
  number of lots has differed for 2 s (`ACCOUNT_GRACE_US`). A difference
  that in-flight fills explain closes first. A real endowment is still
  adopted.
- **Second freeze:** P&L samples marked from the last two-sided touch
  with no age limit, so a book one-sided for long froze the series.
  Samples now use the live mark once the sampled touch is older than 2 s.
- **Tests:**
  - `an_account_answer_ahead_of_its_fill_report_keeps_the_cost`;
  - `a_difference_that_changes_restarts_the_grace_period`;
  - `a_book_one_sided_for_longer_than_a_requote_does_not_freeze_samples`;
  - the endowment test, updated for the grace period.
- **Still derived:** a desk that connects with a position already open
  has no fills behind it, so that position's cost stays unknown until it
  is flat. The fix is the venue's own cost basis and P&L on BNP `Account`
  (ADR 0054, Proposed, owner decision).

### Candles

- `Model::continuous_bars` gives every period from the first trade to
  "now" a bar. A quiet period is a flat bar at the last close with no
  volume, and the forming bar is live.
- The chart keeps at least the view's bar count of band slots, so a short
  history draws at normal candle width from the left instead of a few
  candles stretched across the window.
- **Tests:**
  - `continuous_bars_fill_quiet_periods_and_run_to_now`;
  - `continuous_bars_stay_bounded_after_a_long_silence`.

### Liquidity (first step)

- Practice providers re-quote every 250 ms, not 500 ms.
- They size up with distance: 100, 150, 200, 300 and 400 lots, instead of
  40 at every level.
- The remaining plan, from the agent specs (SIM-AGENT-003/004/005, the
  case primitives §7 and the 2026-10-08 research), is in the desk
  backlog. A smooth market needs finer ticks (ADR 0054 display scale).

### RIT client 1.8.466 audit

- New: `docs/research/rit-binary-audit/desk-client-inventory-1.8.466.md`.
- It covers:
  - every window and gesture, the settings and the toasts;
  - the exact sound triggers;
  - the onboarding flow;
  - case-gated modules;
  - the gaps against the parity spec.
- **Method:** a static read with `pymsi`, `dnfile` and `dncil`. The
  session's permission policy blocked running `morluto/rea` through
  `npx`.
- **Storage:** the derived dumps stay outside Git.
- **Finding:** RIT has no fill sound. It sounds the OS Exclamation on
  news and on some toasts.

### Sounds (`apps/bunting-desk/src/sounds.rs`)

- **Cues and how they are decided:**
  - fill, reject, news, tender and disconnect;
  - picked by a pure diff of the model (`Watch`);
  - never replays what was there at connect;
  - each cue at most once per 250 ms.
- **Tones and playback:**
  - synthesized tones (no RIT or OS assets), written once as WAV;
  - played through `afplay`, `pw-play`/`paplay`/`aplay` or the .NET
    `SoundPlayer`;
  - no audio crate is linked.
- **Mute:** a speaker button in the title bar (RIT's status-bar toggle),
  "Toggle sounds" in the palette, saved in `sounds.json`.

### Connect screen (`apps/bunting-desk/src/onboarding.rs`)

- With no venue flag, the app opens on a connect screen:
  - "Practice locally", with an optional participant number;
  - or "Hosted venue": address, certificate name, and CA, certificate and
    key files with Browse… buttons.
- The last choice is remembered (`connection.json`).
- Enter connects. Connecting runs off the UI thread, with a staged status
  and the error under the form.
- On success the workspace window opens and the connect window closes.
- `--local`, `--server`, `--participant` and `--venue-dir` skip the
  screen.

### Hosted venue

- `local_venue::VenueSpec` and `prepare_with` provision the practice
  market for any bind address, certificate names, port and durable
  storage. `prepare` is unchanged for local play.
- `bunting host-init --dir … --name … --bind … --port …` writes a venue
  directory and prints what each trader needs.
- **Deployed** on `instance-20261019-1530` (Oracle Linux 9.8 aarch64,
  tailnet 100.113.213.48):
  - `bunting` v0.2.0 aarch64 from the release, checked against
    SHA256SUMS, at `/usr/local/bin/bunting`. SELinux refuses to run it
    from a home directory under systemd.
  - systemd unit `bunting-venue.service`, enabled, restart on failure,
    run in `/home/opc/bunting/venue`.
  - BNP on `100.113.213.48:7400`.
  - firewalld rich rule: TCP 7400 from `100.64.0.0/10` only.
  - Participant keys stay on the provisioning machine
    (`~/.local/state/bunting/hosted-oracle/`), never on the host.

## Measurements

**Practice venue, local** (Linux x86_64, release build, this branch before
commit, 75 s, participant 1):

- **BNT V1 book:** 5 levels and about 1,200 lots a side (average 10.2
  levels, 2,398 lots across both sides). The 2026-10-20 run had about
  415 lots a side.
- **Fills on both venues:**
  - buy 100 on V1, buy 100 on V2, then sell 50 on V2;
  - cost known throughout: 100 lots at 100.00, then 200 at 99.50, then
    150 at 99.50;
  - the P&L series kept moving to the end (62 samples).
- **Bars:** 76 continuous against 75 traded seconds.

**Hosted venue, Oracle, over the tailnet** (`bunting-trader` 0.2.0 from
this branch, Linux x86_64 client):

- **Connection:** mutual TLS succeeded, and participant 1 listed all four
  listings.
- **Participant 1:** bought 100 BNT at market on V1, filled 10 + 90 at
  102, with a 5.2 ms order round trip.
- **Participant 2:** sold 50 BNT short on V2 at 100, with an 11.0 ms
  round trip. V2 is the far venue on the latency map.
- **Positions:** the accounts showed +100 and −50.
- **Network:** measured access RTT was about 3.0 ms.
- **Host load:** the venue process used about 10 MB RSS and 2.3% CPU just
  after start.

**Restart recovery** (local, a `host-init` directory):

- A second `bunting server` on the same `origin.wal` started cleanly and
  appended to it (1.2 MB → 2.3 MB).
- Not yet repeated on the Oracle host.

## Checks

See the closing commit's message for the gate results.

## Remaining

- **ADR 0054 (owner decision):**
  - venue cost basis and P&L on `Account`, so a reconnecting desk always
    has cost;
  - display scale and finer ticks.
- **Maker upgrades:** multi-level, inventory-skewed, refill on fill,
  withdraw under stress. Plus Poisson noise and a closing-liquidity
  schedule.
- **An agent on QUARCC's order manager.**
- **Connect screen:** back to the connect screen on disconnect; case-gated
  modules.
- **Visual acceptance:** the connect screen and the speaker button are
  not visually reviewed (no screenshot pass in this session).
- **Oracle host:** runs the v0.2.0 server. It needs the next release's
  binary for future server changes (none in this change).

## Second round (2026-10-22, owner answers)

The owner's answers:

1. ADR 0054: "its the app that should determine realized and
   unrealized P&L … why is the venue calculating Pnl ???".
2. Certificates "should be generated on connection".
3. Release "if all TODOs are done".

### The desk keeps its own books across a relaunch

- **The gap:** a relaunched desk had no fills behind an open position, so
  its cost and unrealized P&L were unknown until flat.
- **The record:** `bunting-trading-state` `LedgerRecord` holds every book
  (position, exact cost, realized, fees), the P&L samples and the
  private-stream cursor. It is restored only when `Welcome` names the
  same run and participant at or past the cursor. Replayed fills at or
  below the cursor are skipped.
- **The desk side:**
  - it saves the record every 2 s and on exit, under
    `ledgers/<certificate>-<venue start nonce>.json`;
  - it reconnects with `resume_after` at the record's cursor, so the
    venue replays every later report.
  - The venue computes nothing new.
- **Live on the Oracle venue**, participant 3:
  - buy 30, then relaunch without the record: cost unknown;
  - relaunch with the record, then buy 10: cost exactly 40 lots at
    75.00, and the P&L history was restored.
- **Test:** `a_ledger_record_restores_cost_and_pnl_after_a_relaunch`.

### ADR 0054 accepted in part; slice R1 (BNP 1.1 reference data)

- **ADR:** status "Accepted in part". §4 Valuation is rejected (owner),
  and its type bytes stay unassigned.
- **Engine:** `ListingDefinition.tick_size` (default 1) and
  `InstrumentDefinition.quoted_decimals` (default 2), `display_unit` and
  `description`.
  - Defaults serialize as before, so existing runs, snapshots and archive
    hashes are unchanged.
  - Scenario validation requires the lower price bound on the tick
    grid.
  - Risk admission rejects an off-grid limit with the new `RejectCode`
    `PriceNotOnTick`.
- **BNP 1.1:**
  - `VERSION_1_1 = 257`, `negotiate`, `ReferenceDataRequest` 0x34 and
    `ReferenceData` 0x76 with currency, instrument and listing records;
  - reason 27 `PriceNotOnTick`, sent to 1.0 sessions as 11;
  - a 1.1 request on a 1.0 session is logged out;
  - the client offers 1.1 and retries once at 1.0.
  - Spec: `docs/specs/bnp-v1.md` "BNP 1.1".
- **FIX parity:** the `y` SecurityList that answers `x` gains a
  `NoRelatedSym` group with tags 55, 48, 207, 167, 15, 231, 969 and the
  new BuntingQuotedDecimals (10023). It is in the profile, the Orchestra
  XML and `PROTOCOL.md`.
- **Client:**
  - `Model` requests reference data after `Listings` and pages through
    it;
  - it sets contract multipliers (so "contract size assumed 1" goes away
    on 1.1);
  - new accessors: `quoted_decimals`, `tick_size`, `money_decimals`,
    `currency`.
  - `bunting_trading_state::display` holds the shared `ticks`, `fixed`
    and `parse_price`.
- **Desk and terminal:** every price shows at the instrument's quoted
  decimals and every money figure at the currency's:
  - Book Trader, Ladder, Order Entry, Blotter, Portfolio, Transaction
    Log, Deals, Fills, Tape, Charting (axis, legend, MA/EMA) and P&L;
  - Order Entry and OTC parse "100.50" back to ticks, and the price field
    steps one tick.
  - Against a 1.0 venue the decimals are 0 and everything shows as
    before.
- **Practice market in cents:**
  - fundamental and opening mark at 10000 ticks ($100.00);
  - providers 1, 2, 3, 5 and 8 cents from the fundamental;
  - the walk steps every 250 ms on a 10-minute "day".
- **Measured** (Linux x86_64, release build, 60 s, fresh local venue):
  - BNP 1.1 negotiated (257);
  - reference data: 2 decimals, tick 1, multiplier 1 (known);
  - BNT ran 99.45–100.83;
  - touch spread median 2 ticks (2 cents), p90 6, max 9;
  - 9–11 visible levels across both sides;
  - a 100-lot buy and sell filled, cost 99.835;
  - the new `bunting-trader` joined the Oracle v0.2.0 venue at 1.0.
- **Tests added:**
  - `bnp-wire`: the new messages round-trip, and
    `versions_negotiate_to_the_lower_1x_minor`;
  - `risk-engine`: `a_limit_price_off_the_tick_grid_is_refused`;
  - server `tests/bnp.rs` asserts `ReferenceData`;
  - `tests/fix_competition_profile.rs` asserts the `y` reference
    fields;
  - desk and trading-state formatter tests.
- **Remaining for R1:**
  - the scenario currency table (code and name);
  - built-in agents do not yet round to a tick size above 1, which no
    shipped scenario uses.

### ADR 0056: enrolment by invite (owner: "certificates should be generated on connection")

- **ADR:** `docs/adr/0056-bnp-enrolment-by-invite.md`, Accepted.
- **`bnp-wire`:** `Enroll` (0x06) and `Enrolled` (0x4e) messages, and
  `InviteToken` (`bnpi1:<host:port>:<ca-sha256>:<secret>`).
- **`bunting-client::enroll`:** a session without a client certificate.
  Its verifier accepts the venue's chain only through the pinned CA, so
  the secret is never sent to another venue.
- **Server** (`bnp_enrol.rs`):
  - `bnp.enrolment` config (CA key, invites, enrolled file,
    certificate days, max enrolled);
  - the TLS verifier allows unauthenticated clients only when it is set;
  - the roster is shared and growable;
  - an `Enroll` checks the invite (hashed secret, unused, unexpired, one
    per participant), verifies the request's signature, and issues a
    client-auth certificate;
  - it writes `enrolled.json`, marks the invite used, and adds the
    roster entry, all under one lock;
  - a refusal is one generic `Logout`.
  - `rcgen` became a server dependency with `x509-parser` (recorded in
    `docs/reference-adoption.md`).
- **`bunting-trading-state::enrolment`:**
  - `enroll_with_invite` makes a P-256 key here and sends only the CSR;
  - `Identity` save and load;
  - `mint_invite`;
  - `ring` (already in the lock) for the secret and its hash.
  - `local_venue::VenueSpec::with_enrolment()` writes no participant
    keys. It writes `ca.key`, `invites.json`, `fix-credentials.json`,
    and a `venue.pem` chain that includes the CA. Practice configs are
    unchanged.
- **CLI:**
  - `bunting host-init --enrolment`;
  - `bunting invite --dir … --participant N [--hours H] [--address …]`;
  - `bunting-trader enroll <invite> --out <dir>`.
- **Desk:** "Hosted venue" takes an invite. The desk enrols off the UI
  thread, saves the identity under `identities/<venue>-participant-N/`,
  remembers it, and connects. Certificate files remain the alternative.
- **Tests:**
  - `apps/bunting-server/tests/bnp_enrolment.rs`:
    - an invite enrols once, and the certificate logs in without a
      restart;
    - a spent invite, a wrong secret, a second invite for an enrolled
      participant and an invalid CSR are all refused ("invite
      refused");
    - a wrong CA pin fails before the secret is sent, and that invite
      still works afterwards;
    - the enrolled roster survives a restart on a copied venue.
  - `bnp-wire` covers the token round trip and malformed tokens;
    `bnp_enrol` covers the date arithmetic and the PEM bundle.
- **Live** (release build, Linux x86_64): five fresh enrolment venues;
  each `bunting invite`, then `bunting-trader enroll`, then a market buy
  with the new certificate filled.
- **One failure seen during development:** a stale server from an earlier
  run held the old CA key while its TLS chain was re-read from a
  recreated directory. The client refused it as "CA does not match".
  Correct behaviour; operators should not swap the CA under a running
  venue.

### Liquidity agents (SIM-AGENT-003/004)

- **New `AgentConfig` options**, defaulted off so existing scenarios and
  snapshots replay unchanged:
  - `inventory_skew_lots`: a fundamental provider moves both quotes one
    tick against its position per that many lots;
  - `inventory_limit`: past it, the provider withdraws the side that
    would add exposure;
  - `poisson_wakes`: takers wake after exponential gaps with the
    configured mean, bounded to 1 ms–5×.
  - The draw is `exponential_millionths`, integer `-ln u` from a 16-step
    table (error under 0.001), so a replay makes the same draw on every
    host.
- **Practice market:** providers lean 1 cent per 200 lots and stop adding
  past 2,000. Noise, mean reversion and informed traders arrive as Poisson
  processes.
- **Measured** (release build, Linux x86_64, 90 s; a desk market-bought
  300 lots every 2 s for 50 s, 7,500 lots):
  - total visible depth p10 1,900 lots, median 2,250;
  - touch spread median 2 ticks, p90 6, max 13;
  - BNT mid 99.46–101.19, starting at 99.94 and ending at 100.22: the
    pressure was absorbed and the price came back.
- **Tests:**
  - `exponential_draws_match_minus_ln_u`;
  - `poisson_wakes_vary_around_the_mean_and_stay_bounded`;
  - `a_fundamental_provider_leans_from_inventory_and_withdraws_past_its_limit`.

### An agent on QUARCC's order management

- **The policy:** `PolicyKind::QuarccSignalTrader`. Each wake it forms a
  QUARCC-style strategy signal:
  - side from the fundamental's gap to the mid;
  - confidence from that gap over `spread_ticks`;
  - a target position of confidence × `base_quantity`.
- **Execution:** QUARCC's order management cancels or replaces its
  working order until the position reaches the target. The order joins
  the touch below 0.7 confidence and takes above it. Like every built-in
  agent, it goes through `QuarccExecutionEngine`'s pre-trade risk,
  lifecycle, ID mapping and positions.
- **Provenance:** only the `StrategySignal` contract's field meanings are
  used (`docs/ports/quarcc-trading-engine.md`). Nothing is translated
  from the C++ code.
- **Practice market:** participants 44 (BNT) and 45 (QRC) on venue 1, up
  to 500 lots each, fully confident 20 cents away.
- **Measured:** in a 45 s durable practice run, the archive held 192 and
  198 commands from 44 and 45, and they traded on 34 and 29 fills.
- **Test:** `a_signal_sets_a_target_and_the_order_closes_the_distance`.

### Connect-screen follow-ups

- **Switch venue:** a "Switch venue (log out)" action (palette,
  `SwitchVenue`) saves the screens, closes the workspace (the session
  logs out and the local claim drops) and returns to the connect screen.
  This is RIT's logout.
- **Case-specific windows:** Deals (tenders, OTC) stays off the default
  screens and opens itself on the first tender offered, as RIT pops a
  tender as it arrives.
- **Still to come:** gating each case module from the venue's case
  information needs ADR 0054 R4.
