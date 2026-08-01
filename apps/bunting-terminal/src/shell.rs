use crate::{
    local_server::{LocalServerController, LocalServerSnapshot},
    model::{PanelKind, WorkspacePreset},
    terminal::{MarketPanel, Terminal, TerminalSnapshot},
};
use gpui::{
    prelude::FluentBuilder as _,
    App, AppContext as _, Context, Edges, Entity, InteractiveElement as _, IntoElement,
    MouseButton, ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div,
    px,
};
use gpui_component::{
    ActiveTheme as _, IconName, Root, Sizable as _, TitleBar,
    button::{Button, ButtonVariants as _},
    dock::{DockArea, DockItem, DockPlacement, PanelView},
    h_flex,
    input::Input,
    status_bar::StatusBar,
    v_flex,
};
use std::{collections::HashMap, sync::Arc, time::Duration};

const DOCK_VERSION: usize = 2;

pub struct AppShell {
    terminal: Entity<Terminal>,
    dock_area: Entity<DockArea>,
    panels: HashMap<PanelKind, Entity<MarketPanel>>,
    snapshot: TerminalSnapshot,
    local_server: LocalServerController,
    server_snapshot: LocalServerSnapshot,
    _terminal_observer: Subscription,
}

include!("shell/layout.rs");
include!("shell/view.rs");
