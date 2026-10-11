# Bunting trading terminal instructions

- This app is a participant client, never a market authority (ADR 0047).
  It renders `bunting-trading-state` and maps keys to its methods; all
  trading state lives in that package so the GPUI desktop app can render the
  same thing. Do not add state, protocol or venue logic here.
- It speaks BNP only, through `bunting-client`. Do not add FIX, an embedded
  engine, agents or any other market host. `--local` starts the real
  `bunting server` as a child process and connects to it like any venue.
- `ui` is pure (keys → model calls and requests) and `view` only reads; keep
  both testable headlessly on Ratatui's `TestBackend`.
- Pass the session's receive instant to `Model::apply`, and send requests as
  soon as a key produces them: the terminal must add no delay (ADR 0035).
- Label inferred or client-measured values as such (trades as they reached
  you, order round trip, ping), and never present the direct-feed blotter as
  a consolidated tape.
