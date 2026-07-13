# Source manifest

## Input corpus

The supplied corpus was found under repository-local `temp/`, not filesystem-root `/temp/`. The files were treated as read-only and copied byte-for-byte to the external analysis workspace before extraction.

| Source | Bytes | Source modification time | SHA-256 | SHA-512 | Classification |
|---|---:|---|---|---|---|
| `temp/RIT.User.Application-1.8.456.msi` | 9,030,144 | 2026-07-13 14:59:27 -0400 | `20fa6853460f6990f4a1491f5e0ffe5077cf5d8792be9df63c0258aabf3d8c67` | `b631d67f0e1d00805cb9fa9d4a7d38e08ae8ce2291cecf39d436eafca8f45579ef94020ed239d584d601954fb7dc533eeb27f148a9b8eda08a99be5935f370be` | MSI, RIT User Application 1.8.456 |
| `temp/RIT2.RTD.API.Link.x64-0.0.15.msi` | 650,240 | 2026-07-13 14:58:48 -0400 | `e9fc53a2ac7842ef0a96d7ec36cf5eb54f25ce49dd83c54c633ccf930111950f` | `fb8413af3e04ba2a611ed618b1288abc06b32e834c6562575b65788422e35437be268261c99a648581610a002d08a55f0be06912acafd398372f0f29365a2022` | MSI, x64 RTD/API Link 0.0.15 |
| `temp/.DS_Store` | 6,148 | 2026-07-13 15:00:37 -0400 | `99e474a3ed1fc827f95c0620d070bbe3b95f11fd416819104b6c75adfc1b5012` | `eee1ef5eeaaba6d044808507512a2d1d3c6961498d9de10108932dec54bac18297d69408bcaa814cb7938b9c07219b15817f803f04b1617052abff0c05b21b22` | Finder metadata; not market-related |

## MSI identity and signatures

| Installer | Product | Product code | Upgrade code | Architecture | Embedded signature evidence |
|---|---|---|---|---|---|
| User Application | `RIT User Application` 1.8.456 | `{5B788E0F-C5F2-41BB-8C99-967A9A16466C}` | `{24D7EAC5-4647-4D71-ACF0-A2AAAFBA41AC}` | `Intel;1033` | CMS stream parses; signer `306W Inc.`; certificate validity 2025-02-18 through 2028-02-18; extended MSI signature stream present |
| RTD/API Link | `RIT 2.0 RTD API Link (x64)` 0.0.15 | `{19599623-65B7-48E0-85A3-F022CCA66085}` | `{0FF109A8-B96D-49B3-84AC-B40AE75F45AC}` | `x64;1033` | CMS stream parses; signer `306W Inc.`; signer certificate validity 2017-12-23 through 2021-12-23; timestamp certificate present |

The certificate chains were decoded successfully with OpenSSL. A Windows trust-policy verification was not run, so this records signature presence and certificate metadata rather than a current platform trust verdict.

## Preservation

The external workspace contains preserved originals, exported MSI tables, extracted streams, cabinets, payloads, hashes, CLR metadata, PE metadata, and static strings. It is intentionally outside the repository and is not a production dependency.
