# Initialization prompt: native Rust tRPC and authorized NBC port

> **SUPERSEDED:** This prompt is retained as historical evidence only. New work follows ADR 0020 and [`../plans/corrected-bunting-implementation-plan.md`](../plans/corrected-bunting-implementation-plan.md).

Status: historical prompt superseded for new work by [`implement-unified-bunting-engine-foundation.md`](implement-unified-bunting-engine-foundation.md), ADR 0018 and ADR 0019

Use this prompt in a fresh implementation session:

```text
Work in /Users/andrewkoumoudjian/Documents/QUARCC/bunting.

Initialize the native Rust tRPC and authorized NBC implementation program. Start with Sprint 0 only from docs/plans/native-rust-trpc-nbc-sprints.md; do not collapse later semantic sprints into the first PR.

Canonical decisions:
- Read AGENTS.md and every nearest scoped AGENTS.md before edits.
- Read docs/architecture.md, docs/implementation-pathway.md, ADR 0013, ADR 0014, ADR 0016, ADR 0017, docs/reference-functionality-audit.md, docs/reference-adoption.md, docs/repository-reorganization.md, docs/ports/nbc-simulation.md, docs/ports/nbc-evidence-manifest.v1.json, schemas/trpc/bunting.v1.json, and the full sprint plan.
- The only public application API is tRPC. There is no REST router or REST fallback.
- The production API is one native Rust workers-rs Worker. Do not add a TypeScript gateway.
- Implement the pinned tRPC wire subset in Rust and prove it differentially against the official tRPC server/client reference. Do not claim native TypeScript AppRouter inference.
- tRPC queries/mutations do not use a Durable Object. ADR 0016 permits a Rust RunStreamCoordinator only after the committed-subscription gate, and it can never own commands, matching, the event log or origin truth.
- Commit origin state before acknowledgement, cache publication or stream publication.
- The pinned NBC JAR is authorized under ADR 0017 for execution, archive extraction, bytecode inspection/decompilation, Rust translation and redistribution.
- NBC is a complete market engine under packages/nbc-market-engine, not scenarios or agent helpers. Every translated module needs JAR class/resource provenance and differential evidence.
- OrderBook-rs 0.10.3 remains the default matcher. Reuse it in NBC only when evidence matches; never add another generic Bunting CLOB.
- FIX compatibility is a native client bridge over tRPC, not a Worker FIX endpoint.

Environment and branch:
1. Fetch/prune origin and verify a clean main at the current GitHub head.
2. Create feat/native-rust-trpc-s0-reference-contract.
3. Verify Rust 1.88.0, rustfmt, clippy and wasm32-unknown-unknown. Do not use CodeGraph.
4. Check disk space before builds; keep target/, generated Worker build/, decompiler output and out/ untracked.

Sprint 0 deliverables:
1. Intake the official tRPC 11.18.0 source at git head 6aec1578a899df50a17e4e78d5512a099b574c18 as a development-only conformance reference, following the repository audit/adoption and exact-pin rules.
2. Record MIT license, package manifests, selected Fetch adapter, HTTP batch client, HTTP subscription client, error formatting, transformer behavior and Worker/Wasm impact.
3. Update the functionality audit and adoption disposition before using tRPC as an oracle.
4. Expand schemas/trpc/bunting.v1.json into an exact supported wire contract: methods, path encoding, query input, POST body, query batching, mutation-batch rejection, content types, response/error envelopes, HTTP subscription events, cancellation and unsupported features.
5. Add versioned golden fixtures emitted by the official tRPC implementation for every supported and rejected case. Keep official TypeScript runtime code in tests/tooling only; add no production gateway.
6. Add a contract validator using existing project tooling, with no empty Rust package scaffolds.
7. Update docs/plans/native-rust-trpc-nbc-sprints.md only if source evidence requires a concrete correction; explain every divergence.

Validation:
- Verify reference pins and licenses.
- Parse every JSON schema/fixture.
- Run cargo metadata --locked --format-version 1 --no-deps.
- Run cargo fmt --all --check.
- Run cargo clippy --locked --workspace --all-targets -- -D warnings.
- Run cargo test --locked --workspace.
- Run cargo tree --locked -p bunting-orderbook and assert orderbook-rs v0.10.3.
- Run cargo check --locked --workspace --target wasm32-unknown-unknown.
- Run git diff --check and a focused stale-contract/path search.

Commit, push, open a review PR, and report exactly what Sprint 1 may now assume. Do not implement the Worker cutover, FIX bridge, Durable Object or NBC translation in Sprint 0.
```
