# Payload inventory

The machine-readable inventory is authoritative for exact size, SHA-256, SHA-512, file type, architecture, PE imports/exports/resources/version metadata, signature directory presence, installed path, component, and local extraction path. This page groups the payloads by role without reproducing proprietary bytes.

## Product-owned managed payloads

| Payload | SHA-256 | Observed role |
|---|---|---|
| `Client.exe` 1.1.8.456 | `ec47c4f926427b82181421d26d884abdb59ba1c7aedb352de4c6931efc9cbbd0` | 32-bit .NET 4.8 RIT client; REST server, RTD data provider, WCF client contracts, market UI, synchronization models |
| `TTS.Common.dll` 1.1.8.456 | `311a384f4c1af43874f1b8270fd8da8997658839e7f100c0483d11000e709940` | shared domain records, enums, WCF contracts, scenario parameters, sync/update messages |
| `RIT2.dll` 1.0.0.5 | `bc8fd70e4ac9780dd92f2772ab9857a848c248920ddfaa955d8ec25c5639163a` | x64 managed VBA/API facade over the local named-pipe API endpoint |
| `TTS.RTD.dll` 1.0.0.5 | `a820c2439baaef518739f5ea6d671f3b650763e04b065fd774159a595f6457fe` | Excel RTD COM server over the local named-pipe RTD endpoint |
| `RegisterDLL.dll` 1.0.0.0 | `4e537000e246e2940221673de6d1f9ac29b4d1468a01020976ad136fb0dae59b` | managed COM registration helper; installation-only |

## Framework and application dependencies

The User Application cabinet also contains Newtonsoft.Json, protobuf-net, Ciloci.Flee, bcParser.NET, Janus GridEX/Common/Data, NAudio Core/MIDI/Wasapi/WinMM/ASIO/WinForms, TeamSpeak.Sdk, selected .NET compatibility assemblies, and `netstandard.dll`. These dependencies support JSON, protobuf serialization, expressions, grids/charts, audio, voice chat, HTTP, compression, registry access, and framework compatibility; they do not independently prove venue semantics.

## Native payloads and resources

Eight native TeamSpeak/audio payloads are present in paired x86/x64 forms: `ts3client`, `ts3server`, DirectSound, and Windows Audio Session backends. `Client.exe.config` supplies local ports and user settings; `rotman.ico` is a UI resource. These are adapter/UI or installation concerns, not market-engine implementations.

## Classification totals

| Class | Count |
|---|---:|
| Managed PE payloads | 28 |
| Native PE payloads | 8 |
| Configuration/resource payloads | 2 |
| Total | 38 |

All exact SHA-512 values are intentionally kept in [`inventory.json`](inventory.json) to avoid duplicating a 38-row digest table in prose.
