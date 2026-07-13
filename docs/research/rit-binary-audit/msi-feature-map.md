# MSI feature and component map

Both installers expose one MSI feature named `DefaultFeature`; the useful decomposition is therefore component-to-installed-payload rather than user-selectable feature hierarchy. The exact component IDs, file IDs, declared sizes, versions, sequences, installed paths, extraction paths, and hashes are in [`inventory.json`](inventory.json).

## RIT User Application 1.8.456

- `DefaultFeature` maps 37 components: 35 file-bearing components, one product component, and one desktop-folder registry component.
- The embedded cabinet `_CCE6C4C34566E21FEFAF3FE1EE72F84F` declares and yielded 35 files.
- Root payloads are `Client.exe`, `Client.exe.config`, `TTS.Common.dll`, 19 framework/application dependencies, `rotman.ico`, and related managed libraries.
- `TS/` contains four TeamSpeak client/server native DLLs; `TS/soundbackends/` contains four 32/64-bit DirectSound and Windows Audio Session native DLLs.
- Two shortcuts launch `Client.exe`; no Windows service is installed.
- The only product registry value records the selected desktop folder.
- Custom actions set the installation directory and run Visual Studio setup/runtime checks; none implement market behavior.

## RIT2 RTD/API Link x64 0.0.15

- `DefaultFeature` maps four components: `RIT2.dll`, `TTS.RTD.dll`, `RegisterDLL.dll`, and the RTD COM registration registry component.
- The embedded cabinet `_FF0154C159F4136F8EF77F8FEFA08BA5` declares and yielded all three files.
- Registry rows register managed class `TTS.RtdServer` as ProgID `RIT2.RTD`, CLSID `{F5A08459-D202-4606-87B0-9EEB5F70A159}`, in-process server `mscoree.dll`, threading model `Both`, CLR runtime `v4.0.30319`, and assembly version `1.0.0.5`.
- Managed install/uninstall actions call `RegisterDLL.dll`; there is no service, shortcut, file association, or type-library table row.

## Stream map

The User Application MSI contains five setup UI binaries/resources, three icons, one CMS signature, one extended signature stream, one summary stream, and one cabinet. The RTD/API Link MSI contains six setup/runtime helper streams, one CMS signature, one summary stream, and one cabinet. Every stream is individually hashed in [`inventory.json`](inventory.json).
