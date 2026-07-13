# NBC evidence tooling

These scripts inspect only the ADR 0017-authorized JAR and write generated
artifacts beneath the ignored `out/nbc-evidence/` directory. They fail closed
unless both the committed gitlink and JAR SHA-256 match the evidence manifest.

Run from the repository root:

```sh
tools/nbc-evidence/inventory.sh
tools/nbc-evidence/inspect-bytecode.sh
tools/nbc-evidence/capture-runtime.sh
tools/nbc-evidence/validate-committed-evidence.sh
```

`inventory.sh` records the archive inventory and per-entry hashes without
extracting the archive. `inspect-bytecode.sh` extracts only a fixed allowlist of
NBC application classes and resources after rejecting unsafe archive paths;
`javap` output remains ignored evidence. `capture-runtime.sh` starts the JAR on
loopback with an empty environment and temporary HOME, captures bounded startup
and HTTP observations, then stops it. It never supplies credentials and does
not claim that a failed startup proves protocol behavior.
`validate-committed-evidence.sh` re-hashes every committed class/resource row
directly from the pinned JAR and validates the JSON fixtures.
