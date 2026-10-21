# Market primitives needed to run the RIT cases

Status: **Target** (2026-10-20). Owner direction (2026-10-20): "make sure
that our app will have ALL of the proper primitives so that we can run all
of those cases simulations." This document lists every primitive the
published RIT cases need. For each one it says whether Bunting has it,
has it in part, or lacks it, and where it would live. It is the engine-side
companion to `rit-desk-window-parity.md` (the windows).

## Sources (observed 2026-10-20; copyright Rotman FRT-Lab, paraphrased, nothing copied)

- **Case material:** `RotmanFRTL/RotmanFRTL.github.io` at `eb978c0` (cloned
  read-only to the coordinator's reference cache; not vendored, because
  the material is all-rights-reserved):
  - the RITC 2026 Case Package, covering Social Outcry, Liquidity Risk
    (LT3), Volatility Trading (OP3), GBE Electricity (COM5), Merger
    Arbitrage (MA1), Algorithmic Market Making (ALGO2e) and scoring;
  - the case briefs for ALGO2, ALGO2e, ALGO3 (smart order routing), LT3,
    Event-driven News, and the RITCx ETF Arbitrage, Stat Arb, Volatility
    and Liability Trading cases (Stat Arb, Event-driven and Liability read
    for their primitives, not line by line);
  - the base scripts (REST and DMA API).
- **API:** *RIT DMA REST API 1.0.5* (`https://rit.306w.ca/RIT-DMA-API/1.0.5/swagger.json`).
  It is the RIT server's own resource model and so the most complete
  list of RIT primitives:
  - case, trader, trader history (NLV), transactions, limits, news;
  - assets and asset history, leases;
  - securities with book, OHLC history and time-and-sales;
  - orders, tenders and bulk cancel.
- **Bunting:** the code at `main@6923320`, in `bunting-engine` (`lib.rs`,
  `simulation.rs`), `ledger`, `risk-engine`, `bunting-agents` and
  `apps/bunting-server`.

**Status words:**

- **Have:** built and reachable in a run.
- **Partial:** a model exists, but part of the behaviour is missing or
  refused (`SimulationError::Unsupported`).
- **Missing:** no model.

## 1. Instruments

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| Stock / equity | All cases | `InstrumentKind::Equity` | Have |
| Currency, FX conversion of P&L | ETF Arb: CAD base; USD ETF; CAD and USD tradable | `InstrumentKind::Currency`; the ledger has FX | Partial: verify that USD-quoted instruments value into a CAD base, and that currencies trade as instruments |
| ETF and index | ETF Arb (RITC), Volatility (RTM), Social Outcry (RT100 index) | `Synthetic { components }` | Partial: index/ETF exists as a type. Creation and redemption are under Converters below. |
| European option, cash-settled at expiry on the last underlying price | Volatility: 20 RTM options, 100 shares each | `Option { underlying, strike, expiry, is_call }` | **Partial:** `ExerciseOption` is refused as Unsupported. Needs cash settlement `max(0, S−K)` or `max(0, K−S)` × multiplier at expiry. |
| Future, cash-settled at the closing spot | Social Outcry: RT100 futures, multiplier 10 | `Future { underlying, expiry, physical_delivery }` | **Partial:** physical `Deliver` is refused. Needs cash settlement against the closing price of the underlying (spot index). |
| Forward with physical delivery the next day | Electricity: ELEC-F (500 MWh) delivers into ELEC-dayX the next day | `Future { physical_delivery: true }` | **Missing:** delivery semantics, which convert a forward position into a spot position at the start of the delivery day |
| Daily spot commodity with a trading window (start/stop period) | ELEC-day2…day6, each tradable on its own day only; NG | `Commodity`; tradable windows | **Missing:** per-instrument start and stop tradable times (RIT `start_period`/`stop_period`, `is_tradeable`) |
| Bond with coupon and interest; rate instruments | RIT API types BOND, RATE, SWAP and SWAP_BOM, with coupon, interest and BOM fixing transactions | `Bond { coupon_bps, maturity }`; `Cashflow` kinds Coupon and Interest | Partial: no published case brief needs them. Keep them reachable; their accrual policy is unresolved. |
| Contract multiplier (contract size) | Options ×100, RT100 ×10, ELEC-F 500 MWh, ELEC-day 100 MWh | instrument multiplier | Have, in the engine and on Listings: **verify P&L and fees per contract** |
| Shortable flag | Electricity (NG not shortable) | `shortable` | Have |
| **Non-tradeable reference instrument** (an index that is quoted and charted but cannot be ordered) | Stat Arb (RSM1000 benchmark); RIT `is_tradeable` | — | **Missing:** a tradeable flag per instrument. The engine refuses orders and BNP still publishes prices. |
| Quoted decimals, minimum and maximum price, integer-only quotes | ELEC-F: integer quotes only | `price_bounds`, ticks | Partial: the tick size per instrument must reach BNP. "Integer only" is a tick of 1.00. |
| Display units and a description | RIT API `display_unit`, `description` | — | Missing on BNP (reference-data gap, desk parity spec) |

