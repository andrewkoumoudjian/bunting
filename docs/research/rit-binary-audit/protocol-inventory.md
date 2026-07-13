# Protocol inventory

## Local service topology

Static metadata proves two local WCF named-pipe contracts:

- API: `net.pipe://localhost/306W/TTS/API`, with a legacy fallback string `net.pipe://localhost/Rotman/RIT/API`;
- RTD: `net.pipe://localhost/306W/TTS/RTD`, with a legacy fallback string `net.pipe://localhost/Rotman/RIT/RTD`.

`RIT2.dll` exposes the automation-facing `IAPI`/`API` facade. `TTS.RTD.dll` implements Excel `IRtdServer` and registers as `RIT2.RTD`. The User Application also hosts a localhost HTTP REST service, default port 9999, authenticated with an API key. The simulation connection defaults to port 10000. Exact server-side transport, framing, and authentication beyond the observed client contracts remain unresolved.

## REST surface

The static `RestServer` constructor registers these exact method/path pairs:

| Method | Path | Observed handler purpose |
|---|---|---|
| GET | `/v1/case` | case/run state |
| GET | `/v1/trader` | private trader/account state |
| GET | `/v1/limits` | trading/risk limits |
| GET | `/v1/news` | news history |
| GET | `/v1/securities` | security and private position data |
| GET | `/v1/securities/book` | book depth |
| GET | `/v1/securities/tas` | time and sales |
| GET | `/v1/securities/history` | historical bars/ticks |
| GET | `/v1/assets` | asset definitions/state |
| GET | `/v1/assets/history` | asset history/log |
| GET/POST | `/v1/leases` | list and create/use leases |
| GET/DELETE/POST | `/v1/leases/{id}` | query, unlease, or use a lease |
| GET/POST | `/v1/orders` | list or submit orders |
| GET/DELETE | `/v1/orders/{id}` | query or cancel an order |
| GET | `/v1/tenders` | active tenders |
| POST/DELETE | `/v1/tenders/{id}` | accept/bid or decline a tender |
| POST | `/v1/commands/cancel` | bulk cancellation |

Observed JSON field names include case name, status, period, tick, ticks per period, total periods, enforcement flag; trader ID/name/NLV; gross/net limits and fines; security L1, position, P&L, fees, limits, size/price bounds, dependencies and execution delay; book bids/asks; order/trade/history/news/tender/lease fields; and structured errors containing `code`, `message`, and sometimes `wait`. Market-order dry runs are explicitly named. Exact response status codes, validation ordering, rate-limit algorithm, and all optional/null rules remain unresolved.

## VBA/API facade

The API supports open/close; time remaining, total time, year time, current period, cash, buying power, NLV; ticker lists and information; order lists/details; market/limit submission; cancellation by ID or expression; queued orders; and tender list/detail/accept/decline. Direction constants are `BUY`/`SELL` and type constants are `MKT`/`LMT`.

## RTD subscription contract

The Excel syntax is `=RTD("RIT2.RTD",,"[Field1]","[Field2]","[Field3]")`. The companion server implements connect, disconnect, refresh, heartbeat, update notification, and topic counting. The client-side RTD data object returns a 500 ms refresh interval; the COM server also has connected, disconnected, and maximum refresh interval state. Excel's `RTDThrottleInterval` registry setting is exposed to the user.

Observed general topics: `TRADERID`, `PL`, `TRADERNAME`, `TIMEREMAINING`, `PERIOD`, `PERIODTIME`, `YEARTIME`, `TIMESPEED`, `ALLASSETTICKERS`, `ALLASSETTICKERINFO`, `ALLTICKERS`, and `ALLTICKERINFO`.

Observed ticker topics:

- L1 and activity: `LAST`, `BID`, `ASK`, `VOLUME`, `POSITION`, `VWAP`;
- private valuation: `COST`, `PLUNR`, `PLREL`;
- personal orders: `OPENORDERS`, `ALLORDERS`, `LIMITORDERS`;
- raw depth: `BID|N`, `BSZ|N`, `ASK|N`, `ASZ|N`, `BIDBOOK`, `ASKBOOK`;
- aggregated depth: `AGBID|N`, `AGBSZ|N`, `AGASK|N`, `AGASZ|N`;
- impact/VWAP: `MKTBUY|N`, `MKTSELL|N`;
- history: `LASTHIST|N`, with positive absolute tick and non-positive ticks-ago semantics;
- risk: `GROSS`, `NET`, `GROSSLIMIT`, `NETLIMIT`, `GROSSFINE`, `NETFINE`;
- metadata: `INFO|FIELD` and `INTERESTRATE`.

Tender topic `TENDERINFO|N` returns ID, ticker, quantity, price, received tick, and expiry tick. News topics are `NEWS|N` from oldest to newest and `LATESTNEWS|N` from newest to oldest.

The 37 observed `INFO` fields are: `Ticker`, `Description`, `Type`, `UnitMultiplier`, `DisplayUnit`, `IsTradeable`, `IsFollowPath`, `StartPeriod`, `StopPeriod`, `StartPrice`, `MinPrice`, `MaxPrice`, `QuotedDecimals`, `IsShortAllowed`, `TradingFee`, `LimitOrderRebate`, `TradingFeeType`, `MinTradeSize`, `MaxTradeSize`, `Currency`, `RequiredTicker`, `UnderlyingTickers`, `BondCoupon`, `InterestRate`, `InterestPaymentsPerPeriod`, `BaseSecurity`, `FixingTicker`, `FixingTicks`, `APIOrdersPerSecond`, `ExecutionDelayMilliseconds`, `InterestRateTicker`, `OTCPriceRange`, `SpreadPrimaryTicker`, `SpreadPrimaryQuantity`, `SpreadSecondaryTicker`, `SpreadSecondaryQuantity`, and `RiskTypes`.

RTD is an adapter concern, but every value above originates in market, scenario, participant, ledger, or risk state and therefore requires a typed `bunting-engine` query surface when the unified architecture is implemented.
