# bunting-trading-state instructions

The participant-side trading state shared by every Bunting trading app (the
Ratatui terminal in `apps/bunting-tui` and the desktop app that follows it).
ADR 0047 records the split: this crate holds state and view-model, apps only
render it and map input to its methods.

- `model` and `ticket` are pure and host-neutral: no I/O, no ambient
  time or randomness. Callers pass the current time in microseconds; the
  model answers with the BNP requests to send. They compile for
  `wasm32-unknown-unknown`.
- `session` (native only) drives one `bunting_client::Client`: it connects,
  reconnects with the resume cursor, pings for round-trip time and forwards
  every server message. Sockets and TLS stay in `bunting-client`; requests
  are written on the caller's thread the moment it calls `send`, never queued
  behind a poll interval. Never add delay, batching or smoothing (ADR 0035):
  a slow client must cost its team.
- `local_venue` (native only) prepares and starts a private local venue: a
  throwaway CA and certificates, a server configuration and the `bunting
  server` process. The app then connects to it exactly as to a hosted venue;
  there is no embedded engine and no second code path.
- `model/run.rs` keeps the run stream (status, halts, news; ADR 0042) and
  deals (tenders, OTC negotiations; ADR 0043) as the venue last committed
  them, newest committed sequence wins. Deal request IDs come from the same
  run-unique counter as client order IDs: the venue commits a reused one
  only once.
- This crate never holds market authority. Everything it shows is what the
  venue committed and sent; state it infers locally (pending orders, trade
  arrival times, round trips) is labelled as such.
- Treat the public API as shared between apps: change it additively, and
  keep every collection bounded.
