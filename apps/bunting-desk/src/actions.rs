//! The desk's commands as GPUI actions: bound to keys here, listed in the
//! command palette, and handled by the workspace. A key or a palette entry
//! runs the same action the buttons do.

use crate::panes::PaneKind;
use gpui_kit::{Action, App, KeyBinding, actions};
use serde::Deserialize;

actions!(
    desk,
    [
        /// Opens or closes the command palette.
        TogglePalette,
        SaveLayout,
        ResetLayout,
        NewWorkspace,
        NextWorkspace,
        PreviousWorkspace,
        ToggleSidebar,
        TileWindows,
        CascadeWindows,
        CloseWindow,
        ReopenClosedWindow,
        FocusNextWindow,
        FocusPreviousWindow,
        MaximizeWindow,
        OpenBookTrader,
        OpenLadderTrader,
        OpenOrderEntry,
        OpenTape,
        OpenBlotter,
        OpenFills,
        OpenPortfolio,
        OpenTraderInfo,
        OpenNotices,
        OpenNews,
        OpenDeals,
        CancelAll,
        /// Asks before activating the kill switch.
        KillSwitch,
        ResetKillSwitch,
        Refresh,
        /// Brings the market back to the middle of a ladder.
        CentreLadder,
        /// Cycles to the next installed theme (ADR 0051 U2a).
        SelectNextTheme,
        /// Switches between the current theme's light and dark sides.
        ToggleLightDark,
        IncreaseUiFontSize,
        DecreaseUiFontSize,
        ResetUiFontSize,
        /// Cycles compact, default and comfortable density.
        CycleDensity,
    ]
);

/// Makes the named theme current: one palette entry per installed
/// theme (ADR 0051 U2a). Handled with the other appearance actions in
/// `crate::appearance::with_appearance_actions`.
#[derive(Clone, Action, PartialEq, Eq, Deserialize, Debug)]
#[action(namespace = desk, no_json)]
#[allow(
    clippy::unsafe_derive_deserialize,
    reason = "the unsafe is in GPUI's generated Action impl, not in deserializing a name"
)]
pub struct SelectTheme {
    /// The theme's name, as written in its theme file.
    pub name: String,
}

/// The key context the workspace sets; bindings apply anywhere inside it.
pub const CONTEXT: &str = "Workspace";

/// Binds the desk's keys. `secondary` is ⌘ on macOS and Ctrl elsewhere.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-shift-p", TogglePalette, Some(CONTEXT)),
        KeyBinding::new("secondary-k", TogglePalette, Some(CONTEXT)),
        KeyBinding::new("secondary-s", SaveLayout, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-n", NewWorkspace, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextWorkspace, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousWorkspace, Some(CONTEXT)),
        KeyBinding::new("secondary-b", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-t", TileWindows, Some(CONTEXT)),
        KeyBinding::new("secondary-w", CloseWindow, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-t", ReopenClosedWindow, Some(CONTEXT)),
        KeyBinding::new("ctrl-`", FocusNextWindow, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-`", FocusPreviousWindow, Some(CONTEXT)),
        KeyBinding::new("secondary-r", Refresh, Some(CONTEXT)),
        KeyBinding::new("secondary-=", IncreaseUiFontSize, Some(CONTEXT)),
        KeyBinding::new("secondary--", DecreaseUiFontSize, Some(CONTEXT)),
        KeyBinding::new("secondary-0", ResetUiFontSize, Some(CONTEXT)),
        KeyBinding::new("f12", CancelAll, Some(CONTEXT)),
        KeyBinding::new("space", CentreLadder, Some("Ladder && !Input")),
    ]);
}

/// The action that opens another pane of `kind`.
#[must_use]
pub fn open(kind: PaneKind) -> Box<dyn Action> {
    match kind {
        PaneKind::BookTrader => Box::new(OpenBookTrader),
        PaneKind::LadderTrader => Box::new(OpenLadderTrader),
        PaneKind::OrderEntry => Box::new(OpenOrderEntry),
        PaneKind::Tape => Box::new(OpenTape),
        PaneKind::Blotter => Box::new(OpenBlotter),
        PaneKind::Fills => Box::new(OpenFills),
        PaneKind::Portfolio => Box::new(OpenPortfolio),
        PaneKind::TraderInfo => Box::new(OpenTraderInfo),
        PaneKind::Notices => Box::new(OpenNotices),
        PaneKind::News => Box::new(OpenNews),
        PaneKind::Deals => Box::new(OpenDeals),
    }
}
