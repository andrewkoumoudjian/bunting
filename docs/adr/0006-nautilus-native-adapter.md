# ADR 0006: Native REST/WebSocket NautilusTrader adapter

- Status: Accepted
- Date: 2026-07-11

## Context

Bunting users must be able to trade through NautilusTrader. NautilusTrader's current adapter guide describes a layered integration: a Rust core for HTTP, WebSocket transport, parsing and Python bindings, plus a Python layer implementing instrument providers, live data clients, live execution clients, configuration and factories.

Bunting also supports FIX, but routing NautilusTrader through the local FIX bridge would add an unnecessary protocol translation, a second session state machine and additional recovery complexity.

## Decision

Implement a dedicated Bunting adapter using the native versioned API:

- HTTP for instrument discovery, historical data, run metadata, account snapshots and reconciliation;
- WebSocket for L1/L2 market data, trades, status, orders, executions, positions and account changes;
- a Rust networking and parsing crate;
- PyO3 bindings where required by the current Nautilus integration model;
- a Python adapter layer containing provider, data client, execution client, configuration and factories.

The adapter follows the layout and sequencing documented in the pinned NautilusTrader repository at `c28b1335c95abbf1bef2385def9a75a1b3862f76`.

Required capabilities:

- stable instrument identifier conversion;
- initial snapshots and incremental sequence recovery;
- reconnect with bounded backoff;
- market and limit order submission;
- cancel and modify;
- private execution stream;
- order, fill, position and account reconciliation;
- deterministic timestamp mapping from Bunting logical/event time;
- explicit environment and run selection.

FIX remains available to FIX-native users, but is not the primary Nautilus transport.

## Consequences

Positive:

- aligns with NautilusTrader's supported adapter architecture;
- avoids FIX translation overhead;
- exposes richer run and simulation metadata;
- permits precise native sequence recovery and reconciliation;
- keeps FIX and Nautilus concerns independently testable.

Negative:

- requires both Rust and Python integration code;
- normalized model mappings must track Nautilus releases;
- adapter CI must test against pinned compatible Nautilus versions.

## Rejected alternatives

### NautilusTrader through FIX only

Rejected because it would lose native simulation metadata and duplicate session/recovery work.

### Python-only adapter

Rejected because the official architecture expects performance-sensitive HTTP/WebSocket parsing in Rust and Python integration above it.

### Pretend Bunting is an existing exchange adapter

Rejected because simulation clocks, runs and scenario metadata are first-class concepts and need explicit mappings.

## Validation

- instrument provider loads all run instruments;
- book snapshots plus deltas remain synchronized after reconnect;
- order submit, modify, cancel, partial fill and full fill map correctly;
- startup reconciliation restores open orders, fills, positions and account state;
- adapter tests follow the upstream mock-server and fixture organization;
- supported Nautilus version range is documented and pinned in CI.

## References

- `ref/nautilus-trader/docs/developer_guide/adapters.md`
- `ref/nautilus-trader` at `c28b1335c95abbf1bef2385def9a75a1b3862f76`
