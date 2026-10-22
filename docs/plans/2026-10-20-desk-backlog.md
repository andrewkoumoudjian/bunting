# Desk backlog (2026-10-20)

Status: **Current.** This is the working TODO list for the desktop app
(`apps/bunting-desk`, ADR 0050 and ADR 0051). Items move to the
implementation log when they land. Owner requests are quoted with their
date.

## In progress

- **U2b fix round 1** (from live screenshots of the merged desk):
  - Order Entry's submit button and kill switch get cut off. Fix with a
    sticky footer and scrolling fields.
  - Pane bodies scroll instead of clipping.
  - The Ladder V input is too narrow.
  - Trader Info gets a quieter refresh control.
  - Long notices truncate.
  - Book Trader columns fit narrow windows.

## Findings from the first multi-desk run (2026-10-20, three desks, one shared venue)

- **Working:**
  - three desks claimed participants 1, 2 and 3 on one server;
  - all three saw the same book;
  - a one-click bid rested, marked "you", with an order trip of 1.6 ms.
- **Blocking practicality: humans cannot trade with each other.**
  - Desk 2's market sell was refused with "insufficient inventory": BNT
    is not shortable and local participants start with no position.
  - Fix: give local participants starting positions (an endowment), or
    make the practice instruments shortable within limits.
