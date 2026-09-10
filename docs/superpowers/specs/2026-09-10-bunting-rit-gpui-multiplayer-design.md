# Bunting canonical RIT-style GPUI terminal and multiplayer design

Date: 2026-09-10
Status: approved design, implementation pending plan/review

## Goal

Make the native GPUI market terminal the canonical human-facing Bunting desktop interface. The product should preserve the information architecture and dense workstation mental model of Rotman Interactive Trader (RIT), while modernizing the interaction model and visual system with `gpui-kit`.

The Ratatui UI is retired as a user-facing product once parity is reached. Its reusable FIX transport, session, and projection logic is extracted into a UI-independent client package and remains shared infrastructure.

The exchange, origin, matching, risk, account, run-control, and competition state remain server-authoritative. The desktop application never implements a second matcher, ledger, fill model, account model, or competition authority.

## Decisions

1. `gpui-kit` is the required desktop UI dependency and the default source of GPUI, components, assets, platform integration, theming, tables, panes, overlays, and charts.
2. All graphical charting uses GPUI Kit chart/plot primitives. The desktop app contains no custom chart renderer and no Ratatui/ANSI chart implementation.
3. The default workspace follows the RIT workstation mental model rather than a generic dashboard or a Zed-editor layout translated literally.
4. Docking and resizing remain available, but the default geometry is deliberate and RIT-like. User customization is secondary to a strong default.
5. The current quote-derived pseudo-candles are removed from the canonical market-history surface. Historical charts consume authoritative server-backed trades/OHLC/time-and-sales once that projection exists.
6. Multiplayer collaboration is separated from market authority. Collaborative state may replicate workspace metadata, annotations, watchlists, chat, shared research, cursor/selection state, and similar non-authoritative artifacts; orders, fills, balances, positions, risk, run lifecycle, and market state never use a CRDT as their authority.
7. DeltaDB is an optional future collaboration backend, not a current hard dependency. The public Zed repository does not currently expose the DeltaDB implementation as a reusable public crate/repository that Bunting can safely adopt. Zed's existing collaboration/server crates are GPL-3.0-or-later and may be used as architectural reference only unless Bunting's licensing strategy is deliberately changed.
8. The collaboration layer therefore uses a Bunting-owned interface boundary so DeltaDB can be plugged in later if Zed publishes it under terms compatible with this project. The first implementation may use a permissively licensed CRDT/event-sync substrate or remain local-only until such a backend is selected.

## Product information architecture

The default desktop window is a dense, persistent trading workstation.

### Top market/run strip

The top strip shows the active run/scenario, instrument selector, market status, logical time/period, connection health, participant identity, and compact global actions. It is not a marketing header and does not consume dashboard-scale vertical space.

### Left market pane

The left region contains the order book and market depth as the primary scan surface, with tenders available as an adjacent tab where relevant. Depth rows use stable numeric columns, right-aligned prices/quantities, best-level emphasis, and keyboard-accessible actions.

### Center market pane

The center region contains the main market chart and instrument information. The chart is the largest default surface. Tabs may expose authoritative time-and-sales, historical bars, instrument metadata, and approved research overlays without obscuring the primary chart.

### Right trading/account pane

The right region contains the order ticket, position/account state, and risk/score. Order entry is always reachable without switching the entire workspace. Account and risk values are projections from committed server state.

### Bottom activity pane

The bottom region contains open orders, order history/fills, news, and FIX/session activity. It is resizable and collapsible, but defaults to enough height for meaningful activity review.

### Secondary modes

Research and competition workflows are implemented as tabs/panels and layout presets over the same workstation shell. Presets rearrange the same canonical panel set rather than replacing the product with unrelated dashboard screens.

## GPUI Kit architecture

The terminal becomes a normal `gpui-kit` application rather than independently pinning `gpui`, `gpui_platform`, old `gpui-component`, and component assets.

The application uses GPUI Kit components for:

- root/window shell and title/status bars;
- resizable regions, tabs, sidebars/panes, menus, dialogs, sheets, and notifications;
- data tables and virtualized lists for depth, orders, fills, positions, news, and diagnostics;
- inputs, selects, number inputs, buttons, command surfaces, and keyboard focus;
- all charts and plots;
- theme tokens, spacing, semantic status colors, and interaction states.

Application code owns Bunting-specific composition and presentation policy. Reusable behavior remains in the client/domain packages rather than being embedded in view rendering.

Repeated market rows use domain-derived stable element identities such as instrument/order/tender/event IDs. Rendering must not use list indexes as identity.

## Chart contract

All desktop charts use GPUI Kit chart/plot primitives. Chart data is a typed projection produced outside the view layer.

The chart data contract distinguishes at least:

- authoritative trades;
- OHLC bars derived by a server-owned/versioned bar policy;
- L1/L2 snapshots;
- participant P&L/account series;
- explicitly labeled analytical overlays.

Quote-derived synthetic candles cannot be labeled or presented as trade OHLC. If authoritative history is unavailable, the UI shows an explicit unavailable/loading state rather than fabricating history.

