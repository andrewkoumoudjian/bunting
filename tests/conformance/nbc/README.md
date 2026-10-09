# NBC evidence fixtures (provenance only)

These files are JAR-observed captures pinned by hash in
[`docs/ports/nbc-evidence-manifest.v1.json`](../../../docs/ports/nbc-evidence-manifest.v1.json).
They are kept as evidence of what the NBC reference simulator did.

Since ADR 0032 (implemented in the slice-11 commit recorded in
`docs/implementation-log/`), **no Bunting code or test consumes them** and
Bunting does not target NBC compatibility. Do not add tests, parsers or
engine behavior that read these files. An NBC-inspired feature is specified
as Bunting-native behavior with its own fixtures.