- **Portfolio and Trader Info are stale.** They showed the account
  snapshot from connection time ("committed sequence 78, received 83 s
  ago", open buy 0 with a 100-lot bid resting). The account must refresh
  after fills and order changes.
- **Truncated statuses.** The Blotter status "rejected: insufficient in…"
  truncates with no tooltip; the rejection reason must be readable.
- **Unrealistic price path.** BNT drifted from 100 to 240 in about 90
  seconds. The momentum agent dominates, so the scenario needs a
  mean-reverting fundamental (see `rit-case-primitives.md` §7).
- **Log noise.** The join probe's bare TCP connect is logged by the venue
  as "BNP TLS handshake failed".

## Second multi-desk run (2026-10-20, after `a2d8127`)

- **Fixed:**
  - Desk 1 bought 10 QRC @ 100 and desk 2 sold short into it at market.
  - Both Portfolios updated within a second: +10 and −10 at a cost of
    100.
  - The book shows about 6 levels and 360–440 lots a side, and BNT
    holds at 99–100.
- **Open:**
  - **The price is pinned, not lively.** No existing agent kind trades
    toward a moving fundamental. A seeded random-walk fundamental agent
    (RIT LT3-style, `rit-case-primitives.md` §7) is the next agent
    slice.
  - **Signals skip the venue shutdown.** A desk stopped by SIGTERM or
    SIGINT skips the last-client shutdown, so the venue keeps running.
    The desk should handle those signals by quitting the app cleanly.
  - **Latency under load.** One desk measured an 85 ms order trip while
    another measured 7 ms, under heavy CPU load from builds. Re-measure
    on an idle machine before calling it a bug.

## Third multi-desk run (2026-10-21, integrate `adf35a3`, release build)

- **Owner report:** "my trades don't get filled" on a local machine.
- **Reproduced with the published `v0.2.0-preview.1` Linux archives:**
  - QRC had no agents on either venue, so QRC orders rested forever.
  - BNT ran from 100 to about 180 in roughly a minute: a single
    5-lot static provider faced a momentum buyer.
  - Sells were refused because nothing was shortable.
  - SIGTERM left the venue running.
- **Fixed on integrate:**
  - Agents on every listing, a random-walk fundamental, resting provider
    quotes, and signal quit (`sonnet/fundamental-walk`).
  - Order Entry preselects a security's only venue (`21a8ecb`).
  - Chart review rounds 1–2 (`muse/charts`).
- **Measured** (Linux x86_64, release build):
  - All four listings were 5–6 levels deep.
  - QRC market orders filled on V1 and V2, and a BNT short filled on V2,
    from the CLI and from two desks (participants 1 and 2).
  - The full 32-agent practice venue used 3.1% CPU and 26 MB RSS after
    60 s.
  - The desk used 29% CPU on the headless output.
  - Desk SIGTERM → desk exits → venue stops (last client).
- **Gates on `adf35a3`:**
  - metadata, fmt, clippy `-D warnings`, the orderbook-rs tree check,
    wasm32 check and `git diff --check main` all passed.
  - `cargo test --locked --workspace`: 85 suites, 339 passed, 0 failed,
    1 ignored.
  - `tools/check_desk.sh`: 113 tests passed.
- **Open:**
  - Book Trader clips Venue 2's asks when two venues sit side by side.
  - The P&L time axis labels collide at the start.
  - Average cost is rounded to a whole tick.
  - Charting opens BNT V1+V2 in % comparison: it should open on one
    series and overlay same-instrument venues on the price axis. With
    `muse/charts`.
  - **1 tick = 1% of price** makes many candles dojis. Finer ticks need
    the tick size and display scale on BNP (ADR 0054, Proposed, owner
    questions open).
- **Venue crash found live after the resting-quote change, then fixed:**
  - Log: `deterministic runtime failed: … InvalidTransition { from:
    PendingCancel, report: Rejected { UnknownOrder } }`.
  - Cause: a taker filled a provider's quote while its cancel was in
    flight, and the cancel's reject reached the agent before the fill.
  - Fixed in `01bc183`. `efdc875` makes the other cancel, expiry and fill
    races idempotent, and `20bba74` retires a failing agent
    (deterministic, in the snapshot) instead of stopping the venue.
  - Stress test (release build): six CLI participants sent random market
    orders to all four listings for 3 minutes, 26,600 orders. The venue
    stayed up with no agent retired, and every book was still quoted.
  - Gates on `20bba74`: all seven pass, and the workspace tests ran
    85 suites, 346 passed, 0 failed, 1 ignored.
- **Chart review round 3, merged as `2caf239`:**
  - Labels never overlap.
  - Book Trader venue cards fit or stack.
  - Average cost keeps its decimals.
  - Charting opens on one series, and same-instrument venues overlay on
    the price axis.
  - `tools/check_desk.sh`: 118 tests passed.

## Next release targets (owner, 2026-10-22)

- **Configurable onboarding at launch** (owner: "make sure the next
  bunting release include configurable onboarding when app is launched
  so user have choice to connect to either local, remote, etc.").
  - A connect screen with "Practice locally" or "Hosted venue", the
    last choice remembered.
  - Flags skip it.
  - First cut in `apps/bunting-desk/src/onboarding.rs`.
- **More liquidity from better, more proactive market making** (owner:
  "more liquidity into the market by better and more proactive market
  making and liquidity agents").
  - First step (2026-10-22): practice providers re-quote every 250 ms
    and quote 100–400 lots per level. Measured at about 1,200 lots a
    side, against about 400 before.
  - Next: makers that do the following, each through the same
    sequencer:
    - quote inside the spread when it widens;
    - refill a level within one wake after it is taken;
    - skew on inventory without leaving the market;
    - layer more levels as volatility rises.
  - Finer ticks (ADR 0054 display scale) so quotes can sit 1 cent
    apart, not 1%.
  - **What the specs already require** (read 2026-10-22; owner: "read
    the existing specs and docs on how all the simulated agents should
    be on the market"):
    - **`docs/specs/rit-class-market-simulation.md`:**
      - SIM-AGENT-001: agents submit ordinary commands through risk and
        the sequencer, and never touch a book.
      - SIM-AGENT-002: named, seeded RNG streams, snapshotted.
      - SIM-AGENT-003: noise traders with bounded arrival, side, size
        and price displacement.
      - SIM-AGENT-004: **inventory-aware liquidity providers with
        configurable levels, spread, quantity, replenishment and
        withdrawal**.
      - SIM-AGENT-005: informed agents on a versioned fundamental path.
      - SIM-AGENT-006: further kinds only with provenance and tests.
    - **`docs/specs/rit-case-primitives.md` §7:**
      - per-security liquidity and volatility levels (LT3);
      - extra closing liquidity, so the close cannot be pushed;
      - an option market maker at a lagged volatility;
      - a price authority for regulated spot;
      - news-linked fundamental jumps.
    - **`docs/research/2026-10-08-agent-market-simulation-implementations.md`
      and `…-expanded-market-algorithm-survey.md`:**
      - a seeded Poisson / zero-intelligence null;
      - then **queue-reactive, state-dependent flow** as the preferred
        background generator;
      - a maker layer with quote IDs, queue position, cancel/replace,
        inventory limits and markouts;
      - AS/GLFT only as faithful policies, never relabelled
        heuristics;
      - latent value feeds agents and is never a second tape;
      - tickets T0–T3.
  - **Gaps against those requirements today** (`bunting-agents`,
    `local_venue.rs`):
    - each `fundamental_liquidity_provider` quotes one level per side;
    - it has no inventory skew (the skew applies only to the
      inventory-skewed kinds) and no withdrawal;
    - it refills only on its next wake;
    - the noise trader fires on a fixed wake, not a Poisson clock;
    - nothing adds closing liquidity.
  - **Plan:**
    - multi-level providers (levels, spacing, size curve) that skew on
      inventory, refill on fill reports and widen or withdraw under
      stress;
    - Poisson-timed noise flow;
    - per-listing liquidity profiles (low, medium, high);
    - a closing-liquidity schedule;
    - each change measured live (depth, spread, trades per second,
      price path) like the 2026-10-22 run.
- **An agent on QUARCC's order management** (owner: "implement agent
  that uses the built in quarcc orders management (ported from CPP)").
  - Today the built-in policies only emit QUARCC execution intents.
    They do not run QUARCC's order manager.
  - The new agent runs a strategy on `quarcc-execution-engine`, the
    port of the C++ QUARCC engine:
    - its order manager and ID mapping;
    - participant risk and the kill switch;
    - positions, and reconciliation against committed reports.
  - It submits through the same sequencer as every agent (ADR 0035).
  - QUARCC stays participant-side, never market authority (AGENTS.md).
  - Read `docs/reference-functionality-audit.md` (QUARCC rows) first.
- **Hosted venue on the Oracle instance** as the second connection
  option (below).

## Owner feedback on v0.2.0 (2026-10-22)

- "the traders (simulated agents) on the market doesnt give enough
  liquidity and doesnt smooth out the market for me to trade".
  - Cause (observed in `local_venue.rs`): one tick is 1% of the price,
    so five providers quoting 1 to 5 ticks from the fundamental give a
    2-tick (2%) touch spread and 5 levels a side. Every fundamental
    step moves the whole book by 1%.
  - A smooth market needs finer ticks, which needs the display scale
    on BNP (ADR 0054, Proposed).
- "the PnL graph is bugged and stops showing evolution at a certain
  point".
- "cost for each stock doesnt show up".
- "the candle chart should be dynamic and not have spaced out candles".
- Decompile the original RIT client (owner sent it by Taildrop; 1.8.466
  from the FRTL repository was used until it arrived) to list every
  feature and spec the desk needs at a minimum.
- "add the same sounds for order filled, etc as we have in RIT client
  and the same onboarding flow".
- **TODO:** probe `flserver.rotman.utoronto.ca` to learn how Rotman
  hosts its RIT server, as inspiration for hosting ours. Only passive,
  public observation: DNS, reachability and the ports the RIT client
  documents (10000 for the client, 9999 for the local REST API). No
  scanning beyond that and no login attempts.
- **TODO** (owner, 2026-10-22): "implement and test the bunting
  server … by the server i mean hosting it onto our oracle instance (in
  our tailnet) … it is a secondary connection option other than local".
  - **The host** (observed 2026-10-22 over SSH as `opc`):
    - `instance-20261019-1530`, tailnet `100.113.213.48`;
    - Oracle Linux 9.8, aarch64, 2 vCPU, 10.9 GB RAM;
    - Docker installed, about 4 GB free on `/` (87% used).
  - **Serve** the practice market as a durable `bunting server` (the
    aarch64 release binary or the container image, ADR 0044), with BNP
    listening on the tailnet address only.
    - A roster of participant certificates handed to desks.
    - The server certificate names the tailnet host.
  - **Desk:** a "Join hosted venue" choice on the connect screen next to
    "Practice locally" (the default). It remembers the address and
    certificate files.
  - **Test it end to end:**
    - several desks on different machines trading, with latency
      measured;
    - a restart and recovery in durable mode.

## Next

**Binding contract:** `docs/specs/rit-desk-window-parity.md` (owner,
2026-10-20): every window must behave like RIT's module of the same
purpose, at a minimum. Charts are universal: any security, several
securities, bar sizes and indicators, plus a P&L chart. The items below
are ordered steps toward that contract.

1. **Practicality test with several identities** (owner, 2026-10-20:
   "test practicality of the app by building it, spinning up multiple
   instances with multiple identity and connect to the same server
   (market) and trade").
   - One `bunting server` with a roster of three participants, each with
     its own certificate.
   - Three `bunting-desk --server … --cert … --key …` instances, each on
     its own configuration directory.
   - Trade between them: a resting bid from one desk, hit by another via
     one-click, then cancel, kill switch and reset.
   - Screenshots of each desk, with every defect found filed and fixed.
   - **The local practice venue must be a market by default** (owner,
     2026-10-20: "default shouldn't be one participant, it defeats the
     whole goal of a market").
     - The first `--local` desk (or TUI) starts one shared local venue
       in a well-known directory, with a roster of several participant
       identities, all funded.
     - Every further `--local` client on the machine finds that running
       venue and joins it as the next free participant.
     - `--participant N` picks a specific identity.
     - The built-in agents keep quoting. The venue stops when its last
       client exits, unless it was started on its own.
2. **Equity graph in Portfolio** (owner, 2026-10-20: "in the portfolio we
   should be able to see the equity graph").
   - **Step 1, client-side:**
     - equity = cash + Σ position × the last trade price the desk has
       seen on that instrument's feeds, sampled on the run clock into a
       bounded series in `bunting-trading-state`;
     - drawn as a line chart in the Portfolio window, labelled "marked
       at last trade seen" so it is never confused with the venue's own
       scoring.
   - **Step 2:** the venue's own valuation and history over BNP (see
     item 4).
3. **Price chart window** (owner, 2026-10-20: "add a window option a price
   graph, just like RIT").
   - A new `PaneKind::Chart` (saved name `chart`, command code `G`):
     - candlesticks or a line per listing;
     - each venue shown separately, never merged (ADR 0050 decision 7);
     - timeframes on the run clock;
     - volume bars;
     - the window's own instrument.
   - Bars come from a bounded per-listing trade series in
     `bunting-trading-state` built from the direct feeds.
   - Uses gpui-kit's chart components.
4. **BNP history and valuation ADR.** Charts and equity should not depend
   on when the desk connected. The ADR covers:
   - a request for past trades or bars per listing;
   - the venue's marks and computed equity per participant.
   RIT shows the whole case's history from the start.
5. **U3:**
   - the command line with function codes (ADR 0051 decision 4);
   - colour link groups and per-window instrument (decision 5);
   - the Watchlist and Inbox sidebar sections.
6. **Copy and number formatting** (from the candid screenshot review
   below):
   - **Trader Info and Portfolio:**
     - "As of committed sequence N, received X s ago" becomes
       "Updated X s ago", with the sequence number in a tooltip;
     - currencies show as a code once BNP carries reference data, and
       until then as "Cash";
     - large numbers use digit grouping.
   - **Run section:** a status the venue does not report shows "—"
     rather than "not reported".
   - **Notices:** times use the same clock format as the tape, so a
     notice at run start no longer shows "0.000".
   - **Order Entry:**
     - the selected Buy/Sell segment is a solid market colour with
       legible text in both themes;
     - segmented controls and the venue chip use the same small control
       height as the toolbars.
   - **Book Trader:** with a single venue, the venue card fills the
     window's height instead of leaving it empty below.
7. **Local venue cleanup.**
   - A desk killed by a signal leaves its local `bunting server` child
     running. Stop the child when the parent dies, for example with
     `PR_SET_PDEATHSIG` on Linux, and handle the platform equivalents.

## Engine: RIT case primitives

Owner, 2026-10-20: "make sure that our app will have ALL of the proper
primitives so that we can run all of those cases simulations." The
contract is `docs/specs/rit-case-primitives.md`. Six engine features, each
with its own ADR then slice, in this order:

1. settlement and close-out, plus portfolio limits;
2. the fines framework and tender kinds;
3. options settlement and the Black-Scholes market maker;
4. converters and FX;
5. roles, endowments, forwards and the price authority;
6. sub-heats and RITC scoring.

Each step ends with a playable case scenario run with several desks.

## Later

- Second-monitor use (ADR 0051 decision 6, unresolved).
- IBM Plex Mono for numbers, if tabular Inter proves insufficient.
- Charts that need history beyond the desk's session (item 4).
