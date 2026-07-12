# NBC scenario instructions

This directory will contain canonical, human-reviewable Bunting scenario documents derived from the NBC source catalog.

Do not copy imported JSON blindly. Every scenario must use the strict `scenario-schema`, exact decimal strings, explicit logical-time units, versioned PRNG derivation and provenance fields including source path and Git blob SHA.

A scenario may reference only implemented and versioned agent models. Unknown or unresolved legacy parameters must be retained as provenance metadata and cannot silently drive behavior.

Legacy names and `DONE` lockstep semantics belong in `crates/protocol-legacy-nbc`, not in canonical scenario files.