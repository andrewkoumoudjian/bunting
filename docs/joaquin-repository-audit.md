# Joaquín Bejar Rust repository audit

This audit focuses on repositories materially related to Bunting's exchange, market-data, strategy, protocol, and simulation requirements. Presence in this document is not automatic dependency approval.

## Adopt now

### OrderBook-rs

- License: MIT.
- Decision: production dependency pinned to `0.10.3`.
- Use directly: matching, order types, price levels, trade results, snapshots, restore, replay helpers, lifecycle tracking, STP, fees, risk hooks, kill switch, expiry, depth, metrics, impact, and enriched snapshots.
- Copy/adapt: focused examples and tests only, with source-path attribution.
- Do not copy: native thread/channel/NATS/file-journal runtime scaffolding into the Worker.

### PriceLevel

- License: MIT.
- Role: transitive core used by OrderBook-rs.
- Decision: pin `0.8.4` for type identity and audit it alongside every OrderBook-rs upgrade.
- Use directly through upstream re-exports. Avoid a parallel Bunting price-level implementation.
- High-value tests: partial-fill priority preservation, checked arithmetic, validation, property tests, and concurrency linearization models.

### workers-rs

- License: Apache-2.0.
- Decision: production Worker and Workers Cache API dependency.
- Copy/adapt: official Cache API usage patterns, especially `Cache::default`, `get`, `put`, response cloning, `Cache-Control`, and ETag handling.

## Add as references for near-term work

### Option-Chain-OrderBook

- License: MIT.
- Current pin: `19e8e45bf122c3ebe3e1784f73e04adba2781ea6`.
- It composes OrderBook-rs rather than replacing it and exposes option-chain hierarchy, deterministic expiry sweeps, per-call fill attribution, snapshots, and sequencer commands.
- Decision: reference and future options-module dependency candidate. Do not pull OptionStratLib, chrono, rust-decimal, and chain-wide structures into the equity kernel prematurely.

### market-maker-rs

- License: MIT.
- Use: strategy decomposition, Avellaneda-Stoikov/GLFT-style formulas, quote intents, risk separation, and tests.
- Do not use as a whole today: its manifest still pins the much older `orderbook-rs = 0.4` and includes optional native API/runtime layers.
- Copy/adapt only pure formulas and tests after exact unit, rounding, and provenance review.

### IronSBE

- License: MIT.
- Current pin: `cf365e4815c04ff31acd81568952e9ff477c6d89`.
- Use later: SBE core/schema/codegen and market-data gap/snapshot recovery patterns.
- Do not import transport, channels, server, or Tokio runtime into the Worker kernel.
- A dependency spike must measure Wasm support and generated-code size.

### fauxchange

- License: MIT.
- Current pin: `293bdc52bedc816f76da5db106f44535e4438593`.
- The repository explicitly states that v0.0.1 reserves the name and no implementation API exists yet.
- Nothing can be copied today. Its useful contribution is the same composition direction Bunting now adopts: OrderBook-rs plus standard FIX/WebSocket/REST surfaces, replay, synthetic data, and configurable microstructure.

## Reference only

### matchbook

- License: MIT.
- Useful ideas: explicit order/open-order account separation, price-time key encoding, free-list reuse, REST/WebSocket channel taxonomy, and strict lint policy.
- Not a Bunting implementation donor: it is a Solana/Anchor system with on-chain accounts, Geyser, TimescaleDB, Redis, Tokio, and Solana-specific constraints.

### OptionStratLib

- Use only for future option analytics, volatility, Greeks, and strategy fixtures.
- It should not enter the equity matching kernel.

### OptionChain-Simulator

- Candidate source for synthetic option-chain scenario generation and distributional fixtures.
- Any stochastic model must be wrapped in Bunting's explicit seed/version policy.

### deribit-fix, alpaca-rs, ig-client, and DXlink

- Native adapter and protocol references only.
- They may inform authentication, reconnect, message mapping, and fixtures, but do not belong in the Worker matching kernel.

### otc-rfq and quant-trading-system

- Useful only if Bunting later adds RFQ workflows or a broader execution/portfolio layer.
- They are not required for the current central limit order book.

## Copy-verbatim policy

MIT permits copying with the license notice, but verbatim copying is not automatically the easiest maintenance choice.

Use this preference order:

1. normal released dependency;
2. call the upstream public API;
3. adapt an upstream example or test into a Bunting-specific fixture;
4. copy a small implementation unit with header, commit, path, license, and divergence notes;
5. minimal fork only for an upstream-blocking Wasm issue.

No repository reviewed here justifies copying its complete source tree into Bunting.
