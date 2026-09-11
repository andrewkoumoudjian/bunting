# Bunting Canonical Terminal Rollout Order

This file is the execution manifest for the approved design in `docs/superpowers/specs/2026-09-10-bunting-rit-gpui-multiplayer-design.md`.

## Required order

Execute the implementation plans in this order unless a task explicitly states an independent parallel-safe path:

1. `2026-09-10-bunting-runtime-correctness.md`
2. `2026-09-10-bunting-competition-replay.md`
3. `2026-09-10-bunting-worker-publication.md`
4. `2026-09-10-bunting-market-history.md`
5. `2026-09-10-bunting-client-extraction.md`
6. `2026-09-10-bunting-gpui-rit-terminal.md`
7. `2026-09-10-bunting-local-state-recovery.md`
8. `2026-09-10-bunting-collaboration-seam.md`
9. `2026-09-10-bunting-terminal-cutover.md`

Each task follows TDD: add the focused failing test, run it and confirm the intended failure, make the smallest production change, run the focused test green, run the task-level regression set, then commit. Do not batch unrelated tasks into one commit.

## Dependency gates

The following cross-plan dependencies are mandatory:

- Competition replay uses the venue-owned logical-time and server-isolation fixes from runtime correctness; it does not reintroduce wall-clock market authority.
- Market history uses committed `TradeExecuted` events and runs only after the server/origin path is stable. Adding `OriginStore::load_event_tail` must update every trait implementation, including `InMemoryOrigin`, server `FileOriginStore`/`NativeOrigin` delegation, and the `CommitRaceOrigin` test mock in `packages/command-transaction/src/lib.rs`.
- Client extraction happens after the market-history reducer exists, so that reducer moves once into `packages/bunting-client` rather than being implemented twice.
- GPUI migration happens after `bunting-client` exists. `apps/bunting-terminal` must depend on `bunting-client`, never on `bunting-tui`.
- The GPUI terminal manifest must define an application feature exactly as `test-support = ["gpui-kit/test-support"]`; all terminal UI tests run with `cargo test --features test-support`. Do not use dependency-feature syntax such as `--features gpui-kit/test-support` in execution commands.
- GPUI chart rendering uses only `gpui_kit::component::chart`/`plot`. `CandlestickChart` consumes authoritative `MarketHistoryProjection`; L1 quote samples never become OHLC.
- Local-state recovery runs after the GPUI test harness exists because its final task adds the explicit recovery action to that shell. Its low-level quarantine code still preserves `local.json` and `scenario.json` and never weakens server restore validation.
- Collaboration runs after the canonical client/UI boundary is established. Collaboration errors cannot alter FIX connectivity or order-submit authority.
- DeltaDB remains behind the generic replication seam. No Delta/Zed collaboration implementation is imported until public source and license are separately audited.
- Terminal cutover is last. `apps/bunting-tui` and the CLI `tui` feature are removed only after the machine-checked workflow parity matrix and packaged live-venue smoke are green.

## Exact packaging paths

`apps/bunting-terminal/scripts/package-macos-arm64.sh` currently defaults `DIST_DIR` to `apps/bunting-terminal/dist`. The canonical release plan therefore uses these exact artifact globs:

```text
apps/bunting-terminal/dist/Bunting-Market-Terminal-v*-macos-arm64.dmg
apps/bunting-terminal/dist/Bunting-Market-Terminal-v*-macos-arm64.dmg.sha256
```

Do not substitute another output directory unless the packaging script and release workflow are intentionally changed in the same reviewed task.

## Golden-file boundary

`tests/goldens/competition-full-run.v1.json` is the existing deterministic simulation-domain golden used by `packages/bunting-engine/tests/simulation_domain.rs`; it is not the `CompetitionArchive` schema and must remain in place. The mixed participant/simulation competition archive contract gets a separate `tests/goldens/competition-archive.v2.json` golden.

## Release stop conditions

Stop the cutover and return to the owning plan if any of these are true:

- canonical root CI is red for a code/test failure;
- a malformed participant/admin client can terminate another venue service;
- reconnect changes open-order limits because of connection-local state;
- competition archive replay cannot reproduce the live canonical event vector and final hash;
- Worker code can prepare/commit a market command or requires the FIX Durable Object;
- the market chart can render quote-derived candles as trade OHLC;
- the GPUI app directly depends on old `gpui-component`, Zed `gpui`, or `bunting-tui`;
- a collaboration path can submit a market command or mutate account/risk state;
- the packaged macOS app has not been exercised against its bundled Wasmer venue;
- any supported TUI workflow lacks a verified GPUI/client/headless replacement.
