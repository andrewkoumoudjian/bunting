# ADR 0047: The trading terminal is a BNP client over shared trading state

- Status: **Accepted** (2026-10-10), implemented in the slice recorded in
  `docs/implementation-log/` for this date. Implements the part of
  [ADR 0031](0031-proposed-bunting-native-client-protocol.md) that makes the
  Ratatui terminal "the first BNP client"; supersedes nothing else.
  Amended 2026-10-11 (slice 39): BNP now carries order-by-order feeds, the
  consolidated tape and the venue's measured access delay (slice 35), and
  the model and terminal show them (decision 5).
- Date: 2026-10-10
- Depends on: ADR 0031 and ADR 0040 (FIX plus BNP only; BNP v1), ADR 0035
  (latency), ADR 0036 (market data), ADR 0044 (native `bunting server`).

## Context

The owner asked (2026-10-10) for the terminal app to be refactored and
reimplemented, and then for a GPUI desktop version of the same app. His
standing direction: the client is an app that connects to a hosted server
over Bunting's own protocol (plus FIX); realism first; teams choose the
venue for each order and venues sit at different distances.

Observed before this decision (`3f92865`): `apps/bunting-tui` was a FIX
initiator (its own `protocol.rs`, tokio, TLS 1.2/1.3 with platform roots)
with an adapted Longbridge component tree and candlestick chart. Its
`--fixture` mode embedded a second market: `local_market.rs` ran
`bunting-engine`, built-in agents and the QUARCC adapter in the terminal
process. That market had to be kept in step with the server by hand; it
broke when the server moved to one run clock (fixed in `3f92865`).
`bunting-client` (slice 25) already implemented BNP once for every app.

## Decision

1. **Shared state, no rendering:** a new package
   `packages/bunting-trading-state` holds what any trading app shows,
   rebuilt from what the venue sent: listings grouped by instrument, one
   direct feed and book per venue listing (`bunting_client::FeedBook`), a
   blotter of trades in the order they reached this client, the
   participant's orders and fills, the account, the kill switch as the
   venue reported it, notices, and the connection with its measured round
   trips. `Model` is pure (time passed in, BNP requests returned) and
   compiles for `wasm32-unknown-unknown`. `OrderTicket` describes an order
   before it is sent, venue included.
2. **One session runner:** `Session` (native) keeps a `bunting_client::Client`
   connected, reconnects with the private-stream cursor, pings once a second
   for round-trip time, and hands every message to the app stamped with the
   instant it was received. Requests are written on the caller's thread the
   moment it sends (`ClientSender`, a small additive `bunting-client` API),
   never queued behind a poll (ADR 0035: a good client adds no delay).
3. **The terminal only renders.** `apps/bunting-tui` is rebuilt from scratch:
   `ui` maps keys to model calls and BNP requests, `view` draws the model
   with Ratatui, `app` is the event loop. Rendering reads state only, so it
   is tested on Ratatui's `TestBackend`. The GPUI app will be a second
   renderer of the same crate.
4. **No embedded engine; one code path.** `bunting tui --local` writes a
   fresh throwaway CA, venue and trader certificates, a scenario and a
   server configuration (`bunting_trading_state::local_venue`), starts the
   same `bunting server` a host runs as a child process, and connects to it
   over BNP with mutual TLS, exactly as to a hosted venue. The local venue
   places venue 2 two milliseconds from the trader, so venue choice costs
   what it would on a real network.
5. **What it shows, honestly labelled.** Per-venue books side by side for
   the selected instrument, with the trader's own resting quantity; trades
   "as they reached you" (each venue's trades arrive after that venue's
   distance; this is not a consolidated tape); per-venue order round trip
   (send to the venue's first answer: real network plus virtual distance
   both ways); `Ping` round trip (no virtual distance). Since slice 39:
   venue feeds are subscribed order by order (falling back to levels where
   a venue sends levels), with each order's queue place and broker; each
   instrument's tape is a separate "from the hub" panel; the access delay
   the venue measured is shown as the venue's measurement.
6. **Configuration:** the same flags and environment as `bunting-trader`
   (`--server`, `--server-name`, `--ca`, `--cert`, `--key`), or a saved
   venue in `terminal.json` version 2. Version 1 files (FIX profiles) are
   refused, not migrated.

## Consequences

- The terminal links no engine, agents, QUARCC adapter, FIX session code,
  tokio or TLS stack of its own; `bunting-cli` no longer needs tokio.
- Local practice exercises the real server, admission sequencer, latency
  map, journal and BNP host, so it cannot drift from a hosted venue.
- Two limits observed while building it were routed to their owners: a
  participant kill switch was run-wide in the engine and canceled nothing
  (fixed per participant, with reset, in slice 32); built-in agents cannot
  quote an instrument listed on two venues, so the local venue lists BNT
  (with agents) on venue 1 only and QRC (no agents) on both.
- The Longbridge Terminal adaptation and the candlestick chart are gone;
  there is no price chart until one is built on the shared state.
- The GPUI app depends on `bunting-trading-state`, whose public API is
  shared and changes additively.

## Rejected alternatives

- **Keep the FIX terminal and add BNP beside it.** Two protocol stacks in
  one app, and FIX session code duplicated outside `simfix-*`.
- **Keep an embedded engine for local play.** A second market host that must
  mirror server semantics by hand; it had just broken on the run clock.
- **Run the server in-process for `--local`.** It would link the server into
  the terminal and share its process fate; a child process is what a host
  runs and keeps the terminal a pure client.
- **State inside the terminal crate.** The GPUI app would have to copy it or
  depend on an app; a package is the reusable unit (AGENTS.md).

## Validation

- `bunting-trading-state` unit tests: listings grouped and every venue
  subscribed; trades reach the blotter with arrival time; a feed gap
  resubscribes; an order's lifecycle with its venue round trip; rejects and
  send failures; an open-orders snapshot closes orders the venue no longer
  lists; round-trip statistics; the kill switch shown only once reported;
  ticket validation.
- `bunting-tui` headless rendering tests on `TestBackend`: both venues'
  books, trades and the connection are drawn; a ticket sends to the chosen
  venue and the order appears with its answer time; the kill switch asks
  first; a bad ticket stays open with the reason.
- `apps/bunting-tui/tests/local_venue.rs`: the generated local venue served
  by the real server; a marketable order fills against an agent, the same
  bid on both QRC venues rests and venue 2 answers at least 4 ms (2 ms each
  way) after sending and later than venue 1, a cancel, the account showing
  the bought lot, and the kill switch report.
- A manual run of `bunting tui --local` in a pseudo-terminal (implementation
  log).

## Operational impact

- `bunting tui --local` writes into `$XDG_STATE_HOME/bunting/local-venue`
  (or `--venue-dir`) and starts `bunting server` from the same executable
  (`--server-bin` names another). The server's output goes to `server.log`
  there. Quitting the terminal stops the server; it shares the terminal's
  process group, so closing the terminal does too.
- Hosted venues are reached with the participant's certificate files, as for
  `bunting-trader`.

## Security impact

- Local certificates and keys are generated per start, never committed,
  and written with owner-only permissions on Unix. The local CA is never
  trusted for anything but that practice venue.
- The terminal can only send ordinary BNP requests under the identity of its
  certificate; nothing in it can reach a book or ledger.

## References

- `packages/bunting-trading-state/`, `apps/bunting-tui/`,
  `packages/bunting-client/src/connection.rs` (`ClientSender`).
- `docs/specs/bnp-v1.md`; ADR 0031, 0035, 0036, 0040, 0044.
