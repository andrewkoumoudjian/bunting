# Expanded market/exchange and agent algorithm research — 2026-10-08

**Status:** research / proposals only. This document does **not** change implemented Bunting behavior, adopt external source, claim model fidelity, or approve a new market design.
**Bunting source baseline:** [main@24965ac](https://github.com/andrewkoumoudjian/bunting/tree/24965acca336776aeda2c4361995001bd9794ec0).
**Companion documents:** [first lightweight-source and queue-reactive audit](2026-10-08-agent-market-simulation-implementations.md); [core-first roadmap](../plans/2026-10-07-evidence-led-core-roadmap.md); [architecture audit](2026-10-07-independent-core-architecture-audit.md); [ADR 0024 on admission intervals](../adr/0024-discrete-matching-interval-fairness.md); [proposed ADR 0028 on headless authority](../adr/0028-proposed-headless-run-authority.md).

## Findings and boundaries

There are **five different algorithm classes**, and mixing them produces an unrealistic or irreproducible market: (1) exchange-side matching/clearing, (2) synthetic order arrivals, (3) autonomous traders, (4) order execution/routing, and (5) causal event delivery/recovery. Bunting needs one authoritative matcher and ledger; everything that generates orders is a participant or scheduled exogenous input entering the ordinary authentication, risk, admission and commit path. No standalone simulator should become an alternative book or shadow ledger.

The [existing agent source](https://github.com/andrewkoumoudjian/bunting/blob/411e0baf767d4c81c27308f25117f1013bbd4161/packages/bunting-agents/src/lib.rs) declares over 40 policy kinds. The generic Avellaneda–Stoikov and GLFT names represent simplified quote adjustments; cross-venue decisions can use a single book's imbalance; generic Hawkes state does not schedule an intensity-sampled next event. The [runtime](https://github.com/andrewkoumoudjian/bunting/blob/411e0baf767d4c81c27308f25117f1013bbd4161/packages/bunting-runtime/src/lib.rs) is single-instrument-configured and scans for the next agent wake. The [engine transition](https://github.com/andrewkoumoudjian/bunting/blob/411e0baf767d4c81c27308f25117f1013bbd4161/packages/bunting-engine/src/lib.rs#L850-L950) still restores an affected listing book from snapshot before command processing. These are observed source-level limitations, not performance measurements.

## I. Exchange/venue matching, allocation, market design

| Mechanism | Algorithm | Bunting choice | Acceptance fixture |
|---|---|---|---|
| **Continuous double auction (CLOB)** | Best-price, then FIFO time priority within a price; partial fills, stop/IOC/FOK conditions | **Retain OrderBook-rs as the production reference.** No duplicate custom FIFO implementation. | Equal-price arrival ordering, amend/cancel queue priority, maker/taker IDs, deterministic restore, STP and exact fills |
| **Price/pro-rata** | Allocate incoming size according to displayed eligible resting quantities; deterministic integer-lot rounding and leftover allocation | Future separate venue matching policy; useful for futures venues, not a reason to rewrite existing stocks CLOB | All allocated quantities sum to actual matched lots, no order gets excess, residual allocation matches documented rule |
| **Hybrid FIFO/pro-rata and top-order priority** | Percentage split, threshold, lead-market-maker allocations and optional priority first | Future per-product venue rules; CME documents multiple matching algorithm compositions | Rule-specific staged allocation tests, deterministic ties and exact journal settlement |
| **Opening/closing call auctions** | Collect auction-eligible orders; find price maximizing executed volume, then apply explicitly selected tie breaks | **High-value after multi-day engine is complete** for equities and RIT games | Single clearing price, imbalance projection, auction-only orders, no pre-auction executions, every fill balanced |
| **Frequent batch auction (FBA)** | Repeated **uniform-price clearing** of a sealed batch at a single price | Research-only alternative to continuous FIFO, requires separate accepted ADR | Same input differs meaningfully from batched FIFO on a defined test case |
| **Dark pool / midpoint cross** | Hidden order interest, reference-price/midpoint execution, min-size and queue/priority policy | Deferred unless competition scenarios require it | Hidden vs public information, no price/reference lookahead, exact internal crossing reports |
| **Halt/reopen auctions and collars** | Venue phase machine and limit states control eligibility and auction transitions | Necessary session-policy extension after economic core | Venue-by-venue halt, DAY expiry, reopening policy, no closed-session fills |
| **Order types** | Iceberg/replenishment, pegged, post-only, stops, GTD/DAY and replace policies | Use existing upstream types and add Bunting-level tests, not parallel implementations | Queue priority, hidden displayed size, expiry, maker/taker and conservation |

Crucial correction: **ADR 0024's shared 100 ms order-admission interval with FIFO release is not a frequent batch clearing auction.** Exchange price discovery, order allocation, and transport-level fairness are independent policies. Changing one requires a venue contract and tests.

Primary references: [CME Globex Matching Algorithm Steps](https://cmegroupclientsite.atlassian.net/wiki/spaces/EPICSANDBOX/pages/457218521/CME+Globex+Matching+Algorithm+Steps); [Nasdaq Opening and Closing Cross](https://www.nasdaqtrader.com/Trader.aspx?id=OpenClose); [Budish, Cramton & Shim (2015) on frequent batch auctions](https://doi.org/10.1093/qje/qjv027); [auction implementation details (2014)](https://doi.org/10.1257/aer.104.5.418); [Jusselin, Mastrolia & Rosenbaum (2019) on auction duration](https://arxiv.org/abs/1906.01713); [Nagumo & Shimada (2024) on call-auction depth](https://arxiv.org/abs/2407.19390).

## II. Generative background liquidity and stochastic market processes

The cheapest plausible interactive synthetic market is **a generator of real market messages**, submitted by actual simulated accounts and executed by the real venue. It does not require an OS thread or expensive strategy object for each synthetic counterparty.

| Family | Generated behavior | Computational / modeling cost | Empirical applicability |
|---|---|---|---|
| ZI-C and simple random order flow | Random bounded feasible orders with seeded side, price offset and quantity | Very low | Useful null; weak cancellation/size/volatility realism |
| Homogeneous Poisson with iid marks | Exponential interarrival, categorical add/cancel/market messages and sampled sizes | Low | First point-process baseline; misses intraday seasonality/clustering |
| Seasonal/non-homogeneous Poisson | Time-of-day/session dependent intensity | Low | Needed to distinguish open/close bursts and quiet periods |
| State-dependent **queue-reactive** | Intensities depend on queue sizes/spread/imbalance, sampling limit/market/cancel messages | Low–medium; compact tables | **First preferred calibrated generator** |
| **Marked/multi-level queue-reactive** | Jointly sample event type, level, price offset and size conditionally | Medium | **Preferred realism upgrade**, addresses unrealistic cancellation and order sizes |
| **Marked multivariate Hawkes** | Recent additions, cancels, trades and other events excite future event intensities | Medium–high | If clustering remains wrong after QR; requires stability + time-rescaling checks |
| **Deep Queue-Reactive / MDQR** | Neural rates across levels and conditional order sizes | High training; inference cost depends model | Research alternative if labeled L3 data suffices and held-out improvement justifies it |
| Neural Hawkes / autoregressive models | Recurrent/learned conditional event time, type and mark | High | Research comparator, caution with out-of-distribution agent interventions |
| Conditional GAN/diffusion LOB generators | State or message sequence generation | High | Offline comparator; must demonstrate executable causal message coherence before using for interactive Bunting agents |
| L2/L3 **historical replay** | Reconstruct price/size/ID book history from recorded messages | Medium | Excellent conformance oracle; a fixed tape cannot by itself adapt to new participant market impact |

For state s and allowed event types e, let λ_e(s) be intensity and Λ(s)=Σ_e λ_e(s). Draw next duration Δt=−ln(U)/Λ(s), then event e with probability λ_e(s)/Λ(s). For a chosen event sample a conditional size and level; a cancellation must target an **actually outstanding, owned** order. At a market-state change or calendar boundary recompute eligible rates; handle Λ=0, negative/overflow rates, empty books, suspended listings and bounded memory explicitly. This is a Gillespie-style conditional next-reaction simulator and can run only when events occur.

For a marked multivariate Hawkes model use λ_e(t,s)=μ_e(s)+Σ_{j,k<t,type=j} α_{e,j} exp(−β_{e,j}(t−t_k)); normalize kernel weights correctly and require a defensible stability condition (e.g. subcritical spectral radius for standard linear Hawkes). Use thinning/integrated-hazard sampling, not a label that merely increments a state variable during fixed interval wakes. With exponential kernels, decaying sufficient statistics avoid rescanning complete history.

References and intended tests:
- [Cont, Stoikov & Talreja, *A Stochastic Model for Order Book Dynamics*](https://doi.org/10.1287/opre.1090.0780) — queue-state event probabilities.
- [Huang, Lehalle & Rosenbaum, *The Queue-Reactive Model*](https://arxiv.org/abs/1312.0563) — foundational state-dependent arrivals.
- [Bodor & Carlier (2024), *Importance of Order Sizes*](https://arxiv.org/abs/2405.18594) — conditional marks.
- [Bodor & Carlier (2025), *Deep Learning Meets Queue-Reactive*](https://arxiv.org/abs/2501.08822) — multidimensional deep intensities.
- [Noble, Rosenbaum & Souilmi (2026), *Bridging the Reality Gap*](https://arxiv.org/abs/2603.24137) — queue-reactive simulator calibrated for microstructure/impact.
- [Morariu-Patrichi & Pakkanen, state-dependent Hawkes](https://arxiv.org/abs/1809.08060).
- [El Karmi (2025), deterministic C++ Hawkes LOB](https://arxiv.org/abs/2510.08085) — deterministic engine, exponential/power-law Hawkes, diagnostics.
- [Lalor & Swishchuk (2025), neural Hawkes LOB](https://arxiv.org/abs/2502.17417).
- [Coletta et al. (2023), conditional generator robustness](https://arxiv.org/abs/2306.12806).
- [Backhouse et al. (2025), diffusion order-book states](https://arxiv.org/abs/2509.05107).
- [Nagy et al. (2025), LOB-Bench](https://arxiv.org/abs/2502.09172): distributions of spreads, depth, size, interarrival time, cancel behavior, impact and cross-correlations. [Reference code](https://github.com/peernagy/lob_bench).

**Latent value is not a second tape.** A scheduled correlated diffusion, mean-reverting state, or jump/news model may supply latent fundamental beliefs to selected agents, but executed price must emerge from order matching. Do not also overwrite traded prices from an exogenous price model; this double-counts impact and makes arbitrage meaningless.

## III. Adaptive and strategic trading agents

| Family | State and policy | Intended simulation role | Suggested scope |
|---|---|---|---|
| ZIC, Giveaway, Shaver | Reservation price, random feasible or one-tick-improved quote | Null competition agents | Keep / independently validate |
| ZIP (Zero-Intelligence Plus) | Learns margin/aggressiveness after trades/quotes | Lightweight adaptive agent | **High priority for RIT-style experiments** |
| GDX | Belief-based trade probabilities and payoff optimization | Strategic CDA auction competitor | Compare against ZIP and AA under diverse cases |
| Adaptive Aggressive (AA) | Adapts quote aggressiveness to market equilibrium/volatility estimates | Strategic auction competitor | Evaluate; no blanket dominance claim |
| PRZI, PRSH, PRDE | Parametric quote distribution with hill climb or differential evolution | Heterogeneous, low-resource learning populations | **Good low-CPU diverse agents** |
| Inventory-skew maker | Open order IDs, fills, spread, markouts and inventory target | Foundational continuous liquidity | **Implement faithfully first** |
| Avellaneda–Stoikov | Inventory/horizon/variance/risk-aversion reservation price and calibrated quote fill intensities | Research maker | Mathematical equation fixtures; current name is heuristic |
| Guéant–Lehalle–Fernandez-Tapia (GLFT) | Finite-horizon inventory/quote optimization and order-arrival assumptions | Stronger maker | Build after inventory-skew baseline |
| Queue-aware and microprice maker | Queue position, cancel latency, delivered imbalance and adverse-selection horizon | Realistic sophisticated liquidity | Incorporate queue and public-delivery semantics |
| Informed/value trader | Delayed/noisy private signal versus executable prices net fees | Price discovery and adverse selection | Must use only permitted information |
| Trend/mean reversion / order-flow momentum | Delivered price or flow windows, inventory limits | Positive and negative feedback / crash tests | Avoid forced profitability assumptions |
| Institution TWAP/VWAP/POV | Parent remaining size, schedule, observed traded volume, actual child fills | Execution workload | Need real volume and fill accounting |
| Almgren–Chriss / implementation shortfall | Risk/temporary/permanent impact tradeoff and parent trajectory | Optimal execution research | Compare with TWAP at same fees and opportunities |
| Fee-aware smart order router | Estimate fill costs/probabilities by venue; route IOC and passive child orders | Cross-listing trader | Distinct ListingKeys, no implicit venue |
| Cross-venue arbitrage | Delivered executable NBBO, fees, latency, settlement and leg risk | Multi-venue realism / QUARCC | Do not route using future internal book |
| Pairs/stat-arb | Delivered correlated instrument history, hedge ratio/spread, bounded leverage | Optional multi-asset strategy | Scenario add-on |
| Tender/news | Private schedule, tender economics and hedging constraints | RIT/NBC case agent | All tender fills must post economically |
| Multi-agent RL | Learn from economically correct interactive episodes | Research client only | Don't require training/inference in core |

Representative literature and algorithms: [Dave Cliff's BSE (2018)](https://arxiv.org/abs/1809.06027) and [MIT-source agent implementation](https://github.com/davecliff/BristolStockExchange/tree/6ebc4155440b07c6102c888040899e6a01383f43); [De Luca & Cliff (2011) on AA](https://doi.org/10.5591/978-1-57735-516-8/IJCAI11-041) vs [Snashall & Cliff (2019) challenging its universal ranking](https://research-information.bris.ac.uk/en/publications/adaptive-aggressive-traders-dont-dominate); [Avellaneda–Stoikov (2008)](https://doi.org/10.1080/14697680701381228); [GLFT (2011)](https://arxiv.org/abs/1105.3115); [Almgren–Chriss](https://doi.org/10.21314/JOR.2001.041); [Cont & Kukanov optimal multi-venue order placement](https://arxiv.org/abs/1210.1625); [ABIDES-Gym](https://arxiv.org/abs/2110.14771); [ABIDES-MARL (2025)](https://arxiv.org/abs/2511.02016).

**Static imbalance and event-flow imbalance are different:** compute top-queue I=(bid_qty−ask_qty)/(bid_qty+ask_qty) using checked wide/fixed-point arithmetic. The present integer division often truncates nonextreme I to zero. A simple **volume-weighted microprice proxy** is (best_ask×bid_qty+best_bid×ask_qty)/(bid_qty+ask_qty), with explicit empty-queue handling; this is not the whole calibrated long-horizon Stoikov microprice model. Also measure **order-flow imbalance** from top-of-book adds, cancels, market orders and price changes, as in [Cont, Kukanov & Stoikov](https://arxiv.org/abs/1011.6402). [Stoikov's micro-price research](https://papers.ssrn.com/sol3/papers.cfm?abstract_id=2970694) and [Gould/Bonart queue imbalance research](https://doi.org/10.1142/S2382626616500064) are useful microstructure anchors.

For multi-asset interaction, [Cont, Cucuringu & Zhang (2021)](https://arxiv.org/abs/2112.13213) found lagged cross-asset flow effects, without evidence that heavy contemporaneous cross-impact machinery always improves fit. Start with correlated scheduled fundamentals plus limited, causally observed cross-asset signals before introducing a general joint LOB generator.

## IV. Source implementations — expanded, commit pinned

These are **reference implementations**, not endorsed dependencies; every imported source would need separate review, license files, commit metadata, notices and independent tests. GitHub API metadata may disagree with README or embedded code comments, so ambiguity remains explicit.

| Project and revision | Language and rights evidence | Relevance |
|---|---|---|
| [BristolStockExchange@6ebc415](https://github.com/davecliff/BristolStockExchange/tree/6ebc4155440b07c6102c888040899e6a01383f43) | Python; embedded source MIT, GitHub API license NOASSERTION | **Best compact strategic agent reference**: ZIP/PRZI etc; not a latency-faithful exchange |
| [hawkes-lob-simulator@54ddab6](https://github.com/grsilva9/hawkes-lob-simulator/tree/54ddab625d3f924ce65c074ea669f387424030de) | C++17/pybind11, README reports MIT, API license metadata unverified | Six message types and multivariate Hawkes; remove its alternate matching engine |
| [High-Frequency-Trading-Simulator@780809e](https://github.com/sohaibelkarmi/High-Frequency-Trading-Simulator/tree/780809e655fe6ba7a3bd74d91966ed3b408da273) | C++/Python; license unverified | Hawkes kernel and order-flow diagnostics |
| [lob-engine@8fa3ddc](https://github.com/Bilal-Aamir-Yousuf/lob-engine/tree/8fa3ddcf91cdd03ae5b52d03278cc2337a161271) | C++17; MIT | Hawkes, execution-impact reference, pooled book performance comparisons (upstream claims only) |
| [microstructure-sim@ca9fe4c](https://github.com/mwaleedta/microstructure-sim/tree/ca9fe4c1dd0b14587e0658ad23d41b7ba836d447) | Rust; license metadata unverified | Queue-position, latency and maker implementation ideas |
| [Cpp-Multi-Agent-Trading-Simulator@3050059](https://github.com/stevie-x/Cpp-Multi-Agent-Trading-Simulator/tree/30500595c15e97a8fe01bf924e3aab922bdd74ae) | C++17; README states MIT; API metadata unverified | Small strategy mixtures, fixed allocations and agent tests |
| [DeepQR-Microstructure@ad9bf52](https://github.com/archer-paul/DeepQR-Microstructure/tree/ad9bf5229951315463c11e53010b0d36323d3819) | Python/notebooks; MIT | QR versus Deep QR/MDQR calibration research |
| [JAX-LOB@d1f5966](https://github.com/KangOxford/jax-lob/tree/d1f596610b04a09941c7a1b609e5bab541ecfc98) | JAX; license metadata unverified | Parallel batched RL worlds, not single-run engine speed |
| [LOB-Bench code](https://github.com/peernagy/lob_bench) | Python; verify license before borrowing source | Compare conditional/statistical distributions of real vs generated LOB messages |

Already covered by the focused note: [Queue-Reactive C++](https://github.com/SaadSouilmi/Queue-Reactive/tree/3080096cc0c79f43f1b82112cbde713710c014f6), [ABIDES Rust](https://github.com/mariotrerotola/abides-rs/tree/98c236fd7561edfa58933f883157e838a768a249), [TALON C++](https://github.com/pankajj6/talon/tree/42554957225ac7d518609c6a5be6c3d4fb5714f9) (AGPL), [hftbacktest Rust](https://github.com/nkaz001/hftbacktest/tree/5f3ec40b2afb764e0fea112f941ed85523ef4e88) and [market-sim C++](https://github.com/ptorpis/market-sim/tree/0ad2a6a4099325940945e36c1f9449487b74fe86).

Do not rank these by advertised throughput: 20 million isolated matches/sec and 5 million global simulation events/sec exclude very different work and do not quantify Bunting's risk + ledger + durability cost.

## V. The smallest deterministic Rust architecture that can support them

1. **Run-owned mutable state:** one in-memory matcher per ListingKey, consolidated fungible inventory, one transactional posting journal. The run writer takes one canonical admitted command and produces one durable ordered economic transition. Snapshots are checkpoints, not per-action rebuilding inputs.
2. **Event heap:** ordered keys (logical instant, phase, stable sequence, event ID), with events for calendar/auction, external news, exchange admission, committed publication, delayed public/private delivery and next strategic/generator wake. Stable O(log N) scheduling is preferable to O(number of agents) scans. Preserve the accepted ADR 0024 interval policy separately.
3. **Model interface:** per-version frozen scenario config, seed domain, small mutable state and one bounded action emitter. Strategies observe only delivered listing data/private fills; synthetic traffic must own orders/funds; market matching consumes normal participant commands through QUARCC/admission. Agent policies cannot inspect a future authoritative book snapshot.
4. **Stochastic determinism:** cross-platform floating-point exponential/log/thinning draws may produce different timestamps even under equal RNG seeds. Use a documented integer time quantization, stable RNG version/domain separation and **persist the realized exogenous message/time stream**; replay must not silently resample on a new host. Persist policy internal state and outstanding client IDs at the same durable checkpoint as the exchange.
5. **Economics:** every fill, cancel, fee, borrow, inventory mark, OTC/tender and settlement is reconciled; real cash accounts fund simulated liquidity; no magical fill-generation privileged path. News/fundamental shocks are scheduled information to selected agents, not direct mutation of last traded price.

**Causality fixture:** an agent observes venue A at t=8; venue B's true best ask changes at t=10 but its feed is delivered at t=15. The agent deciding at t=12 must not see the t=10 ask, regardless of the current authoritative book. Venue A and B can have the same InstrumentId with separate queue priorities. A restart at t=13 must reproduce the same subsequent fills/fees and event hashes.

**Auction fixture:** run fixed orders through continuous FIFO and separately through a uniform-price FBA. FIFO must honor maker queue timestamps; the FBA must maximize executable crossed size at its clearing price and respect the specified tie rules. Batching ingress and then FIFO matching must never be reported as FBA.

## VI. Concrete model-selection ladder and gates

| Research question | Cheapest candidate | Upgrade only when | What to measure |
|---|---|---|---|
| Can a plausible continuous book exist? | Seasonal Poisson + inventory maker | Quote, spread, depth or interarrival mismatches persist | Spread L1/L2, message mix, order size, cancellation, fill rate |
| Can state dependence fix spread/depth? | Marked queue-reactive | Held-out conditional size/depth/impact still wrong | Conditional depth, queue occupancy, price response, event-to-event markout |
| Does clustering remain underfit? | State-dependent exponential Hawkes | Poisson/QR residual goodness-of-fit decisively fails | Time-rescaling, durations, intensity stationarity, excess bursts |
| Are educational trading populations rich enough? | ZIC, ZIP, PRZI, simple value agent | GDX/AA give meaningful incremental behavior | Allocative efficiency, profits, price discovery, seed stability |
| Is market making financially credible? | Inventory skew + queue-aware cancel | AS/GLFT independently fits arrival, markouts better | Realized spread, inventory tail, toxic fill markout, fee-adjusted P&L |
| Do venues interact correctly? | Fee-aware router + stale quote arbitrage | Need more complex causal network routing | Crossed NBBO duration, realized arb P&L after leg/fees, causal timestamps |
| Are auctions needed? | One opening/closing call auction profile | Detailed rule support demanded by an explicit venue profile | Volume clearing, reference prices, ties, phase transitions |
| Are expensive generative models necessary? | Best validated QR/Hawkes | Neural model passes held-out/intervention tests and CPU cost is justified | LOB-Bench + realistic adverse-selection/impact and model-drift metrics |

**Design of empirical validation:** use historical L3 data with full add/cancel/trade/order-ID semantics when possible; separately train/calibrate and validate by day and instrument, distinguishing large-tick vs small-tick. Keep an explicit null generator, paired seeds, frozen diagnostic metrics, held-out impact/response functions and long-horizon stability. A strategy being profitable against synthetic counterparties is not evidence that its generator represents real markets.

## VII. Proposed, dependency-ordered implementation tickets

- **T0. Correct current low-level policies:** checked rational imbalance/microprice, quote ownership, cancel/requote rather than indefinitely stacking live orders, exact paper-name provenance.
- **T1. Event-driven scheduler and causal observations:** heap, public/private delivery time, checkpointed RNG/event sources, stable replay. Only after core run authority can checkpoint economically.
- **T2. Low-cost traffic:** seasonal ZI/Poisson null and marked queue-reactive sampling through actual simulated participant accounts. Keep all cancellation references real.
- **T3. Deliberate participant diversity:** inventory-aware maker, BSE-derived ZIP/PRZI, noisy value investor and TWAP/VWAP parent execution; verify fill/accounting effects and published algorithms.
- **T4. Multiple venues:** delayed feed reconstruction, fee-aware routing and cross-venue arbitrage with leg risk; never use a direct query to future book state.
- **T5. Calibrate against LOB-Bench:** held-out impact/depth/size/arrival/markout discrepancies; only then Hawkes or more costly neural generators if needed.
- **T6. Additional matching policies (separate ADR):** opening/closing clearing auctions before exotic pro-rata/dark venue variants, unless a competition requirement changes their order.

**Non-goals:** no second production matcher, no unlicensed/AGPL code copied into Bunting without approval, no deep RL as an engine dependency, no host-dependent stochastic replay, no throughput promises based on unrelated microbenchmarks, and no hard-coded market-price moves that bypass the CLOB.
