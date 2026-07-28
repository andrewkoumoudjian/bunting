# ADR 0026: Language bindings, concrete façade and FFI lint boundaries

- Status: Accepted
- Date: 2026-07-28
- Depends on: ADR 0022

## Context

`bunting-rs` currently re-exports generic application types whose origin-store and cache parameters cannot cross a stable foreign-function boundary. The workspace also forbids unsafe Rust through inherited lints. A C ABI necessarily contains a reviewed unsafe boundary, while PyO3 and cxx macro expansions contain generated unsafe code that cannot override an inherited `unsafe_code = "forbid"`.

Competition participants already have a language-neutral FIX/TCP boundary. Native bindings serve embedding, practice, scoring and tooling rather than replacing FIX as the event protocol.

## Decision

`bunting-rs` owns one concrete, non-generic façade over selected host-neutral store and cache implementations. Rust, C, Python and C++ bindings expose that façade rather than independently wrapping engine internals.

Binding crates live under `bindings/` and may declare explicit per-crate `[lints]` tables instead of inheriting the workspace Rust lint table. `bunting-ffi` permits unsafe code only in a small reviewed C ABI module with opaque handles, checked error outputs and explicit ownership/free functions. `bunting-py` uses PyO3's stable ABI support. `bunting-cpp` uses cxx with opaque Rust handles and shared value structs.

All non-binding packages continue to inherit the workspace lint policy, including `unsafe_code = "forbid"`. Generated headers and bridge output are release artifacts unless a consuming build contract requires checked-in output.

## Consequences

One façade keeps semantics and errors aligned across languages. Binding crates receive a narrow exception to lint inheritance, increasing their review and test burden while preventing an unsafe carve-out from spreading into the engine.

The façade must own resources with explicit lifetimes and avoid callbacks or borrowed data across FFI. Foreign runtimes receive copied, versioned value objects and deterministic serialized results.

## Rejected alternatives

### Relax `unsafe_code` for the whole workspace

Rejected because the engine and protocol packages do not need unsafe Rust and benefit from the stronger invariant.

### Wrap the generic application service directly

Rejected because Rust generics and borrowed lifetimes do not define a stable C, Python or C++ ABI.

### Make Python and C++ wrap the C header

Rejected because direct PyO3 and cxx bindings provide safer ownership and native error mapping while still sharing the same Rust façade.

## Validation

- the façade has no public generic store or cache parameters;
- each binding declares and documents its own lint table;
- unsafe code is absent outside the explicitly reviewed C ABI and generated binding expansions;
- Rust, C, Python and C++ replay the same golden archive to identical canonical bytes;
- allocation and error paths have leak and double-free tests;
- bindings do not introduce Worker dependencies into `bunting-rs` or `packages/`.

## Operational impact

Release automation builds and tests bindings separately from the default native venue. ABI and archive-schema compatibility are versioned and published with each binding artifact.

## Security impact

Opaque handles, bounded inputs, explicit frees and fail-closed error mapping minimize memory-safety and resource-exhaustion risks. No binding receives direct mutable access to engine internals.

## References

- [`0022-native-competition-venue-and-publication-worker.md`](0022-native-competition-venue-and-publication-worker.md)
- [`../streamlining-audit.md`](../streamlining-audit.md)
- [`../../bunting-rs`](../../bunting-rs)