## Client extraction

The current `bunting-tui::client` module is extracted into a UI-independent package, tentatively `packages/bunting-client`.

That package owns:

- profile/config parsing needed by desktop/CLI clients;
- FIX socket/TLS/session lifecycle;
- authentication/logon construction;
- bounded I/O ownership and reconnect policy;
- recovery cursor and sequence tracking;
- redacted protocol diagnostics;
- typed participant/public projection reducers;
- command constructors and request correlation;
- transport errors and connection state.

It does not own presentation, local chart state, dock geometry, or account fallbacks.

`bunting-tui` may temporarily depend on the extracted package during migration. Once the GPUI terminal reaches functional parity and release validation, the Ratatui presentation is removed from canonical packaging and documentation.

## Server and competition correctness work

The UI cutover is gated by repair of the following correctness issues.

### Replay and archive completeness

Competition archives must represent the complete authoritative command history, not simulation commands only. Participant order commands, simulation/operator commands, committed ordering, venue-arrival ordering needed by the competition policy, and resulting canonical events must be replayable to the same final state hash.

The archive schema must make command kind explicit and versioned. Replay must reject missing, reordered, duplicated-with-conflict, or event-divergent histories.

### Local server readiness and diagnostics

The desktop launcher must stop probing readiness by opening the FIX participant socket. Readiness uses a versioned health/admin handshake that proves the peer is a compatible Bunting server without consuming a participant session.

Launcher failures return structured diagnostics containing phase, stable error code, human-readable detail, server log path, and process exit status where relevant. The UI surfaces the root cause rather than only `exit code 2`.

Discovered Wasmer executables are version-checked against the supported runtime range before launch. A listener on the expected port is not treated as Bunting merely because a TCP connection succeeds.

### Venue failure isolation

Malformed admin requests, bad FIX peers, TLS/terminator mismatches, participant disconnects, and per-session protocol errors must terminate or reject only the affected request/session. They must not bring down unrelated venue listeners or the scenario runtime.

The top-level supervisor reports fatal listener/runtime failures distinctly and continues independent services where policy permits.

### Authoritative reconnect accounting

Open-order limits and reconnect state are reconstructed from authoritative origin state rather than connection-local counters. A reconnect cannot reset or bypass a participant's venue limits.

Logical time used by participant commands is derived from the authoritative run/scheduling contract. Host wall time may timestamp transport diagnostics but cannot silently become simulation logical time.

### Cloudflare publication-only cleanup

The Worker contains publication/query/subscription behavior only. Legacy mutation execution, D1 command-authority paths, FIX Durable Object authority, and obsolete bindings/migrations are removed once no supported route depends on them.

The Worker may cache or publish committed server state, but it cannot become an alternate command authority.

### CI/toolchain repair

Canonical CI must install/build the WASIX toolchain using a bootstrap Rust version compatible with current `cargo-wasix` dependencies while preserving the engine/workspace toolchain contract. The dedicated GPUI workflow and canonical CI should not encode conflicting bootstrap assumptions.

## Authoritative market history

Before the chart is considered RIT-class, the server exposes bounded, versioned market-history projections for committed trades and OHLC/time-and-sales.

Bar construction is deterministic and server-owned. Its interval, boundary semantics, empty-bar policy, retention, reset behavior, and committed sequence are versioned and testable. The desktop client consumes these projections; it does not invent bar semantics.

## Multiplayer boundary

Bunting multiplayer has two separate meanings and they must remain separate in code.

### Exchange multiplayer

Multiple participants, teams, instructors, administrators, and built-in agents interact with one authoritative venue. This already belongs to Bunting's server/FIX/origin architecture. Commands are authenticated, ordered, committed, and replayable by the venue. No CRDT participates in market correctness.

### Workspace collaboration

Human collaborators may share non-authoritative workstation state. The collaboration subsystem can synchronize:

- shared watchlists and selected research sets;
- annotations and notes attached to instruments/events;
- chat/discussion threads;
- cursor/selection presence where useful;
- optional shared panel/layout state;
- shared research documents or strategy drafts;
- references from discussion to immutable Bunting event/command IDs.

The collaboration layer stores references to authoritative market facts, not copies that can overwrite them.

### Collaboration backend interface

A UI-independent `CollaborationBackend` boundary should expose operations such as join/leave workspace, subscribe to replicated document changes, publish a local document operation, presence updates, and durable snapshot/recovery metadata.

The domain model uses Bunting-owned types so the UI is independent of a specific CRDT vendor. A future `DeltaDbBackend` can implement this contract without changing exchange packages or the GPUI views.

If DeltaDB becomes publicly available under a compatible license, adoption requires a focused source/license/protocol audit before code is imported. Until then, Zed/Delta behavior is a design reference only.

## DeltaDB feasibility finding as of 2026-09-10

Zed publicly describes DeltaDB as a CRDT-based system that captures fine-grained operations, virtualizes worktrees, links edits to conversations, and replicates collaborative state in real time. The standalone Delta application is currently in private beta/early access.

