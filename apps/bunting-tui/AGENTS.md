# Bunting trading terminal instructions

- This app is a participant client, never a market authority (ADR 0047).
  It renders `bunting-trading-state` and maps keys to its methods; all
  trading state lives in that package so the GPUI desktop app can render the
  same thing. Do not add state, protocol or venue logic here.
- It speaks BNP only, through `bunting-client`. Do not add FIX, an embedded
  engine, agents or any other market host. `--local` joins the shared
  local venue (`local_venue::join_or_start`): the first client starts the
  real `bunting server` detached, later terminals and desks join the same
  market as the next free of eight participants (`--participant N` picks
  one), and the server stops when its last client leaves.
- `ui` is pure (keys → model calls and requests) and `view` only reads; keep
  both testable headlessly on Ratatui's `TestBackend`.
- Pass the session's receive instant to `Model::apply`, and send requests as
  soon as a key produces them: the terminal must add no delay (ADR 0035).
- Label inferred or measured values as such (trades as they reached you,
  order round trip, ping, the venue-measured access delay), and keep the
  direct-feed blotter and the consolidated tape apart: the tape comes from
  the hub on its own path.
