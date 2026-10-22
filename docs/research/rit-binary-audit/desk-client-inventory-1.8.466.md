# RIT client 1.8.466: desk inventory (windows, alerts, onboarding)

Status: **Evidence** (2026-10-22). This is a static audit of the RIT User
Application 1.8.466. It lists what the desk must match at a minimum
(`docs/specs/rit-desk-window-parity.md`), from the binary rather than the
feature guides. It extends the 1.8.456 audit in this directory, which
covers the market and protocol surface. This file covers the trader-facing
client.

Owner request (2026-10-22): "inspect the package, decompile it … so we can
continue our implementation of the bunting desktop app while knowing
exactly all the features and specs we need to have AT A MINIMA", plus
RIT's sounds and onboarding flow.

## Source and method

- **Input:** `RIT User Application-1.8.466.msi`, 9,032,192 bytes, SHA-256
  `ba966cdc2a76390d667b09caa2c7060d0da8ed231ba992a30588a70360dd5842`.
  The owner sent it by Taildrop on 2026-10-22. It is byte-identical to
  the copy in the local FRTL clone
  (`~/.cache/bunting-skills-src/rit/RotmanFRTL.github.io/`).
- **Extraction:**
  - `pymsi` extracted the MSI: `Client.exe`, `TTS.Common.dll`,
    `Client.exe.config`, Janus GridEX, NAudio, TeamSpeak and the
    framework DLLs.
  - `dnfile` read the CLR metadata: types, members, user strings and
    managed resources.
  - `dncil` disassembled every method body to CIL with resolved tokens.
  - Nothing was run.
- **Not used:** the owner named `morluto/rea`. Its .NET support is the
  same static metadata and CIL reading, but this session's permission
  policy blocked running it through `npx`.
- **Output location:** the derived dumps (types, strings, CIL, resource
  indexes) and the two scripts stay outside Git, in
  `~/.cache/bunting-skills-src/rit/analysis-1.8.466/`. The installer
  corpus rule in `docs/reference-adoption.md` applies: no payload,
  resource, decompiled body or proprietary text enters the repository.
  Below, RIT's help texts are paraphrased.

Labels: **observed** (in metadata, a string literal or CIL), **inferred**,
**Bunting-added**, **unresolved**.

## Architecture (observed)

- **Client stack:** a WinForms client (`TTS.Client`) on Janus GridEX
  grids. Each window is a `TTSForm`, and windows dock in workspaces.
- **Server connection:** one WCF duplex service at
  `net.tcp://<address>:<port>/TTSClientService`. The default port is
  10000 (`Client.exe.config` `ServerPort`).
- **Client to server calls** (`IClientService`):
  - session: `Ping`, `Authenticate`, `NewUser`, `SyncReceived`;
  - orders: `AddOrder`/`AddOrder2`, `AddSpreadOrder`,
    `AddTransportArbOrder`, `CancelOrder`/`CancelOrder2`,
    `CancelAllLimitOrders`/`CancelAllLimitOrders2`,
    `CancelTradeAtSettleOrder`;
  - OTC: `AddOTCOrder` (plus a price-expression variant),
    `InitiateBreakOTCOrder`, `UpdateOTCOrder`;
  - assets: `LeaseAsset`, `UnleaseAsset`, `UseLeasedAsset`, `UseAsset`;
  - data: `GetReportChunk`, `GetCustomCaseFile`, `GetAdHocCaseFile`,
    `ReadNews`, `GetTimeAndSalesData`;
  - other: `BidOnTender`, `ChatMessage`, `ComplianceResponse`, and the
    voice-call calls.
