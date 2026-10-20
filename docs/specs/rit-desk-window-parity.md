# RIT window parity for the desk

Status: **Target** (2026-10-20). This is the acceptance contract for the
desktop app's windows (`apps/bunting-desk`, ADR 0050 and ADR 0051). It
replaces `rit-tui-parity-matrix.md` for the desk; that matrix stays as the
TUI's FIX-era record.

## Owner requirement (2026-10-20)

Every window a trader can open must be, at a minimum, the RIT client's
module of the same purpose, with the same behaviour and interactions.
"At a minimum" means Bunting may add to a module (venue choice, order-by-
order depth, themes, floating windows) but may not drop a column, control
or gesture RIT has. Charts are universal: any security, not only the
trader's own equity.

## Sources (observed, 2026-10-20)

The three guides describe the same RIT 2.0 client through the Liability
Trading 3 case:

- Rotman FRTL, *RIT User Application (RIT Client) Feature Guide*, build
  1.06 (10 pp.):
  <https://rotmanfrtl.github.io/RIT%20User%20Application%20(RIT%20Client)%20Feature%20Guide.pdf>
- University of Lethbridge, *RIT 2.0 Client Software Feature Guide v1.0
  (LT3 Case Tutorial)* (14 pp.):
  <https://www.ulethbridge.ca/sites/default/files/NEW%20RIT2%20-%20Client%20Software%20Feature%20Guide_v1.0%20%28LT3%20Case%20Tutorial%29.pdf>
- University of Calgary (Haskayne), *RIT User Guide: Client Software
  Feature Guide*, build 1.04 (14 pp.):
  <https://haskayne.ucalgary.ca/sites/default/files/teams/94/RIT%20-%20User%20Guide%20-%20Client%20Software%20Feature%20Guide.pdf>

The guides are copyright Rotman School of Management. This spec
paraphrases behaviour; it copies no text, layout or image (AGENTS.md
"prohibited to copy"). Behaviour that the guides do not state is marked
*unresolved*.

## How to read the tables

- **Data:**
  - **BNP** means BNP v1 already carries what the window needs.
  - **Derived** means the desk computes it from committed BNP data it
    has, with a label saying so.
  - **Gap** means BNP must be extended. Gaps are collected in "Protocol
    gaps" at the end.
- **State** is the desk at `integrate/desk-u1-u2` (2026-10-20):
  - **Done** matches RIT;
  - **Partial** says what is missing;
  - **Missing** has no equivalent window yet.

## The frame

| RIT element | Required behaviour | Data | State |
|---|---|---|---|
| Module menu (top bar) | One button per module. Each click opens another copy, so several of one kind can be open. | — | Partial: the sidebar "+" menu, the palette and the command field open any window, any number of times. A one-click module bar (or the U3 command line) is still to come. |
| Saved workspace | The layout is restored at the next login. Named workspaces can be saved and loaded. | — | Done (screens, ADR 0051). |
| Windows menu | Close all windows, and arrange them. | — | Partial: Tile and Cascade exist. **Close all** is missing. |
| Case status bar (bottom) | Running or paused, the current period, the time remaining, and the case name. | Gap (period, time remaining, case name) | Partial: run status and logical time in the sidebar. |
| API / RTD / API Orders indicators | Green when the interface is enabled, grey when it is off, red on error. Clicking shows details. | Gap (which interfaces the venue enables) | Missing. Bunting equivalent: BNP session, FIX session and order permission. |
| Help Files | Install guides, the feature guide and API documentation. | — | Missing: a Help window linking Bunting's protocol docs (`PROTOCOL.md`, `docs/specs/bnp-v1.md`). |
| Case Files | The case brief and support files for the run being traded. | Gap (case brief published by the venue) | Missing. |

## Portfolio

RIT shows one row per security in the market, whether or not the trader
holds it.

