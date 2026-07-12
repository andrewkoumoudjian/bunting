# Vendored third-party source

This directory is reserved for third-party source that Bunting actually compiles or redistributes after explicit approval. Reference repositories belong under `ref/`, not here.

The directory is intentionally empty except for policy files until a dependency spike proves that selective vendoring is necessary.

## Admission requirements

A vendored component must contain:

```text
vendor/<component>/
  LICENSES/
  NOTICE.md
  UPSTREAM.md
  PATCHES.md
  src/...
```

`UPSTREAM.md` records:

- repository URL;
- exact commit and source paths;
- package/crate version when applicable;
- reason normal dependency management was rejected;
- retained and rejected behavior;
- target Bunting crate;
- update owner and review cadence.

`PATCHES.md` records every Bunting change from upstream. `NOTICE.md` contains copyright and attribution text required by the selected files' licenses.

## Approval gates

Before source enters this directory:

1. file-level license and generated/specification-data licenses are verified;
2. `docs/reference-adoption.md` marks the source as a selective source candidate;
3. the smallest viable source set is identified;
4. a normal released dependency has been evaluated first;
5. Worker-bound code passes a minimal-feature Wasm build;
6. transitive dependencies, binary-size and memory impact are measured;
7. equivalence, property and fuzz tests exist;
8. an ADR records any `unsafe`, native code or nontrivial cryptographic/protocol risk.

## Current candidates

- Minimal IronFix tag-value codec files, only if published crate dependencies fail the Worker/Wasm and stability evaluation.
- Small pure market-making formulas or tests from `market-maker-rs`, if independent implementation is less auditable than a notice-preserving adaptation.
- Narrow deterministic helper/test material from `OrderBook-rs`, only when it does not import concurrent structures.

No complete reference repository is approved for vendoring.