- **Server to client calls** (`IClientCallbackService`):
  - session: `Sync` (the full state download), `OrderedUpdate`
    (sequenced deltas), `Reset`, `Heartbeat`, `UpdateGameStatus`,
    `UpdateVariable`;
  - data: `UpdateNews`, `TenderOffer`, `CloseTenderOffer`,
    `AddOrderHistory`, `UpdateAssetLeaseCount`, `AddAdHocFile`;
  - messages: `SendMessage` (server messages), `SendNewTraderMessage`,
    `ComplianceMessage`, `ToggleReportDownload`;
  - people: connected traders and assessors, chat, and voice-call state.
- **Local API:** a REST server (`TTS.Managers.RestServer`) on localhost.
  The default port is 9999 and it is protected by an API key. Its routes
  are:
  - `/v1/case`, `/v1/trader`, `/v1/trader/history`, `/v1/limits`;
  - `/v1/news`, `/v1/assets`, `/v1/assets/history`;
  - `/v1/securities`, `/v1/securities/book`,
    `/v1/securities/history`, `/v1/securities/tas`;
  - `/v1/orders`, `/v1/orders/{id}`, `/v1/commands/cancel`;
  - `/v1/tenders`, `/v1/tenders/{id}`;
  - `/v1/leases`, `/v1/leases/{id}`, `/v1/transactions`.

  This matches the DMA swagger in `rit-references`. Bunting covers it
  with FIX and BNP, never REST (ADR 0031).
- **Excel RTD:** a field catalogue that the RTD Info window lists. Its
  groups are General, Asset, Security, Tender, Historical, Risk and
  News, with fields such as `TICKER|COST`, `PLUNR`, `PLREL`, `VWAP`,
  `BID|N`, `AGBID|N`, `MKTBUY|N`, `LASTHIST|N`, `GROSSLIMIT` and
  `NETFINE`.

## Security reference data (observed, REST formatter)

Each security carries the following fields. The desk needs them over BNP
before it can show RIT's columns and price format (ADR 0054, Proposed):

- ticker, type, size (contract), position, vwap (cost), nlv, last;
- bid and bid size, ask and ask size, volume, total volume;
- unrealized, realized, currency, limits, interest rate;
- is_tradeable, is_shortable, start and stop period, description;
- unit multiplier, display unit, start price, min and max price;
- **quoted_decimals**, trading fee, limit-order rebate;
- min and max trade size, required and underlying tickers;
- bond coupon, interest payments per period, base security;
- fixing ticker, API orders per second;
- **execution_delay_ms**, interest-rate ticker, OTC price range.

`quoted_decimals` is how RIT shows a 0.01 grid as dollars and cents.
Bunting's integer price ticks have no display scale yet. That is why the
practice market runs on a 1%-of-price tick (backlog, 2026-10-22).

## Main window (observed)

- **Login panel:** described under Onboarding below.
- **Menu: workspaces.**
  - New Workspace (asks for a unique name), Save, Load (a `.wkspc` file)
    and Delete. "Default" cannot be deleted.
  - Close All and Close All Order Entries, each with a confirmation.
  - Tile horizontally, Tile vertically, Cascade.
  - Shift-click on a menu item opens another copy of an open window.
- **Menu: windows.**
  - Market View (Portfolio), Trader Info, Transaction Log, Trade
    Blotter, Assets, News.
  - Ladder Trader, Book Trader, Order Entry (Buy/Sell Entry), Spread
    Entry, Transportation Arbitrage Entry, OTC Trade.
  - Security Charting, NLV (P&L) Charting, Time & Sales, Electricity
    Charting.
  - Case Files, Help Files, Chat, Voice Chat, Kill All, RTD Info,
    API Info.
  - Assessor-only: Traders & Positions, News and Voice-chat monitors,
    and Security Charting Assessor.
  - A case can restrict which windows are allowed
    (`EnforceAllowedWindows`).
- **Status bar:** the case date, period, tick, ticks per period with a
  tick progress bar, connection state, the API server label, a sounds
  toggle (speaker and muted icons) and a settings button.
