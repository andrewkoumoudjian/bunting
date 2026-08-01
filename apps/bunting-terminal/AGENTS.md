# Bunting GPUI terminal instructions

Scope: `apps/bunting-terminal/**`.

- This application is a presentation and operator-input client. It must never match orders, mutate authoritative account state, infer fills, or bypass the existing Bunting FIX/session/application contracts.
- Reuse `bunting_tui::client` for profiles, credentials, transport ownership, bounded channels, FIX recovery, projections, and outbound commands.
- Passwords and tokens must remain environment-only. Never persist or render credential values. Raw FIX diagnostics must remain redacted by the shared client reducer.
- The outer application owns one native GPUI window. Market tools are movable, resizable internal panels on the workspace canvas; adding a panel must preserve focus, z-order, close/minimize, and bounded rendering behavior.
- The entire Apache-2.0 `longbridge/gpui-component@88f102d13654fe25aa2fede076274b6b751a3704` `crates/ui/src` component surface is approved for direct dependency, verbatim reuse, or close adaptation when it improves the terminal. Prefer its title bar, status bar, dock, resizable, table, chart, plot, input, button, menu, popover, dialog, notification, sidebar, skeleton, progress, tabs, tooltips, badges, lists, trees, and theme APIs over new local equivalents.
- Keep custom GPUI code limited to Bunting-specific market composition and the internal floating-window manager when the component library has no equivalent.
- Verbatim or close source adaptation is allowed only from a license-compatible source whose exact repository, commit, path, license, retained behavior, and divergence are recorded in `THIRD_PARTY_NOTICES.md` and `docs/gpui-terminal-reference-inventory.md`.
- Zed's `crates/ui` is GPL-3.0-or-later. Coop and Zorite are GPL-3.0-or-later, and Rox is AGPL-3.0. They are interaction and architecture references only unless Bunting is deliberately relicensed; do not copy their implementation into this Apache-2.0 application.
- Keep market collections bounded on screen. Never let a live feed create an unbounded GPUI element tree.
- Every order, cancel, tender, run-control, reset, logout, or reconnect action must report local queue status and remain subject to server-side identity, role, sequence, risk, and lifecycle validation.
- Use the dedicated macOS GPUI workflow only as diagnostic evidence. GitHub Actions is not a delivery gate for this task; continue implementation and report what was or was not verified.
