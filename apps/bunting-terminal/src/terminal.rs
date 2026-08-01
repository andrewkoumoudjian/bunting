//! Shared FIX client state and gpui-component market panels.
//!
//! The shell intendionally follows Zed's workspace translation: one native
//! title bar, a dock-managed pane tree, compact status chrome, and panels that
//! own only presentation. Bunting's existing FIX/session client remains the
//! sole transport and projection boundary.

use crate::model::{PanelKind, WorkspacePreset};
use bunting_tui::client::{
    FixClient, IoTask, OutboundCmd, TerminalConfig, UiEvent, book_request, cancel,
    competition_action, competition_requests, new_order,
};
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Subscription,
    Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    chart::CandlestickChart,
    dock::{Panel, PanelControl, PanelEvent},
    h_flex,
    input::{Input, InputState},
    spinner::Spinner,
    table::{Table, TableBody, TableCell, TableHead, TableHeader, TableRow},
    v_flex,
};
use std::{env, path::PathBuf, time::Duration};
use tokio::runtime::Runtime;


include!("terminal/state.rs");
include!("terminal/panel.rs");
include!("terminal/views_trading.rs");
include!("terminal/views_research.rs");
include!("terminal/views_admin.rs");
