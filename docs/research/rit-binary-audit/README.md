# RIT binary audit

Status: initial static extraction and feature inventory complete

This directory records a clean-room, static-only audit of the two RIT installers supplied on 2026-07-13. The proprietary installers and extracted payloads remain outside Git at `/Users/andrewkoumoudjian/Documents/QUARCC/bunting-rit-analysis-20260713`; only hashes, metadata, behavioral names, protocol surfaces, and independently written interpretations are tracked here.

The extraction recovered all 38 files declared by the MSI `File` tables: 35 from the RIT User Application cabinet and three from the RTD/API Link cabinet. It also recovered all 21 independently addressable MSI streams. No payload or stream extraction failed.

Evidence labels follow the repository-wide vocabulary:

- **observed** means directly present in an MSI table, extracted payload, PE/CLR metadata, static IL literal, configuration file, or existing pinned reference record;
- **inferred** means a reasoned behavioral interpretation that the static evidence does not fully prove;
- **Bunting-added** means a new architecture, safety, determinism, or recovery requirement;
- **unresolved** means the available bytes do not establish exact behavior;
- **prohibited to copy** covers proprietary implementation text and resources that may be inspected for behavior but are not production source.

Start with [`source-manifest.md`](source-manifest.md), then [`extraction-method.md`](extraction-method.md), [`protocol-inventory.md`](protocol-inventory.md), and [`market-feature-ledger.md`](market-feature-ledger.md). [`inventory.json`](inventory.json) is the machine-readable hash and payload record; its `generated_from` path identifies the local ignored evidence workspace.

No installer, executable, DLL, service, or DLL entry point was run. Dynamic claims remain unresolved until separately authorized in a disposable environment.