Public issue logs refer to internal paths such as `crates/deltadb/src/client.rs`, but those sources are not present in the public `zed-industries/zed` default branch and no public `zed-industries` DeltaDB repository was found during this design audit.

Zed's currently public `collab` and `text` crates are GPL-3.0-or-later. Bunting therefore must not copy their implementation into the Apache-2.0 terminal as part of this change. Their architecture can inform boundaries and tests.

## State and data flow

1. The native client establishes a FIX session through the extracted client package.
2. The server authenticates the participant/role and remains the only command authority.
3. Public/private committed projections update the client reducer with committed sequence/cursor information.
4. GPUI entities own local view state and render typed projections.
5. GPUI Kit charts/tables consume immutable/retained view models, not transport messages directly.
6. Optional collaboration state flows through `CollaborationBackend` into separate GPUI entities. It may reference market IDs but cannot mutate market projections.
7. Local workspace geometry/preferences persist independently of server market state and collaboration state unless the user explicitly chooses a shared workspace policy.

## Error handling

Connection, protocol, authority, replay, collaboration, and local-launch errors remain separate typed categories. The terminal must never convert an unavailable authoritative projection into fabricated local state.

Disconnected/stale views retain the last committed sequence and visibly mark their state. Mutating actions are disabled or rejected when the client cannot prove the required session/run state.

Collaboration failure never blocks trading or corrupts market state. A collaboration disconnect degrades shared annotations/presence only.

## Testing strategy

### Engine/server

- archive round-trip includes participant and simulation commands;
- deterministic replay reproduces canonical events and final state hash;
- concurrent arrival/order tests prove the intended venue ordering policy;
- reconnect restores authoritative open-order accounting;
- malformed FIX/admin peers cannot terminate unrelated listeners;
- logical-time tests prove host time is not market authority;
- market-history golden vectors cover trades, bars, retention, and boundaries;
- Worker tests prove mutation/authority routes and bindings are absent.

### Client package

- FIX logon/recovery/reconnect fixtures;
- bounded queue/backpressure tests;
- redaction tests;
- projection reducer tests with gaps/reset/replay;
- duplicate/out-of-order transport behavior;
- no local fill/account fallback.

### GPUI terminal

Use GPUI Kit UI integration tests for real component interaction, focus, selection, tabs, resizable regions, order-entry actions, dialogs, and offline states.

- default RIT layout renders the required five regions;
- keyboard path covers instrument selection, order entry, submit/cancel, tab navigation, and command palette;
- stable row identity survives insert/reorder updates;
- chart accepts only typed authoritative history series for OHLC mode;
- disconnected and stale states are explicit;
- light/dark/custom theme tokens do not break hierarchy/alignment;
- representative window sizes and scale factors preserve alignment spines and minimum pane constraints.

### Collaboration

- two peers converge on the same non-authoritative shared document state;
- reconnect/replay converges after missed operations;
- presence loss does not alter document or market state;
- collaboration messages cannot submit market commands or mutate account/order projections;
- references to Bunting event/command IDs remain stable across collaboration edits.

## Migration sequence

1. Repair canonical CI bootstrap so branch evidence is trustworthy.
2. Fix server isolation, readiness, authoritative reconnect accounting, and logical-time boundaries.
3. Extend archive/replay to the complete competition command stream.
4. Remove Cloudflare's obsolete command-authority implementation and bindings.
5. Add authoritative trade/OHLC/time-and-sales projection primitives.
6. Extract the UI-independent native FIX client package from `bunting-tui`.
7. Migrate `bunting-terminal` to `gpui-kit` and remove independent old GPUI/component pins.
8. Rebuild the default terminal composition as the RIT-style workstation and migrate every chart to GPUI Kit.
9. Remove local fill/account fallback and make unavailable authority explicit.
10. Add the collaboration interface seam, initially without coupling exchange correctness to any CRDT backend.
11. Validate functional parity, accessibility/keyboard behavior, packaging, macOS release, and representative live competition flows.
12. Retire Ratatui from canonical user-facing packaging/documentation once the GPUI terminal satisfies the parity gate.

## Release gate

The GPUI terminal becomes canonical only when:

- the authoritative venue survives malformed/untrusted client behavior without cross-session failure;
- participant reconnect cannot bypass limits;
- complete competition replay reproduces final state exactly;
- market-history charts consume authoritative data;
- the default desktop layout provides the RIT-class trading workflow without requiring panel rearrangement;
- all graphical charts use GPUI Kit;
- no client-side account/fill authority remains;
- Cloudflare is publication-only;
- canonical CI and GPUI packaging are green;
- the macOS application is exercised against a live local Bunting venue, not only compiled;
- Ratatui retirement does not remove any supported participant workflow.

## Non-goals

This change does not attempt to recreate RIT's proprietary visual styling pixel-for-pixel, infer undocumented RIT formulas, make collaborative CRDT state authoritative for trading, copy GPL Zed collaboration code into Bunting, or depend on unpublished DeltaDB internals.
