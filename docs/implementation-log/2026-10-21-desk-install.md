# Desktop app install bundles (2026-10-21)

Status: **Current.** Owner, 2026-10-21: "please make the installation easier
(maybe just drop the app in apps, for mac at least)".

## What changed

- **The app opens a market with no flags.**
  - `bunting-desk` with no `--server` joins the local practice venue, as
    `--local` did. A double-click from Finder, a desktop launcher or
    Explorer passes no arguments.
  - `--participant`, `--venue-dir` and `--server-bin` now conflict with
    `--server` instead of requiring `--local`.
- **Errors reach the screen.** A launch with no terminal shows why the
  venue could not be reached or started, in a window with a Quit button
  (`src/startup.rs`). The terminal still gets the message, and the exit code
  is still 2.
- **Closing the last window quits the app** on every platform. On macOS the
  app no longer runs on with no window while holding a seat in the local
  venue.
- **Title bar fix (found live).** On a narrow window, "Kill all orders"
  overlapped the palette field's Ctrl+K hint. The field now shrinks first,
  and the button never shrinks.
- **Release bundles** (`tools/bundle_desk.sh`, the new `bundle` job in
  `release.yml`, files in `apps/bunting-desk/packaging/`). Each bundle puts
  `bunting` beside the app:
  - macOS: `Bunting-<tag>-macOS.dmg`, a universal (`lipo`) `Bunting.app`
    with an icon, signed ad hoc, and an Applications link to drag onto;
  - Windows: `bunting-desk-<tag>-x86_64-pc-windows-msvc.zip`;
  - Linux: `bunting-desk-<tag>-<target>.tar.gz`, with a launcher entry and
    an icon.
  - The bundle job installs the app from the disk image on a macOS runner,
    opens it with `open` (no arguments), waits for the practice venue to
    start from the bundled `bunting`, then stops the app and checks that the
    venue stops too.
  - `workflow_dispatch` runs everything except the container image and the
    release as a dry run on a branch.
- **`install.sh` installs the app.** It is always installed on macOS,
  into `/Applications` (or `~/Applications`). On Linux it is installed when
  a display is present: `bunting-desk` into the install directory, with a
  launcher entry and an icon. `BUNTING_DESK=0` or `1` overrides the choice.
  Releases without app bundles are skipped with a message.
- **The app icon:** a string of pennants over a rising price line
  (`packaging/bunting.svg`, Bunting-added).

## Checks

- **Live** (Linux x86_64, release build, headless Hyprland output):
  - With no flags and `bunting` beside it, the app started the practice
    venue and joined as Participant 1. A second instance joined as
    Participant 2.
  - With no `bunting` anywhere, it showed the error window, and Quit exited.
  - Closing the desk window quit the app and stopped the venue.
- **`install.sh`**, run against a local release directory
  (`BUNTING_DOWNLOAD_BASE=file://…`):
  - with a display, it installed the app, its launcher entry and its icon;
  - without a display, it skipped the app;
  - with a release that lists no app bundle, it printed a message and
    exited 0.
- **`tools/check_desk.sh`:** 119 tests passed, 0 failed.
- **Released as `v0.2.0`** (owner, 2026-10-21: "make the release real release"): the desk, `bunting` and `bunting-trader` report 0.2.0, and a plain version tag is published as the latest release so `install.sh` finds it.
- **Not verified locally:** the macOS and Windows bundles. They are built
  and checked only on GitHub's runners, by the bundle job.

## After publishing v0.2.0

- **Installer bug found against the live release.** The publish job wrote
  `SHA256SUMS` entries as `./name`, so `install.sh` reported "Release
  checksum … is missing". This was also true of `v0.2.0-preview.1`: the
  one-line installer had never worked against a published release.
  - The `v0.2.0` `SHA256SUMS` asset was re-uploaded with bare names (same
    hashes).
  - The workflow now writes bare names.
  - `install.sh` accepts both forms.
- **Verified** with the installer from `main` against the live release
  (into a scratch prefix):
  - it installed `bunting`, `bunting-trader`, `bunting-desk 0.2.0`, the
    launcher entry and the icon;
  - the installed app, opened with no flags, started the practice venue
    and joined as Participant 1;
  - closing it stopped the venue.
  - `v0.2.0-preview.1` also installs with the fixed installer.
- `bunting --version` is not a recognised argument (pre-existing). Open.

## Remaining

- The macOS app is not notarized. A disk image downloaded in a browser
  needs **Open Anyway** on first launch, while `install.sh` avoids that step.
  Notarization needs the owner's Apple Developer ID.
- Windows opens a console window beside the app.