## 2. Trading rules and costs

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| Limit and market orders; a marketable limit acts as market | All | `OrderKind` | Have |
| Maximum order (trade) size per instrument | All: 10,000 shares, 100 option contracts, 5 futures, 10/80 ELEC-F/NG; Liability: CRZY 25,000, TAME 10,000 | `RiskLimits.max_order_quantity`, per participant | **Partial:** needs to be **per instrument** (or a security type), not one value per participant |
| Commission per share or contract, by instrument | LT3 $0.01–$0.04; options $1/contract; ALGO $0.02 taker | `FeeSchedule { taker_per_lot, maker_per_lot }` | Partial: verify the schedule is per listing and in money units per lot |
| Limit-order rebate, maker vs taker, by venue | ALGO2e rebates $0.01–$0.025; ALGO3 main/alt venue take fees and an alt-venue rebate; ETF Arb | `maker_per_lot` negative = rebate | Have (per listing). Verify per venue. |
| No commission on tenders | LT3 | tender settlement | Have (verify) |
| Gross and net limits across all securities, or per security type, with a limit multiplier per instrument | LT3 250k/150k; Volatility by type (ETF 50k, options 2,500/1,000 contracts); Merger 100k/50k; ETF Arb counts the ETF ×2 | `max_absolute_position` per instrument only | **Missing:** portfolio gross and net limits over **limit groups**, with a per-instrument weight, enforced at admission (RIT `limits[]`) |
| Limit breach as a **fine** instead of a refusal | ALGO2e: $5/share above the limits; ALGO2: $0.10/share above a 25,000 position; RIT GROSS/NET_LIMIT_FINE | refusal only | **Missing:** a per-case choice between refuse and fine, and a fine schedule |
| API order rate limit per second; execution delay | RIT `api_orders_per_second`, `execution_delay_ms`; DMA 429 | BNP/FIX rate limits per session; latency per ADR 0035 | Have (Bunting's own model). Map the case's rate onto the session limits. |
| Dry-run order check | RIT `dry_run` | — | Missing: optional. A BNP pre-trade check would be Bunting-added. |
| Interfaces per case: manual on/off, API on/off | ALGO cases ban manual trading; Electricity disables API orders | — | **Missing:** a per-run permission per participant and role (BNP manual or app vs algo session). FIX vs BNP alone is not the same thing. |
| Several venues for one security, with different fees | ALGO3 (main and alternative exchange) | per-venue listings, fees and latency | Have |
| Bulk cancel by ticker or ids | API `commands/cancel` | cancel-all; kill switch | Partial: by ticker and by ids |

## 3. Tenders and institutional orders

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| Private tender: fixed price and quantity, accept or decline before expiry | LT3, ALGO3, ETF Arb, Liability | `TenderState`, BNP tenders (ADR 0043) | Have |
| **Competitive auction**: sent to all; anyone bidding better than a hidden reserve is filled at their own price | Liquidity Risk | — | **Missing** |
| **Winner-take-all**: sent to all; the single best bid or offer wins if it beats the hidden reserve | Liquidity Risk | — | **Missing** |
| A declined tender passes to another random trader | LT3 brief | — | Missing (an agent or policy) |
| Tender generator: random times, random recipients, equal count per participant, price near the market | Liquidity Risk | scheduled actions / agents | Partial: needs a fair tender-schedule generator |
| Factory or institutional tender reports through news | Electricity Traders | news | Have |

## 4. News and information

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| Public news on a schedule, with headline, body and ticker | All | `NewsItem`, BNP news | Have. Verify ticker linkage. |
| **Private news per role or participant** | Electricity (Producers get sunshine; Distributors get temperature); Volatility analyst forecasts; delta limit announced by news | `NewsAudience` | Partial: verify role and team audiences on BNP |
| News that drives the price process (fundamental shocks; probability updates in merger arb) | Merger Arb, Event-driven, Social Outcry, ALGO2e | agents | **Partial:** agents must follow a scripted **fundamental value path** with news-linked jumps (see 7) |

## 5. Assets, leases and conversions

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| **Converter with several inputs and outputs and a fee per use** | ETF Arb: ETF creation (10k BULL + 10k BEAR → 10k RITC, $1,500 USD per use) and redemption | `FacilityKind::Conversion`, one input and one output instrument | **Partial:** several legs, ratios, a fee per use, and human-only (refused for API sessions) |
| Lease an asset for N ticks at a price; use it to convert inputs to outputs over a conversion period | Electricity: NG power plant, 80 NG → 10 ELEC, delivered the next day; RIT `/leases` with three input legs; COM cases (containers, pipelines, ships, refineries) | `FacilityKind` (Asset, Lease, Transport, Storage, Production, Conversion), `FacilityJob`, `CompleteFacilityJob` | Partial: verify the lease lifecycle (lease, use, unlease), capacity, conversion delay, and that asset history is published |
| Endowment: units of an instrument granted on a schedule | Electricity: solar output 6 × sunshine hours, given to Producers each day | scheduled actions | **Missing:** a scheduled position grant (RIT ENDOWMENT transaction) |
| Disposal of perishable inventory at day end, with a fine | Electricity: undelivered ELEC-dayX disposed at $0, plus $20,000 per contract; a short is also penalized | — | **Missing:** per-instrument end-of-window close-out at a fixed price, with a fine per unit long or short |

## 6. Close-out, settlement and fines

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| Close-out of every position at the end of a sub-heat at the **last traded price** | LT3, Merger, ALGO2e, Volatility (ETF) | official close, `DaySettled` marks | Partial: needs an explicit close-out that **trades** the position to flat at the close mark (RIT CLOSE_OUT transaction) and resets for the next sub-heat |
| Close-out at a **fixed price** (distressed $0) or an **announced price** (RAE price on day 6) | Electricity | — | **Missing:** a close-out price source per instrument (last, fixed, announced, settlement formula) |
| Option cash settlement at expiry | Volatility | — | **Missing** (see 1) |
| Futures settle at the closing **spot** price | Social Outcry | — | **Missing** (see 1) |
| Mark-to-market in P&L | All (RIT MARK_TO_MARKET) | ledger marks | Have |
| **Case fines engine**, with per-case rules evaluated on the run clock, posted as ledger entries (CASE_FINE) and shown in real-time P&L | (see the next four rows) | — | **Missing:** a fine-rule framework |
| – Speculation and front-running fines | Liquidity Risk / LT3 / ALGO3: $1/share up to 5,000, then $2; $10/share uncovered at the end; classification against open tender windows | — | Missing |
| – Delta-limit penalty | Volatility: per second, (\|portfolio delta\| − limit) × rate. Needs portfolio delta from option deltas (Black-Scholes). | — | Missing |
| – Aggregate position limit at each market close | ALGO2e: Σ\|position\| over four stocks, $10/share above, assessed each minute | — | Missing |
| – Excess and unmet-demand penalties | Electricity Distributors: unmet customer demand $20,000/contract per day | — | Missing |
| Distressed lease and cover fines | RIT transaction types DISTRESSED_* | — | Missing (COM cases) |
| Dividends, coupons, interest | RIT transaction types | `CashflowKind` | Have (scheduled cashflows) |
| Mark against a scripted "correct price" on divergence | ETF Arb close-out | — | Missing: a close-out price may come from the scenario's fundamental path |

## 7. Price processes and computer traders

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| Random-walk price path (normal returns, annualized volatility) that drives the quotes | LT3, Volatility (σ shifts weekly), Social Outcry (spot) | `bunting-runtime` `FundamentalPath::RandomWalk` (seeded, integer fixed-point, a pure function of the run clock, scheduled volatility regimes), observed by every agent as `fundamental`; market makers centre quotes on it | **Have** for the basic form (slice entry `2026-10-20-fundamental-walk.md`). Still missing: news-linked jumps and a published fundamental for close-out marks |
| Option market maker quoting Black-Scholes at a **lagged** volatility estimate, 2-cent spread, unlimited size | Volatility | — | Missing (agent kind) |
| Quote liquidity and volatility per security (low, medium, high) | LT3 tables | agent parameters | Partial |
| Extra closing liquidity, so the close cannot be manipulated | LT3 | — | Missing (an agent schedule) |
| Regulated spot market: an authority sets the price daily from supply and demand, with limited volume (100 buy / 100 sell) and a 1-cent spread | Electricity RAE | — | Missing: an "authority" quoting agent with a fixed price and capacity |
| Merger dynamics: target value = p × deal value + (1 − p) × standalone value; p moves with news (direction × severity × category × deal multipliers); stock-for-stock links the target to the acquirer | Merger Arb | — | Missing: a fundamental model plugin |
| ETF parity: P(ETF, USD) × FX = Σ components, with shocks | ETF Arb | — | Missing: a fundamental relation for the agents |
| **Correlated multi-asset paths**: stocks with set correlations and betas to an index, plus injected divergences that revert | Stat Arb (correlations such as 0.51 to the index; betas 0.48 and 0.57; divergence to +8% then convergence) | — | Missing: a multivariate path generator (Cholesky on a correlation matrix) with scheduled divergence episodes |
| **News with a delayed, sector-weighted price impact** | Event-driven News (broad vs industry news; reaction 20–30 s after the headline; sectors react differently) | — | Missing: a news-to-fundamental mapping with lag and per-instrument sensitivity |

## 8. Case structure, roles and scoring

| Primitive | RIT evidence | Bunting | Status |
|---|---|---|---|
| A case of N independent sub-heats (periods), each M seconds of trading representing K calendar days, with a tick clock | All (for example 5 × 600 s = 1 month) | run clock, calendar, multi-day paced runs, iterations | Partial: needs **period and tick** reporting (RIT `period`, `tick`, `ticks_per_period`, `total_periods`) and reset of positions and P&L between sub-heats |
| Case status: active, paused, stopped | RIT CaseStatus | `RunLifecycle` | Have |
| **Roles** with instrument access and asset access per role (Producer / Distributor / Trader) | Electricity | participant role (operator / participant) | **Missing:** case roles that gate instruments, assets and news audience |
| **Teams** of traders whose P&L is summed | All RITC scoring; Electricity (4 per team) | teams in competition policies | Partial: verify team aggregation in scoring |
| Ranking: team P&L per sub-heat → rank → average rank → case rank → points (n+1 scheme); a weighted final score with a variance tie-break | RITC scoring | `competition-policies-v1` | Partial: implement and verify this policy as one selectable scoring policy |
| Trader NLV history | API `/trader/history` | `DaySettled` NLV | Partial: needs an intraday NLV series (desk parity spec, P&L chart) |
| Transaction log with typed entries | API `/transactions` (26 types) | ledger postings | Partial: needs a typed ledger journal on BNP (desk Transaction Log) |
| Performance report at the end of an iteration (fines summary, orders summary, P&L chart with and without fines) | ALGO3 notes | `IterationReport` | Partial |

## 9. What this means

Bunting's core is already right for most cases: one book per listing,
venues with fees and latency, tenders, OTC, news, facilities, cashflows,
the ledger with FX and marks, and multi-day runs. The missing pieces form
six coherent engine features. Each needs an ADR, then a slice:

1. **Settlement and close-out:**
   - option cash settlement at expiry;
   - futures cash-settled at the spot close;
   - forward-to-spot delivery;
   - per-instrument tradable windows;
   - close-out price sources (last trade, fixed, announced, formula),
     with fines on leftover positions (perishables).
2. **Portfolio limits:** gross and net over limit groups with
   per-instrument weights, a per-instrument maximum order size, and
   refuse-or-fine per case.
3. **A case fines framework:** run-clock rules that post `CASE_FINE`
   entries. The first rules are:
   - speculation and front-running relative to tender windows;
   - the uncovered position at the end;
   - the delta limit (needs option greeks);
   - the aggregate position limit at market close;
   - unmet demand.
4. **Tender kinds:** competitive auction and winner-take-all, both with a
   hidden reserve; reassignment of declined tenders; fair random
   schedules.
5. **Conversions and endowments:** multi-leg converters with a fee per
   use and human-only use; the lease lifecycle; scheduled endowments;
   asset history.
6. **Case structure:**
   - sub-heats (periods and ticks) with resets;
   - roles that gate instruments, assets and news;
   - team scoring with the RITC ranking policy;
   - interface permissions (manual vs algorithmic).

**Agents and fundamentals** follow as plugins on those features:

- a seeded price path with volatility regimes and news jumps;
- a Black-Scholes option market maker;
- the regulated-price authority;
- merger, ETF-parity and stat-arb fundamental models;
- closing liquidity.

**Protocol:** each feature that changes what a participant sees extends
BNP and FIX in the same slice:

- reference data;
- case period and tick;
- limits and their use;
- fines;
- typed transactions;
- tender kinds;
- conversions and leases;
- endowments.

That is the BNP gap list in `rit-desk-window-parity.md`, and both lists
must stay in step.

**Order (proposed):**

1. close-out and settlement, with portfolio limits (needed by
   LT3/Liquidity Risk, Merger, Volatility and ALGO2e);
2. the fines framework and tender kinds (Liquidity Risk);
3. options settlement plus the Black-Scholes market maker (Volatility);
4. converters and FX (ETF Arb);
5. roles, endowments, forwards and the authority (Electricity).

Each step ends with a playable **case scenario** in `scenarios/`, run
end to end with several desks.
