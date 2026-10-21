# Desktop app packaging

Release packaging for `bunting-desk`, used by `tools/bundle_desk.sh` and
`install.sh`. Every bundle carries the `bunting` executable beside the app,
so opening the app with no flags starts or joins the local practice market.

- `bunting.svg`: the app icon (Bunting-added, 2026-10-21), and
  `bunting.png`, its 1024 px rendering (`rsvg-convert -w 1024 -h 1024
  bunting.svg -o bunting.png`). Re-render the PNG when the SVG changes.
- `macos/Info.plist`: the `Bunting.app` property list; `@VERSION@` and
  `@SHORT_VERSION@` are filled in by `tools/bundle_desk.sh`.
- `linux/bunting-desk.desktop`: the launcher entry; `install.sh` fills in
  `@EXEC@`. `StartupWMClass` matches the window's `app_id`.

The macOS app is signed ad hoc, not with a Developer ID, and is not
notarized. `install.sh` installs it without a quarantine flag, so it opens
directly; a `.dmg` downloaded in a browser needs **Open Anyway** in System
Settings → Privacy & Security on first launch.