- **Client settings:**
  - colour scheme: Default, High Contrast, Protanopia or Dark;
  - grid font size and line height, as sliders;
  - persisted settings: `ServerAddress`, `ServerPort`, `Workspace`,
    `SoundsEnabled`, `IsSilentAlerts`, `IsOTCAutoAccept`,
    `ChartExpressions`, `APIKey`, `APIPort`, `NewsFontSize`,
    `NewsLineHeight` and `ColorScheme`.
- **Toasts:**
  - a title, a body and an icon (info, error, telephone or cross);
  - clickable, they blink while highlighted and close on a timer;
  - at most 5 are open, and the oldest closes first;
  - `ShowToastInfo` and `ShowToastError` take a "play sound" flag.

## Windows (observed members; behaviour inferred from the names)

- **Portfolio (Market View):**
  - one grid, rows draggable onto other windows (drag a ticker into
    Book/Ladder Trader or a chart);
  - "Copy RTD Table/Field To Clipboard";
  - columns (from the `Game` tables and headers) include Ticker, Type,
    Position, Pending Position, **Cost (the position's VWAP)**, Contract
    Size (unit multiplier), Realized P&L, Unrealized P&L, Bid, Ask, Bid
    Size, Ask Size, Last, NLV, Volume, My Volume and the security's
    VWAP.
- **Book Trader:**
  - ticker selector; a position, VWAP and last header;
  - bid and ask grids;
  - an attached order entry: V (volume), O (offset), price, LMT/MKT,
    Submit Buy, Submit Sell, Cancel All;
  - a shortcut-trades toggle;
  - shortcut gestures (paraphrased help):
    - left-click a level: a limit order at that price, improved by
      offset × the minimum increment, for V lots;
    - right-click: a market order for V lots (ask side buys, bid side
      sells);
    - Alt-left-click: cancel that order;
    - Shift-right-click: sweep with a limit at the clicked price, for
      all the size up to and including it.
- **Ladder Trader:**
  - ticker selector; a position, VWAP and last header;
  - one price ladder with Bid Qty, Bid Size, Ask Size and Ask Qty
    columns;
  - a step size from the minimum increment, a settings button and
    rows on screen;
  - recentring on a timer with a progress bar (recenter limit and
    seconds);
  - gestures (paraphrased):
    - left-click a Size cell: a limit order at that price for V lots;
    - left-click a Qty cell: cancel all own orders at that price;
    - Space: recentre on mid now.
- **Order Entry (Buy/Sell Entry):**
  - panels can be added and removed;
  - each panel has ticker, Buy/Sell toggle, LMT/MKT toggle, price,
    volume, Submit, and a TAS (trade at settle) toggle with helper
    text.
- **Spread Entry:** add and remove panels. Each panel has a buy ticker,
  a sell ticker, their volumes (kept in step), a switch-legs control
  and Submit.
- **Trade Blotter:** tabs for Live/Partial (with Cancel All),
  Filled/Cancelled, Tenders, Endowments and Trade At Settle (shown only
  when the case has it).
- **Transaction Log:** one grid. Its columns are ID, period, tick,
  timestamp, ticker, price, quantity, currency, type, value and
  balance (the cash-flow journal).
- **Trader Info:** trader ID, name, NLV, fines, and a trading-limits
  grid (gross and net limit per risk category, with fines).
- **Time & Sales:**
  - ticker selector;
  - filters by ticker and by minimum volume (validated);
  - "scroll to top" when new prints arrive;
  - a menu option to show anonymous-to-anonymous trades;
  - columns: ID, period, tick, ticker, price, volume, buyer/seller and
    price × volume.
- **Security Charting:**
  - a ticker or expression box, with history. Tickers go in brackets,
    and `+ - * / ^` and constants are allowed, e.g. a spread between
    two tickers;
  - a second expression on an optional Y2 axis;
  - period selector, grouping (bar size, plus custom);
  - overlays: volume, MA, EMA, RSI, with "clear all";
  - zoom on/off and zoom out, scroll;
  - drawing annotations in a colour, with undo;
  - a line/candle toggle;
  - copy the image to the clipboard, save as PNG;
  - drag and drop a ticker onto it.
- **NLV (P&L) Charting:** NLV by period and tick, with zoom, copy or
  save the image, and copy or save the data (CSV with Period, Tick and
  NLV).
- **News:** a grid (period, tick, ticker, headline) and a reading pane
  (time, ticker, headline, HTML body). Rows are unread or read, and the
  rows can be copied as TSV. The news font size and line height are
  settings.
- **Tender Order:** a dialog per tender offer, with these parts:
  - a caption, ticker, quantity, action and price;
  - a bid field for tenders that are not fixed-bid;
  - Submit Tender, and Decline with a live countdown to expiry;
  - an automatic expiring bid.
- **OTC Trade:**
  - fields: counterparty trader, ticker, buy/sell, volume;
  - price as a fixed value or an expression;
  - settlement immediate or at a period and tick;
  - an Auto-accept checkbox, persisted;
  - Pending and Completed tabs.
- **Assets:** tabs for Transporters, Producers, Converters and a Log.
  Lease, use and unlease go through a usage dialog.
- **Kill All:** one button.
- **Case Files and Help Files:** downloadable case documents (standard,
  custom and ad hoc), with a download progress bar. A newly added ad hoc
  file raises a "New Case File" toast.
- **Report download:** the server can toggle a trader-report download,
  with Download, Downloading…, Download Again and Open.
- **Chat, Voice Chat:** text and TeamSpeak voice. These are out of
  Bunting's scope until a case needs them (*unresolved*).

## Alerts and sounds (observed)

RIT 1.8.466 plays sound in exactly these places. Everything else is
silent.

| Trigger | Sound | Condition |
|---|---|---|
| A news item arrives (`StateManager.UpdateNews`) | Windows "Exclamation" system sound | Sounds enabled |
| A toast raised with its sound flag on | Windows "Exclamation" | Sounds enabled |
| A new ad hoc case file (toast "New Case File") | Exclamation (the toast's sound flag is on) | Sounds enabled |
| A server message while "silent alerts" is on (shown as a toast instead of a dialog) | Exclamation (the toast's sound flag is on) | Sounds enabled |
| Trader-report download toggled | Exclamation | Sounds enabled |
| Voice call connected | embedded `connected` WAV | voice |
| Voice call disconnected or rejected | embedded `disconnected` WAV | voice |
| Incoming call | embedded `incoming_call` WAV, looped until answered | voice |
| Call waiting | a 440 Hz tone for 0.6 s | voice |

Further observations:

- An `announcement` WAV is embedded, but no code path names it. It is
  probably unused in this build (*inferred*).
- **There is no sound on an order fill, an order acceptance, a reject
  or a tender offer** in 1.8.466. A tender opens its dialog without a
  sound.
- A trading-limit breach ("Gross/Net limits in risk category N
  exceeded") and an accepted OTC order raise toasts.
- The sounds toggle in the status bar flips `SoundsEnabled` and saves
  it.
- The four WAVs are Rotman's assets (prohibited to copy). The
  Exclamation sound is the operating system's.

**Bunting-added for the desk:** the owner asked for "sounds for order
filled, etc". The desk plays original synthesized tones (no RIT or OS
assets) at RIT's trigger points: news, a toast-worthy notice and a
tender offer. It adds a fill tone, a reject tone and a connection-lost
tone, behind the same one-click mute that RIT's status bar has.

## Onboarding (observed)

1. **Login panel** (titled as RIT's login):
   - fields: Trader ID, Password, server address, port (10000 by
     default);
   - the address and port are remembered between launches;
   - Enter submits.
   - Submitting saves the settings, disables the form and shows a
     loading spinner with a status line. The password is hashed (MD5)
     before it is sent.
2. **Connect:**
   - a malformed address gives "invalid address format";
   - an unreachable server gives "unable to connect to address:port";
   - both are error dialogs, and the form re-enables.
3. **Authenticate:** if the trader ID is unknown, the server sends
   `SendNewTraderMessage`. The client asks whether to create a new
   account. Yes switches to the New User panel.
4. **New User panel:**
   - fields: first name, last name, trader ID, password, confirm
     password, address and port;
   - inline validation: every field is required, and the two passwords
     must match;
   - submitting calls `NewUser` and logs in.
5. **Sync:**
   - the status line moves through "Syncing..." (the server's full
     state) and "UI...";
   - queued updates that arrived during the sync are applied in order;
   - active tenders are restored.
6. **Main window:**
   - the saved workspace for the case is restored;
   - only the case's allowed windows open;
   - the status bar binds to the date, period and tick;
   - new case files raise a toast.
7. **Disconnect:** returns to the login panel. Logout is on the menu.

**Bunting mapping (Bunting-added):**

- BNP identity is a registered certificate, never a password (ADR 0031,
  ADR 0040). So the desk's login asks for:
  - a venue address and port;
  - the participant certificate and key files, plus the CA;
  - or "Practice locally" (the shared local venue, ADR 0050 practice
    mode).
- It remembers the last values and shows the same staged status
  (connecting, authenticating, syncing, loading workspace).
- It returns to the login screen on disconnect.
- RIT's "new account" has no BNP equivalent: there is no enrolment
  message. Creating a key and certificate request for an instructor to
  register needs an operator-side ADR (*unresolved*).

## Case-specific modules are off unless the case turns them on

- **Observed:** RIT gates windows per case. `EnforceAllowedWindows`
  closes and hides the windows the running case does not allow, and the
  server's case data says which those are. It runs again when the case
  changes.
- **Owner, 2026-10-22:** "some features you decide from feature-parity
  are only used in certain cases not always on".
- **The desk shows these only when the scenario enables them:**
  - Tender Order dialogs and the Tenders tab: tender cases (LT3);
  - Endowments and Trade At Settle tabs, and the TAS toggle on Order
    Entry: cases with endowments or settlement trades;
  - Spread Entry: spread and futures cases;
  - OTC Trade: OTC cases;
  - Assets, Transportation Arbitrage Entry and Electricity Charting:
    commodity and electricity cases;
  - Chat and Voice Chat: cases that turn them on;
  - Case Files and the trader report: whenever the case supplies them.
- **Always on:**
  - Portfolio, Book Trader, Ladder Trader, Order Entry;
  - Trade Blotter's live and filled tabs, Transaction Log, Trader Info;
  - Time & Sales, Security Charting, P&L Charting, News, Kill All.
- **Bunting-added:** a scenario declares its enabled modules, and the
  venue sends that list to the desk. Until BNP carries case information
  (ADR 0054 "case information"), the desk uses the local practice
  scenario's list. That list has none of the case-specific modules.

## Gaps this inventory adds to the parity contract

These are new against `rit-desk-window-parity.md`:

- Shift-click opens another copy of a window.
- Tile and cascade.
- Workspace save, load and delete, with "Default" protected.
- Colour schemes: High Contrast and Protanopia.
- Font-size and line-height sliders.
- Toasts: at most 5, blinking, with a sound flag.
- Book Trader offset (O:) and Shift-right-click sweep.
- Ladder recentring (timer, Space) and Qty-column cancel-all.
- Spread Entry.
- Time & Sales volume filter and anonymous toggle.
- Chart expressions (spreads), a Y2 axis, annotations, and image and
  CSV export.
- NLV CSV export.
- Trade At Settle and Endowments blotter tabs.
- The tender countdown.
- OTC expression pricing and settlement timing.
- The login, new-user, sync and workspace onboarding.
- The sound triggers above.