| Column | Behaviour | Data | State |
|---|---|---|---|
| Ticker | The symbol. | BNP | Done |
| Type | Stock, spot, future, index or option. | Gap (instrument kind) | Missing |
| Contract size | The unit multiplier. | Gap (multiplier) | Missing |
| Position | Signed: long positive, short negative. | BNP (`Account`) | Done |
| Cost | Volume-weighted average price of the current position. For a short, the average sale price. | Derived (own fills) | Missing |
| Last | Last trade price. The background is green on an up-tick and red on a down-tick. | BNP (trade feeds) | Missing |
| Bid / Ask | Best bid and best ask. | BNP (L2) | Missing in Portfolio |
| NLV | Value of closing the position now: at the best bid if long, at the best ask if short. | Derived | Missing |
| Realized P&L | Money made or lost on closed quantity. Commissions count as a loss. | Derived (own fills and fees) | Missing |
| Unrealized P&L | P&L of closing the open position at the best bid (long) or best ask (short), excluding commissions. | Derived | Missing |
| VWAP | Market VWAP of all trades in the security. | Gap (history before the desk connected); Derived from feeds since connect | Missing |
| Volume | Cumulative traded volume. | Gap (as VWAP) | Missing |
| Open buy / open sell | Bunting addition. | BNP | Done |

With several venues, the Bid, Ask and Last columns show the best venue
**labelled with its venue**, or one row per listing. Never show a merged
book (ADR 0050 decision 7). How RIT treats multi-venue securities is
*unresolved* (its cases are single-venue).

## Book Trader

| Behaviour | Data | State |
|---|---|---|
| Bids on the left and asks on the right. Columns are broker, volume and price, best first (bids descending, asks ascending). | BNP (L3 order-by-order and broker IDs) | Done (also place and ahead) |
| A new order flashes green for 250 ms. An order traded against flashes red for 250 ms before it goes. Your own orders are highlighted blue. | BNP | Done |
| Your position and cost for the security are shown at the top. | BNP plus Derived (cost) | **Missing** |
| Lightning toggle on/off, with V (volume) and O (offset, in ticks). | — | Done |
| One-click limit order: left-clicking a bid sends a buy at its price plus O; left-clicking an ask sends a sell at its price minus O. | — | Done |
| One-click market order: right-clicking anywhere in the asks buys V; right-clicking anywhere in the bids sells V. The order walks the book for any remainder. | — | Done |
| Swipe: shift+right-click a price to take everything up to that price (buy in asks, sell in bids). | — | Done |
| Alt+left-click your own orders to cancel them. | — | Done |
| Choose the security inside the window. | — | Done (instrument chips) |

## Ladder Trader

| Behaviour | Data | State |
|---|---|---|
| Five columns: your bid quantity, bid size, price, ask size, your ask quantity. | BNP (L2 plus own orders) | Done |
| Prices descend, centred on the market. | — | Done (Centre/Space recentres) |
| Your position and cost are shown at the top. | BNP plus Derived | Partial: position shown, cost missing |
| Left-click a size cell: a limit order for V at that price. No market orders from the ladder. | — | Done |
| Left-click your own quantity: cancel those orders. | — | Done |
| Your resting order shows in the size column at its price. | BNP | Done |

## Order Entry

| Behaviour | Data | State |
|---|---|---|
| Choose the security by ticker. | BNP | Partial: follows the desk instrument. Needs its own ticker picker in the window. |
| BUY/SELL toggle. | — | Done (segmented) |
| Volume field with stepper buttons of **10** per click, and direct typing. | — | Partial: the stepper step must be 10 lots (RIT), not 1 |
| LMT/MKT toggle. Market orders have no price. | — | Done |
| Price field with stepper buttons of **one tick** per click, and direct typing. | — | Done (verify the step is 1 tick) |
| The submit button says **"Submit BUY"** or **"Submit SELL"**. Changing fields never sends. | — | Partial: the label is "Buy 1 BNT @ … on Venue 1". Keep the detail, but lead with the RIT verb: "Submit BUY · 1 BNT @ 1477 · Venue 1". |
| A limit order that crosses the spread executes like a market order. | engine | Done (engine) |
| **"+" opens another Order Entry window; "−" closes this one.** | — | **Missing**: put a "+" in the window frame's actions for Order Entry; "−" is the existing close. |
| Venue choice, TIF, iceberg, post-only, anonymous. | — | Done (Bunting additions) |

## Trade Blotter

