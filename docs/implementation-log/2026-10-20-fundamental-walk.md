# Fundamental random walk and a desk that quits on SIGTERM (2026-10-20)

The local practice market's BNT price sat at 99-100: the agents' fundamental
was the constant `scheduler.fundamental_price`. RIT cases (LT3, Volatility)
move the price as `P_t = P_{t-1} (1 + r_t)`, `r_t ~ N(0, sigma)`, with
scheduled volatility regimes (`docs/specs/rit-case-primitives.md` section 7).

## What changed

- **Runtime** (`packages/bunting-runtime/src/fundamental.rs`): an optional
  `fundamental_path` in the scheduler config (`serde` default `None`, so
  existing configs keep a constant fundamental). `RandomWalk { seed, step_ns,
  annual_volatility_bps, trading_days_per_year, run_ns_per_day, regimes[],
  min_price_ticks, max_price_ticks }`, all integers. Per-step sigma is
  `annual / sqrt(trading_days * run_ns_per_day / step_ns)`.
- **Determinism (Bunting-added):** fixed-point (Q32, `i128`) integers only, no
  floating point in the generator. Draws are counter-based `SplitMix64` over
  `(seed, step)`; a normal is the Irwin-Hall sum of twelve uniforms minus six
  (variance exactly 1, tails cut at 6 sigma). Floats were rejected because
  `ln`/`cos`/`sqrt` are not promised bit-exact across platforms. The recursion
  is path dependent, so `fundamental(t)` is a pure function of config and run
  time by recomputation from step zero. A one-entry cursor `(step, price)` is a
  cache only: it is not serialised, it is rebuilt from the seed after a restart
  or an earlier query, and memory is O(1). No new journaled input and no wall
  clock: the runtime reads the run clock `logical_time` it already has. The
  price is clamped to `[min, max]` every step (at least 1 tick) and one step's
  return is bounded below by -50%.
- **Agents** (`bunting-agents`): `FundamentalLiquidityProvider`, an existing
  kind that behaved exactly like the static provider, now centres its quotes on
  `observation.fundamental`. Every other kind is unchanged (they quote around
  the last trade; `agents_resume_after_the_venue_is_killed_mid_run` depends on
  that). Informed, mean-reversion and similar kinds already read
  `observation.fundamental`, so they now chase the moving value.
- **Local venue** (`bunting-trading-state` `server_config`): BNT's four liquidity
  providers use `fundamental_liquidity_provider`; the path is seed
  20261020, 0.5 s steps, 40% annualised volatility over a 60 s "trading day"
  (15120 s year), doubling to 80% from run time 300 s. QRC stays agent-free.
- **Desk** (`apps/bunting-desk/src/main.rs`): SIGTERM and SIGINT (Ctrl+C on
  Windows) now call `App::quit` through the app's normal path, so the session
  logs out and `SharedVenue`'s last-client stop runs. Uses `async-signal` 0.2
  (safe `Signals` stream) and `futures-lite` 2, both already in the desk lock
  through GPUI; recorded in `docs/reference-adoption.md`. Not exercised live
  here (no display in the agent shell); the Windows branch is not compiled here.

## Tests added

- `fundamental`: same config gives the same path; different seeds differ;
  absent path is constant; a restarted walker and an earlier query agree;
  empirical annualised volatility of a 20,000-step path is within 200 bps of
  4,000; a regime change moves it to 8,000 +/- 600; clamping; validation.
- `bunting-runtime`: a walking fundamental moves the quotes, and a runtime
  restored from a snapshot mid-run ends with an identical snapshot and state
  hash.
- `bunting-agents`: fundamental provider quotes follow the fundamental (90,
  100, 112); the static provider does not.

## Measurement

- Command: `price_path` (scratch program: `Session::start` as participant 1,
  `Model::tape_prints`, depth sampled every 5 s) over `LocalVenue::start` with
  the real `bunting` executable (`bunting server`), 360 s.
- Build: dev (unoptimized + debuginfo) profile; Linux x86_64; the
  `sonnet/fundamental-walk` branch after commit `b386d2d` plus the
  `fundamental_liquidity_provider` change recorded in the final commit.
- BNT: 654 trades in 360 s (1.8 per second); min 96, max 106, last 97.
- 30 s closes, from t=30 s: 97, 100, 99, 104, 104, 101, 102, 101, 101, 104,
  104, 97.
- Depth (72 samples, 5 s apart): average 5.2 bid levels / 415 lots and 5.2 ask
  levels / 401 lots; minimum 5 levels each side.
- A first run with every provider on the static kind centred on a constant 100
  (before this change in `bunting-agents`) is not part of the numbers above.
- One seed and one run: the range (-4% / +6%) is a sample, not a distribution.

## Checks

See the commit message of the closing commit for the gate counts.

## Remaining

- News-linked jumps and a published fundamental for close-out marks (spec
  section 7 rows stay Partial / Missing).
- The desk signal path needs a live check on a machine with a display.
