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
