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
