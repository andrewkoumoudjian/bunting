# NBC matcher-oracle instructions

This crate is development-only translated evidence authorized by ADR 0017. It
must never be linked into production packages or exposed as a selectable market
engine.

- Every translated module must cite exact JAR class or resource hashes in the translation ledger.
- Preserve unresolved reference parameters as inert provenance; they must not drive behavior.
- Use exact checked units and deterministic hashes without floating-point authority.
- Do not add scheduling, agents, scoring, or recovery behavior here; proven
  non-matching compatibility belongs in `packages/bunting-engine`.
- Keep native and `wasm32-unknown-unknown` compatibility.
