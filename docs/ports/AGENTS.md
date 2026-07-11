# Port documentation instructions

Every file in this directory is a provenance and divergence record for a port or behavior-derived rewrite.

Required sections:

- source repository, exact commit and source paths;
- license and notices that must be preserved;
- observed behavior and confidence level;
- behavior retained and rejected;
- Bunting destination crates/modules;
- unit, property, differential, replay and Wasm tests;
- copied versus translated versus clean-room behavior-derived code;
- unresolved questions and update history.

Do not claim equivalence unless a reproducible fixture proves it. Do not copy code before its license is recorded. Do not edit files inside upstream submodules.
