use crate::{
    model::{PanelKind, WorkspacePreset},
    terminal::{MarketPanel, Terminal, TerminalSnapshot},
};
use gpui::{
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
use std::{collections::HashMap, sync::Arc};

const DOCK_VERSION: usize = 2;

pub struct AppShell {
    terminal: Entity<Terminal>,
    dock_area: Entity<DockArea>,
    panels: HashMap<PanelKind, Entity<MarketPanel>>,
    snapshot: TerminalSnapshot,
    _terminal_observer: Subscription,
}


include!("shell/layout.rs");
include!("shell/view.rs");
