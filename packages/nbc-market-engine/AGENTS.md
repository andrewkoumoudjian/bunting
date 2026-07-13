# NBC market-engine package instructions

This package is the coherent NBC venue-engine port authorized by ADR 0017.

- Every translated module must cite exact JAR class or resource hashes in the translation ledger.
- Preserve unresolved reference parameters as inert provenance; they must not drive behavior.
- Use exact checked units and deterministic hashes without floating-point authority.
- Do not add matching, scheduling, agents, scoring, or recovery behavior before its vertical slice.
- Keep native and `wasm32-unknown-unknown` compatibility.
