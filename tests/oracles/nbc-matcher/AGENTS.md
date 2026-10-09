# NBC matcher-oracle instructions

This crate is development-only translated evidence authorized by ADR 0017. It
must never be linked into production packages or exposed as a selectable market
engine. NBC is reference evidence only (ADR 0032). Observed 2026-10-09: this
crate is a standalone workspace member and no test compares it with the engine
book, so it is not currently acting as an oracle. Keep it only if a
differential test shows it catches matching differences the OrderBook-rs
oracle misses; otherwise retire it.

- Do not add features here; no scheduling, agents, scoring, recovery or
  compatibility behavior moves from here into `packages/bunting-engine`.
- Every translated module cites exact JAR class or resource hashes in the translation ledger.
- Keep native and `wasm32-unknown-unknown` compatibility while it exists.
