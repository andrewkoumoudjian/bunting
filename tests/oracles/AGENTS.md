# Reference-oracle test instructions

Oracle harnesses are development-only and must never be linked into production artifacts.

Implemented today: the OrderBook-rs differential test inside `packages/bunting-engine` (dev-dependency) and the QuickFIX-Go interoperability test under `tests/interop/`. `nbc-matcher` remains only until its fate is decided under ADR 0032.

Each new harness pins an upstream commit, records license/attribution, accepts deterministic fixture input and emits normalized Bunting-owned output. Store generated fixtures under `tests/fixtures/reference/<oracle>/` with the command, version and expected result. CI must run Bunting's fixture tests without network access or the external oracle; refresh jobs cannot alter expected fixtures silently.

Candidate oracles (not implemented; add only with a reviewed adoption record): Liquibook for matching, exchange-core for risk/accounting, QuickFIX/J and Fixer for FIX, ABIDES/NeXosim for scheduler or distributional comparisons.
