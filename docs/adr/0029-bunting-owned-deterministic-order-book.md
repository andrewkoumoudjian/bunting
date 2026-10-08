# ADR 0029: Bunting-owned deterministic order book

- Status: Accepted
- Date: 2026-10-08
- Supersedes: ADR 0013's and ADR 0019's selection of released OrderBook-rs as the production matcher, ADR 0018's "released OrderBook-rs" clause, and ADR 0028 item 10. ADR 0018's single-engine authority and ADR 0019's package ownership (`bunting-engine` owns matching privately) remain.

## Context

ADR 0018/0019 pinned OrderBook-rs 0.10.3 as the production matcher behind a private adapter. Implementation evidence gathered while repairing the engine showed the adapter imposing costs that no configuration, upstream contribution or fork removes cheaply:

1. **Correctness defect.** An IOC limit order that partially fills consumes resting liquidity and then returns `InsufficientLiquidity`, discarding the trade result (`orderbook/modifications.rs`, `is_immediate` branch). Any partially filling IOC therefore failed the whole Bunting command. Slice 9 needed a GTC-then-cancel workaround.
2. **Identity mismatch.** The matcher keys orders by 64-bit `Id::sequential`, while Bunting's canonical `OrderId` is 128-bit. The engine had to persist a run-wide `u64` allocator and a complete reverse index, adding state, validation and a mapping step to every fill.
3. **No cheap ownership.** `DefaultOrderBook` is built from `Arc`/atomics/concurrent maps and is not `Clone`. Each command restored the touched book from a checksummed JSON snapshot and re-serialized it afterwards, so per-command cost scaled with book size, and every committed state carried full JSON book packages.
4. **Non-determinism leaks.** `PriceLevel` statistics read wall-clock time; snapshots had to be rewritten (`first_arrival_time`) before checksumming. Timestamps came from a `StubClock` recreated per command.
5. **Unused surface.** Pegged, trailing-stop, reserve and market-to-limit variants were exposed in the canonical command schema but used by no adapter, and their semantics were never specified at the Bunting level.

The repository owner explicitly directed replacing components that are not ideal instead of adapting around them.

## Decision

`bunting-engine` owns a private, deterministic price-time limit order book (`src/book.rs`):

- one book per listing, keyed directly by canonical 128-bit `OrderId`; no secondary identity space;
- `BTreeMap` price levels, each a FIFO keyed by a monotonically increasing priority; insert, cancel and best-price lookup are `O(log n)`; all traversal is ordered;
- plain owned values: no clocks, randomness, interior mutability or shared ownership, so the book is `Clone` and persists as a canonical list of resting orders (`next_priority` plus orders in side/price/priority order) validated on load;
- semantics: execution at the resting price; GTC, IOC, FOK (feasibility checked against displayed plus hidden quantity before matching), GTD on the run's logical clock and DAY (expiry at session close); post-only rejection; iceberg display with refresh at the back of the level; market orders sweep and cancel their remainder; self-matching permitted (prevention is a policy above the book).

Canonical `OrderKind` is reduced to `Limit`, `Market` and `LimitWithPolicy { time_in_force, post_only, display_quantity }`. `AdvancedLimit`, `AdvancedOrderPolicy` and `PegReference` are removed; `TradeExecuted.upstream_engine_sequence` is removed; `EventEnvelope.event_id` is the run-wide event sequence (the previous command-ID-plus-offset derivation collided across commands).

OrderBook-rs remains only as a **dev-dependency differential oracle**: a test drives both books with seeded random GTC limit, market and cancel streams and requires identical maker/price/quantity fills, cancel remainders, market executions and full depth after every step.

## Consequences

- The engine no longer needs `upstream_to_canonical`, `next_upstream_order_id`, listing snapshot JSON, snapshot caches or `CachedListingSnapshot`; command transactions and services lose their cache parameters.
- Committed state contains books directly; the state hash covers them through canonical serialization.
- Bunting now owns matching correctness. The differential oracle, unit tests of priority/iceberg/identity behavior and replay tests are the guard; any new order type needs both Bunting-level semantics and tests before it enters the command schema.
- Engine snapshot version 3 states from before this change are rejected, not migrated.

## Rejected alternatives

- **Keep the adapter with workarounds:** preserves identity mapping, JSON restore per command and wall-clock canonicalization; fixes none of the structural costs.
- **Fork OrderBook-rs:** retains `Arc`/atomic concurrency machinery the single-writer engine does not need and a 64-bit identity model, while adding fork maintenance.
- **Another external CLOB crate:** no evaluated crate offered 128-bit identities, `Clone` state and logical-clock-only determinism together; the needed core is small enough to own and test against an oracle.
