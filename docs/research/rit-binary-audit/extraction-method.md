# Extraction method

## Safety boundary

All work was static. The installers and payloads were never launched, registered, loaded, or invoked. No service installation, COM activation, managed entry point, native entry point, network connection, or installer custom action occurred.

## Toolchain

- `msiinfo` exported the summary information, all table names, every present table in IDT form, and every addressable stream.
- `msiextract` listed and expanded both embedded cabinets while preserving installer paths.
- `shasum` produced SHA-256 and SHA-512 digests for originals, payloads, and streams.
- macOS `file` classified containers, PE architecture, managed assemblies, resources, and configuration data.
- `pefile 2024.8.26` recorded PE headers, architecture, CLR and Authenticode directories, imports, exports, resources, and version metadata.
- `dnfile 0.18.0` and `dncil 1.0.2` recorded CLR assemblies, namespaces, types, methods, fields, properties, events, resources, references, signatures, and method-local static string/integer literals.
- OpenSSL decoded the embedded PKCS#7 certificate chains.

The analysis script and Python dependencies live only in the external evidence workspace. [`inventory.json`](inventory.json) identifies the exact generated artifact root.

## Procedure

1. Enumerate the source corpus, record metadata and hashes, and preserve exact copies outside Git.
2. Export each MSI summary, table list, stream list, and all present tables.
3. Extract each embedded cabinet with `msiextract`; independently extract every named stream with `msiinfo extract`.
4. Join `Feature`, `FeatureComponents`, `Component`, `Directory`, and `File` tables into an installer-feature-to-payload map.
5. Verify every `File` row resolves to an extracted payload and that cabinet file counts equal 35 and three respectively.
6. Hash and classify every payload and stream; parse all PE and CLR files without loading them.
7. Extract static strings and IL literals, then search the complete output for market, run, account, risk, scenario, protocol, history, news, tender, asset, and transport concepts.
8. Record behavioral interpretations separately from observed evidence and list remaining unknowns in [`unresolved-evidence.md`](unresolved-evidence.md).

## Completeness result

| Check | Result |
|---|---|
| Source files enumerated | 3 of 3 |
| MSI tables exported | 84 per installer, including empty standard tables |
| Addressable streams extracted | 12 of 12 User Application; 9 of 9 RTD/API Link |
| Cabinet payloads extracted | 35 of 35 User Application; 3 of 3 RTD/API Link |
| Managed payloads parsed | 28 of 28 |
| Native PE payloads parsed | 8 of 8 |
| Non-PE payloads classified | 2 of 2 |
| Extraction or parser failures | 0 |

The MSI files contain no embedded transforms, services, ODBC definitions, environment changes, file extensions, MIME registrations, type-library table rows, or MSI `Class`/`ProgId` rows. The RTD COM class is instead registered through explicit `Registry` rows and a managed installer action.
