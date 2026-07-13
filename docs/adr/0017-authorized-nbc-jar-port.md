# ADR 0017: Authorized NBC JAR translation and Rust market-engine port

- Status: Accepted
- Date: 2026-07-12
- Authority: explicit project-owner authorization recorded on 2026-07-12
- Amends: ADR 0014 and the NBC evidence restrictions

## Context

The direct `ref/nbc_engine` tree contains configuration, scenarios and external application documentation but no Java source or named JAR. The pinned `ref/nbc-hft-simulation` tree at `35b8050546679547dc737198ea13aa0ec8ed7db8` contains `app/exchange-simulator-0.0.1-SNAPSHOT.jar` with SHA-256 `80afc2816970b2538dcaff808008bfebdce5426ac248c074859626605547e254`.

Earlier documents treated that JAR as opaque evidence and prohibited decompilation because ownership and authority were unresolved. The project owner has now explicitly authorized bytecode inspection, decompilation, translation into Rust and redistribution for the Bunting NBC port.

Authorization does not turn inferred behavior into observed behavior or prove that the JAR was built from the direct snapshot. Provenance, license notices, translation records and compatibility evidence remain required.

## Decision

Select the pinned JAR above as the NBC reference runtime and authorized source artifact for the `nbc-v1` Rust market-engine port.

Authorized work includes:

- executing the JAR in an isolated environment and capturing external traces;
- listing and extracting archive contents;
- decompiling and inspecting bytecode and resources;
- translating implementation behavior into independently reviewed Rust;
- retaining necessary protocol/configuration compatibility material;
- redistributing the authorized JAR-derived Rust port and required notices;
- retaining the reference JAR under `ref/` as provenance evidence.

Every translated module records the JAR hash, class/resource path, translation method, retained behavior, intentional divergence and reviewer. Generated/decompiled intermediate text is evidence, not production source, and is stored only in an approved ignored evidence workspace unless a later source-adoption decision explicitly tracks it.

The destination is one coherent `packages/nbc-market-engine` package implementing venue authority: run lifecycle, configuration, logical step advancement, order processing, public/private market data, agents, scoring and Bunting recovery. It is not a scenario helper or participant client.

Use shared `market-types`, `market-events`, ledger/risk and `packages/orderbook` where differential evidence shows compatible behavior. OrderBook-rs remains the current default matcher. If the authorized JAR proves incompatible matching semantics, an NBC-specific implementation is allowed only behind the NBC engine boundary with a documented behavior requirement and differential tests; it must not become a second generic Bunting CLOB.

Compatibility claims are layered:

- exact external compatibility requires reproducible JAR-versus-Rust fixtures;
- translated internal behavior cites bytecode evidence;
- Bunting recovery, state hashing, checked numerics and bounded buffers remain explicitly Bunting-added unless the JAR proves equivalents;
- unresolved or nondeterministic behavior remains labeled until a test resolves it.

## Consequences

The NBC package is no longer blocked on clean-room authority or limited to black-box contracts. Bytecode-derived implementation work can proceed as a first-class market-engine sprint, and the authorized port may be redistributed with its provenance record.

The project still carries translation and maintenance responsibility. The direct tree/JAR relationship, original build process and upstream license metadata remain unresolved facts and must not be misstated.

## Rejected alternatives

### Treat the JAR only as a black-box oracle

Rejected because explicit project-owner authority now permits a closer Rust port and the JAR contains the selected implementation evidence.

### Mechanically drop decompiled Java into production

Rejected because Bunting requires Rust, checked fixed-point boundaries, deterministic recovery, Wasm compatibility and reviewable source provenance.

### Reclassify NBC as scenario data

Rejected because the authorized JAR is a complete market-engine source artifact and ADR 0014 already assigns NBC venue authority.

## Validation

- the gitlink and JAR SHA-256 match the evidence manifest before every extraction;
- an inventory records every class/resource and its translation disposition;
- external REST/WebSocket/DONE fixtures run against the JAR and Rust port;
- differential tests cover matching, scheduler order, partial fills, cancellation races, agents, scoring and termination behavior as discovered;
- deterministic replay and state hashes cover Bunting-added recovery;
- translated modules retain file-level provenance and divergence records;
- native and Worker-target tests pass for the selected deployment graph;
- redistribution bundles contain the authorization record, provenance manifest and required notices.

## Operational impact

JAR execution and decompilation occur in an isolated, reproducible tool environment with no production credentials. Extracted intermediates, traces and generated reports have bounded locations and cleanup rules. The JAR never becomes a production runtime dependency.

## Security impact

Treat the JAR and decompiler output as untrusted input. Run tools without credentials or network access where practical, scan archive paths before extraction, bound output, and never execute translated code in production before review and tests.

## References

- `docs/ports/nbc-evidence-manifest.v1.json`
- `docs/ports/nbc-simulation.md`
- `tests/fixtures/nbc/external-contract-manifest.v1.json`
- ADR 0014
