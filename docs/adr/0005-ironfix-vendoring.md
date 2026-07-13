# ADR 0005: Adapt a pinned, transport-neutral subset of IronFix

- Status: Superseded in Worker scope by ADR 0015; native FIX bridge gates retained
- Date: 2026-07-11

## Context

Bunting needs FIX parsing, validation, dictionaries, serialization and session behavior in both a Wasm Worker and a native Rust bridge.

IronFix is modular and separates core, dictionary, tag-value, session, store, transport and engine crates. Its core and tag-value crates are relatively dependency-light, but the current session and transport layers use Tokio and native synchronization. Fixer is broader but tightly coupled to Tokio, filesystem and native networking. FerrumFIX has useful layering but explicitly warns against production use before 1.0.

No candidate should become a hidden dependency across the market kernel.

## Decision

Pin IronFix at commit `6ac17a37f7c4efbdfe97a06f428809733d88b66b` and evaluate/adapt only:

- core types and errors;
- FIX dictionaries needed for FIX 4.4;
- zero-copy tag-value framing and serialization;
- selected transport-independent session algorithms;
- generated FIX 4.4 field and message definitions where licensing permits.

Create project-owned crates:

- `bunting-simfix-wire`;
- `bunting-simfix-session`;
- `bunting-simfix-mapping`.

Project-owned traits define clock, transport and message store. Neither matching nor risk depends directly on IronFix APIs.

Do not include in the Worker graph:

- `ironfix-transport`;
- Tokio socket code;
- native TLS;
- filesystem stores;
- thread-oriented synchronization;
- the high-level native engine façade.

Use Fixer as a native interoperability oracle and fallback implementation for the bridge if the IronFix-derived session code fails conformance gates. Use FerrumFIX only as a design and test reference.

## Mandatory gates before production use

1. exact upstream commit and file inventory;
2. MIT license and third-party notices preserved;
3. FIX specification/dictionary data licensing reviewed separately;
4. selected crates compile for `wasm32-unknown-unknown`;
5. dependency audit and minimal feature selection;
6. malformed-frame fuzzing;
7. BodyLength and CheckSum golden tests;
8. repeating-group tests;
9. session conformance tests;
10. interoperability with Fixer and another independent implementation;
11. benchmarks for allocation, size and parsing latency;
12. local patches documented in `vendor/ironfix/PATCHES.md`.

If the gates fail, retain the project-owned interfaces and replace the implementation without changing the market kernel.

## Consequences

Positive:

- reuse of focused protocol work;
- one shared wire model for Worker and bridge;
- replaceability through local traits;
- reduced time compared with implementing every FIX field parser from scratch.

Negative:

- vendoring creates update and security-review responsibility;
- IronFix is young and may contain incomplete behavior;
- generated specification data can have separate licensing constraints;
- session extraction requires careful redesign.

## Rejected alternatives

### Depend on the complete IronFix engine

Rejected because Tokio/native transport dependencies conflict with the Worker runtime and over-couple Bunting.

### Depend on the complete Fixer engine in the Worker

Rejected because its native runtime, filesystem and networking dependencies are unsuitable for the Wasm Worker graph.

### Write every FIX component from scratch immediately

Rejected because it discards useful tested parsing work, but remains a fallback if conformance gates fail.

## References

- `ref/ironfix` at `6ac17a37f7c4efbdfe97a06f428809733d88b66b`
- `ref/fixer` at `c1c27c3287d6f275a9c33122cc2af063de7c5a08`
- `ref/ferrumfix` at `ca2bbe4c6461108646f35f7cc9245bf1848ec368`
