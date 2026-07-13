# Unresolved evidence

No extracted stream was encrypted or inaccessible. The unknowns are behavioral because the corpus contains client/API software, not the authoritative RIT server implementation or case files.

## RIT engine behavior not proved by these installers

- exact matching priority, hidden/reserve behavior, self-trade prevention, auction rules, halt rules, and cancel/replace priority;
- authoritative validation order and exact rejection/status-code mapping;
- tick scheduling, pause/resume transitions, inter-period settlement ordering, and termination rules;
- rate-limit windows, burst behavior, API read/write accounting, and delay queuing semantics;
- fee, rebate, mark-to-market, interest, coupon, dividend, settlement, distressed-cover, and fine formulas;
- buying-power, NLV, realized/unrealized P&L, VWAP, Sharpe, scoring, and leaderboard formulas;
- tender winner selection, reserve handling, private targeting, compliance behavior, and tie-breaking;
- news/event generation and whether news changes fundamentals directly or only informs participants;
- path-following liquidity generation and all virtual-trader/agent formulas;
- OTC price-expression evaluation, break workflow, counterparty selection, and settlement correction;
- asset lease allocation, renewal, conversion, containment, backhaul, and distressed logic;
- spread atomicity, transport-arbitrage validation, trade-at-settle, electricity, options, bond, swap, forward, and fixing formulas;
- server persistence, replay, snapshots, recovery, idempotency, deduplication, ordering, and state hashes;
- protocol wire serialization beyond CLR signatures and named transports;
- dynamic RTD refresh/error behavior under disconnect, malformed topics, and high topic counts.

## Signature limitations

Both MSI signature streams and certificate chains parse. A Windows Authenticode/MSI trust verification was not available in the completed static pass, so current trust, revocation, and timestamp-policy validation remain unresolved. Individual PE files record only embedded Authenticode-directory presence in the inventory.

## License and clean-room boundary

The supplied proprietary payloads have no adoption license recorded in the repository. Their implementation bodies, resources, UI text, and documentation are prohibited to copy into production. Stable external names needed for compatibility, independently written behavioral specifications, field inventories, and clean-room tests may be retained with this evidence record. Any broader source adaptation requires a separate authorization and an update to `docs/reference-adoption.md`.

## Required next evidence

The highest-value next inputs are authorized case files; official REST/RTD/VBA documentation; captured server responses; isolated traces for order validation, timing, tenders, settlement, and rate limiting; and any licensed server implementation. Until then, the feature ledger assigns engine owners while labeling exact semantics unresolved rather than inventing compatibility.