| Behaviour | Data | State |
|---|---|---|
| Tabs: **Live/Partial, Filled/Cancelled, Tenders, Endowments**. | BNP | **Partial**: Blotter shows live orders only. Fills is a separate window. Tenders are in "Tenders & OTC". Endowments are missing. |
| Live/Partial shows the filled quantity out of the order size, and a **Cancel** button per row. Cancelling a partial cancels only the rest. | BNP | Partial: verify the per-row cancel and the filled-of-size column |
| Filled/Cancelled keeps the history of done orders. | BNP (private stream) | Missing as a tab |
| Orders cannot be modified (RIT). Bunting may add replace later. | — | Done |
| Endowments: securities granted by the case. | Gap (endowment events) | Missing |

## Tender offers

| Behaviour | Data | State |
|---|---|---|
| An institution offers a trade at a quantity and price. The trader accepts or declines. A declined offer may go to another trader. | BNP (ADR 0043) | Done in "Tenders & OTC". RIT also shows them in the Blotter's Tenders tab, so add that tab. |
| The offer shows its price against the market and its size against liquidity. | BNP | Partial: show the current best bid/ask and depth beside the offer |

## Transaction Log

| Behaviour | Data | State |
|---|---|---|
| Every credit and debit to the cash balance, with time, amount, reason and resulting balance. | Derived (fills, fees, tender and OTC settlements); Gap for other ledger postings | **Missing** (new window kind `transactions`) |
| Right-click → "Copy to clipboard" copies the table as tab-separated text that pastes into a spreadsheet. | — | Missing |

## Kill All Orders

| Behaviour | Data | State |
|---|---|---|
| One button in the top bar cancels all your live orders. | BNP | Partial: "Cancel all" exists in Order Entry and the palette (F12). It also needs a top-bar button. Bunting's participant kill switch is a separate, confirmed action. |

## News

| Behaviour | Data | State |
|---|---|---|
| Columns: **Tick** (seconds of run time), **Symbol** (if any), **Headline**. | BNP | Partial: verify Tick and Symbol columns |
| The selected story's body is shown under the list. | BNP | Partial: the body sits beside the list. Put it below, as RIT does, or follow the window's aspect ratio. |
| Right-click → "Copy to clipboard". | — | Missing |

## Trader Info

| Behaviour | Data | State |
|---|---|---|
| Trader ID and name. | BNP (participant ID); Gap (name) | Partial |
| P&L. | Derived; Gap (the venue's own figure) | Missing |
| Trading limits table: gross and net limit per security, with current use. | Gap (risk limits over BNP) | Missing |
| Cash balance and reserved. | BNP | Done |

## Charting

Charting is one window kind (`chart`) with two modes.

| Behaviour | Data | State |
|---|---|---|
| **Security chart**: candlesticks of price against run time for **any** security. | Derived (trade feeds since connect); Gap (earlier history) | **Missing** |
| Choose the security by typing a ticker and pressing Enter, or from a "+" drop-down. | BNP | Missing |
| Several securities on one chart. | — | Missing |
| Bar size picker ("S:"). | — | Missing |
| Indicators from a "+" menu: MA, EMA and RSI at a minimum. | Derived | Missing |
| Each venue is its own series, never merged (Bunting rule). | — | Missing |
| **P&L chart**: the trader's P&L against the period's progress. | Derived; Gap (the venue's valuation) | **Missing** |

## Modules RIT has in other cases

These are not in the LT3 guides above, but RIT has them for other cases.
Each needs its own source pass before it is specified:

- Assets;
- Leases;
- Converters;
- OTC (Bunting already has OTC in "Tenders & OTC").

## Protocol gaps (one ADR: BNP reference data, history and valuation)

1. **Instrument reference data**: kind, contract multiplier, tick size,
   currency code and name. The engine has kind and multiplier; BNP
   `Listings` carries only listing and symbol.
2. **History**: past trades or bars per listing from run start, for
   charts, market VWAP and volume. Today the desk sees only trades after
   it subscribes.
3. **Valuation**: the venue's own marks, realized and unrealized P&L and
   NLV per participant, so the desk shows the figure that scoring uses.
   The desk's derived figures are labelled until then.
4. **Trader identity and limits**: the participant's display name, and
   risk limits with their current use.
5. **Case information**: case name and brief, period, period length and
   time remaining, plus endowments.
6. **Interface status**: which interfaces this participant may use
   (BNP orders, FIX, data).

## Acceptance

A window is at parity when:

- every row of its table is Done;
- it is tested in a headless window test (gesture → request, or data →
  cell);
- it is accepted by screenshot against a live venue with several
  participants trading.

Derived figures carry a visible label until their gap closes.
