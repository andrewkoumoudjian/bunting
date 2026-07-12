# Reference-oracle test instructions

Oracle harnesses are development-only and must never be linked into production artifacts.

Each harness pins an upstream commit, records license/attribution, accepts deterministic fixture input and emits normalized Bunting-owned output. Store generated fixtures under `tests/fixtures/reference/<oracle>/` with the command, version and expected result.

CI must be able to run Bunting's fixture tests without network access or the external oracle. Oracle refresh jobs may use containers or native runtimes but cannot alter expected fixtures silently.

Use Liquibook and OrderBook-rs for matching, exchange-core for risk/accounting/state hashes, QuickFIX/J and Fixer for FIX, and ABIDES/NeXosim only for scheduler or distributional comparison cases.