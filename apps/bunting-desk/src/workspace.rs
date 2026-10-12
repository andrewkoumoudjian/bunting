//! The window: a sidebar of workspaces, panes to add and a watchlist; a top
//! bar for the workspace and the connection; the dock of panes; and a status
//! bar (ADR 0050 decision 6). Panes can be opened any number of times,
//! moved, split, tabbed and resized; each workspace keeps its own layout and
//! every change is saved.

use crate::actions::{self, CONTEXT};
use crate::desk::Desk;
use crate::format;
use crate::layouts::{LAYOUT_VERSION, LayoutStore, MAX_WORKSPACES, Preset, Saved};
use crate::panels::{confirm_kill_switch, connection_label};
use crate::panes::{self, PaneKind};
use crate::theme;
use bunting_trading_state::Connection;
use bunting_trading_state::bunting_client::bnp_wire::Side;
use gpui_kit::component::button::Button;
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::dock::{DockArea, DockEvent, DockPlacement, DockSkin};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, AsyncApp, Context, Div, Entity, FocusHandle, Focusable, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled as _, Subscription, TestSupportExt as _, WeakEntity,
    Window, div, prelude::*, px,
};
use std::time::Duration;

/// Element IDs of the panes a first run shows, in layout order.
pub const PANELS: [&str; 8] = [
    "books",
    "ticket",
    "tape",
    "orders",
    "fills",
    "positions",
    "account",
    "notices",
];

/// Layout changes are written this long after the last one.
const SAVE_DELAY: Duration = Duration::from_millis(500);
const SIDEBAR_WIDTH: f32 = 224.;
/// Commands in the palette's workspace group before the workspaces.
const FIXED_WORKSPACE_COMMANDS: usize = 5;

/// The window's content.
pub struct Workspace {
    desk: Entity<Desk>,
    area: Entity<DockArea>,
    workspaces: Vec<Saved>,
    active: usize,
    store: Option<LayoutStore>,
    save_pending: bool,
    focus: FocusHandle,
    /// The command palette while it is open.
    palette: Option<Entity<CommandState>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// Opens the workspaces saved in `store` (or one per preset), showing
    /// the last active one. Without a store nothing is saved.
    pub fn new(
        desk: Entity<Desk>,
        store: Option<LayoutStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        panes::register(&desk, cx);
        let (area, _skin) = DockSkin::dock_area("bunting-desk", Some(LAYOUT_VERSION), window, cx);
        let (workspaces, active) = store
            .as_ref()
            .and_then(LayoutStore::load)
            .unwrap_or_else(|| (Saved::defaults(), 0));
        let subscriptions = vec![
            cx.observe(&desk, |_, _, cx| cx.notify()),
            cx.subscribe(&area, |this, _, event: &DockEvent, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.schedule_save(cx);
                }
            }),
        ];
        let mut this = Self {
            desk,
            area,
            workspaces,
            active,
            store,
            save_pending: false,
            focus: cx.focus_handle(),
            palette: None,
            _subscriptions: subscriptions,
        };
        this.show(active, window, cx);
        window.focus(&this.focus, cx);
        this
    }

    #[must_use]
    pub const fn desk(&self) -> &Entity<Desk> {
        &self.desk
    }

    #[must_use]
    pub const fn area(&self) -> &Entity<DockArea> {
        &self.area
    }

    /// The workspaces' names, and which one is shown.
    #[must_use]
    pub fn workspaces(&self) -> (Vec<String>, usize) {
        (
            self.workspaces
                .iter()
                .map(|each| each.name.clone())
                .collect(),
            self.active,
        )
    }

    /// Shows workspace `index`, keeping the current one's layout.
    pub fn switch_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.workspaces.len() || index == self.active {
            return;
        }
        self.remember(cx);
        self.show(index, window, cx);
        self.save(cx);
        cx.notify();
    }

    /// Adds a workspace laid out as `preset` and shows it.
    pub fn add_workspace(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        if self.workspaces.len() >= MAX_WORKSPACES {
            return;
        }
        self.remember(cx);
        let number = self.workspaces.len() + 1;
        self.workspaces
            .push(Saved::new(format!("Workspace {number}"), preset));
        self.show(self.workspaces.len() - 1, window, cx);
        self.save(cx);
        cx.notify();
    }

    /// Puts the current workspace back to its preset.
    pub fn reset_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(saved) = self.workspaces.get_mut(self.active) {
            saved.layout = None;
        }
        self.show(self.active, window, cx);
        self.save(cx);
    }

    /// Opens another pane of `kind` in the centre.
    pub fn add_pane(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        let pane = kind.build(self.desk.clone(), window, cx);
        self.area.update(cx, |area, cx| {
            area.add_panel_view(pane, DockPlacement::Center, None, window, cx);
        });
    }

    /// Saves every workspace now.
    pub fn save_layout(&mut self, cx: &mut Context<Self>) {
        self.remember(cx);
        self.save(cx);
    }

    /// Whether the command palette is open.
    #[must_use]
    pub const fn palette_open(&self) -> bool {
        self.palette.is_some()
    }

    /// Opens the command palette, or closes it if open.
    pub fn toggle_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.palette.is_some() {
            self.close_palette(window, cx);
            return;
        }
        let state = cx.new(|cx| CommandState::new(window, cx));
        state.update(cx, |state, cx| state.focus(window, cx));
        self.palette = Some(state);
        cx.notify();
    }

    fn close_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    /// Shows the workspace `step` places along, wrapping around.
    fn step_workspace(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.workspaces.len();
        if count > 1 {
            let next = if forward {
                (self.active + 1) % count
            } else {
                (self.active + count - 1) % count
            };
            self.switch_to(next, window, cx);
        }
    }

    /// The palette's entries: every command, with its key where it has one.
    fn palette(&self, state: &Entity<CommandState>, cx: &mut Context<Self>) -> impl IntoElement {
        let mut workspaces = CommandGroup::new()
            .label("Workspace")
            .item(
                CommandItem::new()
                    .label("New workspace")
                    .icon(IconName::Plus)
                    .action(Box::new(actions::NewWorkspace)),
            )
            .item(
                CommandItem::new()
                    .label("Next workspace")
                    .action(Box::new(actions::NextWorkspace)),
            )
            .item(
                CommandItem::new()
                    .label("Previous workspace")
                    .action(Box::new(actions::PreviousWorkspace)),
            )
            .item(
                CommandItem::new()
                    .label("Save layout")
                    .action(Box::new(actions::SaveLayout)),
            )
            .item(
                CommandItem::new()
                    .label("Reset layout to its preset")
                    .action(Box::new(actions::ResetLayout)),
            );
        // Entries after the fixed ones show a workspace; confirming one
        // switches (see `on_confirm` below).
        for (index, saved) in self.workspaces.iter().enumerate() {
            workspaces = workspaces.item(
                CommandItem::new()
                    .label(format!("Show {}", saved.name))
                    .icon(IconName::LayoutDashboard)
                    .checked(index == self.active)
                    .keywords(["switch", "workspace"]),
            );
        }
        let mut panes = CommandGroup::new().label("Open pane");
        for kind in PaneKind::ALL {
            panes = panes.item(
                CommandItem::new()
                    .label(format!("Open {}", kind.title()))
                    .icon(kind.icon())
                    .action(actions::open(kind)),
            );
        }
        let trading = CommandGroup::new()
            .label("Trading")
            .item(
                CommandItem::new()
                    .label("Cancel all my orders")
                    .action(Box::new(actions::CancelAll)),
            )
            .item(
                CommandItem::new()
                    .label("Activate the kill switch…")
                    .action(Box::new(actions::KillSwitch)),
            )
            .item(
                CommandItem::new()
                    .label("Reset the kill switch")
                    .action(Box::new(actions::ResetKillSwitch)),
            )
            .item(
                CommandItem::new()
                    .label("Reload orders and account")
                    .action(Box::new(actions::Refresh)),
            );
        let close = cx.entity();
        let cancel = cx.entity();
        Command::new(state)
            .group(workspaces)
            .group(panes)
            .group(trading)
            .placeholder("Type a command")
            .max_h(px(420.))
            .on_confirm(move |path, window, cx| {
                close.update(cx, |this, cx| {
                    this.close_palette(window, cx);
                    if path.section == 0 && path.row >= FIXED_WORKSPACE_COMMANDS {
                        this.switch_to(path.row - FIXED_WORKSPACE_COMMANDS, window, cx);
                    }
                });
            })
            .on_cancel(move |window, cx| {
                cancel.update(cx, |this, cx| this.close_palette(window, cx));
            })
    }

    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index;
        let Some(saved) = self.workspaces.get(index) else {
            return;
        };
        let preset = saved.preset;
        let state = saved
            .layout
            .clone()
            .filter(|state| state.version == Some(LAYOUT_VERSION));
        let loaded = state.is_some_and(|state| {
            self.area
                .update(cx, |area, cx| area.load(state, window, cx))
                .is_ok()
        });
        if !loaded {
            let layout = preset.layout(&self.desk, window, cx);
            self.area
                .update(cx, |area, cx| area.set_center(layout, window, cx));
        }
    }

    /// Copies the shown layout into its workspace.
    fn remember(&mut self, cx: &App) {
        let state = self.area.read(cx).dump(cx);
        if let Some(saved) = self.workspaces.get_mut(self.active) {
            saved.layout = Some(state);
        }
    }

    fn save(&self, _cx: &App) {
        if let Some(store) = &self.store
            && let Err(error) = store.save(&self.workspaces, self.active)
        {
            eprintln!(
                "bunting-desk: workspaces not saved to {}: {error}",
                store.path().display()
            );
        }
    }

    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.store.is_none() || self.save_pending {
            return;
        }
        self.save_pending = true;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(SAVE_DELAY).await;
            let _ = this.update(cx, |this, cx| {
                this.save_pending = false;
                this.save_layout(cx);
            });
        })
        .detach();
    }

    // ---- sidebar -------------------------------------------------------------

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let mut workspaces = v_flex().gap_px();
        for (index, saved) in self.workspaces.iter().enumerate() {
            let active = index == self.active;
            workspaces = workspaces.child(
                sidebar_row(
                    SharedString::from(format!("workspace-{index}")),
                    IconName::LayoutDashboard,
                    saved.name.clone(),
                    active,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.switch_to(index, window, cx);
                })),
            );
        }
        workspaces = workspaces.child(
            sidebar_row("workspace-new", IconName::Plus, "New workspace", false, cx).on_click(
                cx.listener(|this, _, window, cx| {
                    this.add_workspace(Preset::Trading, window, cx);
                }),
            ),
        );
        let mut add = v_flex().gap_px();
        for kind in PaneKind::ALL {
            add = add.child(
                sidebar_row(
                    SharedString::from(format!("add-{}", kind.name())),
                    kind.icon(),
                    kind.title(),
                    false,
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.add_pane(kind, window, cx);
                })),
            );
        }
        v_flex()
            .id("sidebar")
            .test_support()
            .w(px(SIDEBAR_WIDTH))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            .px_2()
            .pb_3()
            .gap_1()
            .child(
                theme::serif("Bunting")
                    .text_size(px(24.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .px_2()
                    .pt_3()
                    .pb_2(),
            )
            .child(section_label("Workspaces", cx))
            .child(workspaces)
            .child(section_label("Add pane", cx))
            .child(add)
            .child(section_label("Watchlist", cx))
            .child(self.watchlist(cx))
    }

    /// Each instrument with its last consolidated trade and the best bid and
    /// offer across venues as the tape shows them; clicking one shows it.
    fn watchlist(&self, cx: &mut Context<Self>) -> Div {
        let desk = self.desk.read(cx);
        let model = desk.model();
        let shown = desk.instrument().map(|instrument| instrument.instrument_id);
        let rows: Vec<(u128, String, String, String)> = desk
            .instruments()
            .into_iter()
            .map(|instrument| {
                let id = instrument.instrument_id;
                let last = model
                    .tape_prints()
                    .iter()
                    .rev()
                    .find(|print| print.listing.instrument_id == id)
                    .map_or_else(|| "—".to_owned(), |print| print.price.to_string());
                let quote = model.tape(id).map_or_else(String::new, |tape| {
                    let side = |side| {
                        tape.book
                            .best(side)
                            .map_or_else(|| "—".to_owned(), |(_, level)| level.price.to_string())
                    };
                    format!("{} × {}", side(Side::Buy), side(Side::Sell))
                });
                (id, instrument.symbol, last, quote)
            })
            .collect();
        let mut list = v_flex().gap_px();
        if rows.is_empty() {
            return list.child(
                div()
                    .px_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Instruments appear when the venue lists them."),
            );
        }
        for (id, symbol, last, quote) in rows {
            let active = shown == Some(id);
            let theme = cx.theme();
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("watch-{id}")))
                    .test_support()
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .when(active, |row| row.bg(theme.sidebar_accent))
                    .hover(|row| row.bg(theme.sidebar_accent))
                    .font_features(theme::tabular())
                    .justify_between()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.sidebar_foreground)
                            .child(symbol),
                    )
                    .child(
                        v_flex()
                            .items_end()
                            .child(div().text_sm().child(last))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(quote),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.desk
                            .update(cx, |desk, cx| desk.select_instrument(id, cx));
                    })),
            );
        }
        list
    }

    // ---- top bar -------------------------------------------------------------

    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let name = self
            .workspaces
            .get(self.active)
            .map_or_else(String::new, |saved| saved.name.clone());
        let preset = self
            .workspaces
            .get(self.active)
            .map_or(Preset::Trading, |saved| saved.preset);
        h_flex()
            .id("top-bar")
            .test_support()
            .h(px(44.))
            .flex_shrink_0()
            .px_3()
            .gap_2()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.title_bar_border)
            .child(
                h_flex()
                    .gap_2()
                    .items_baseline()
                    .child(theme::serif(name).text_size(px(17.)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("from {}", preset.name())),
                    ),
            )
            .child(div().w(px(8.)))
            .child(
                Button::new("save-layout")
                    .label("Save layout")
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| this.save_layout(cx))),
            )
            .child(
                Button::new("reset-layout")
                    .label("Reset layout")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.reset_layout(window, cx))),
            )
            .child(
                Button::new("open-palette")
                    .label("Commands")
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_palette(window, cx))),
            )
            .child(div().flex_1())
            .child(self.connection_pill(cx))
    }

    /// Connection state and each venue's measured order round trip.
    fn connection_pill(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let desk = self.desk.read(cx);
        let model = desk.model();
        let (color, text): (Hsla, String) = match model.connection() {
            Connection::Connected { welcome, .. } => (
                theme.success,
                format!("Connected · participant {}", welcome.participant_id),
            ),
            Connection::Connecting { attempt } => {
                (theme.warning, format!("Connecting (attempt {attempt})"))
            }
            Connection::Disconnected { .. } => (theme.danger, "Disconnected".to_owned()),
            Connection::Idle => (theme.muted_foreground, "Not connected".to_owned()),
        };
        let mut pill = h_flex()
            .id("connection-pill")
            .test_support()
            .gap_2()
            .text_xs()
            .font_features(theme::tabular())
            .child(div().size(px(8.)).rounded_full().bg(color))
            .child(div().text_color(theme.foreground).child(text));
        for (venue, trip) in model.venue_round_trips() {
            pill = pill.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("Venue {venue} {}", format::duration(trip.last_us))),
            );
        }
        if let Some(status) = model.run_status() {
            pill = pill.child(
                div()
                    .id("run-status")
                    .test_support()
                    .px_2()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .text_color(theme.secondary_foreground)
                    .child(format!("Run {status}")),
            );
        }
        if model.kill_switch_active() {
            pill = pill.child(
                div()
                    .px_2()
                    .rounded(theme.radius)
                    .bg(theme.danger)
                    .text_color(theme.background)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("KILL SWITCH"),
            );
        }
        pill.into_any_element()
    }
}

impl Workspace {
    /// Handles every desk action: from keys, the palette or a button.
    fn with_actions(root: Stateful<Div>, cx: &Context<Self>) -> Stateful<Div> {
        root.on_action(cx.listener(|this, _: &actions::TogglePalette, window, cx| {
            this.toggle_palette(window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::SaveLayout, _, cx| {
            this.save_layout(cx);
        }))
        .on_action(cx.listener(|this, _: &actions::ResetLayout, window, cx| {
            this.reset_layout(window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::NewWorkspace, window, cx| {
            this.add_workspace(Preset::Trading, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::NextWorkspace, window, cx| {
            this.step_workspace(true, window, cx);
        }))
        .on_action(
            cx.listener(|this, _: &actions::PreviousWorkspace, window, cx| {
                this.step_workspace(false, window, cx);
            }),
        )
        .on_action(
            cx.listener(|this, _: &actions::OpenBookTrader, window, cx| {
                this.add_pane(PaneKind::BookTrader, window, cx);
            }),
        )
        .on_action(
            cx.listener(|this, _: &actions::OpenOrderEntry, window, cx| {
                this.add_pane(PaneKind::OrderEntry, window, cx);
            }),
        )
        .on_action(cx.listener(|this, _: &actions::OpenTape, window, cx| {
            this.add_pane(PaneKind::Tape, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::OpenBlotter, window, cx| {
            this.add_pane(PaneKind::Blotter, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::OpenFills, window, cx| {
            this.add_pane(PaneKind::Fills, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::OpenPortfolio, window, cx| {
            this.add_pane(PaneKind::Portfolio, window, cx);
        }))
        .on_action(
            cx.listener(|this, _: &actions::OpenTraderInfo, window, cx| {
                this.add_pane(PaneKind::TraderInfo, window, cx);
            }),
        )
        .on_action(cx.listener(|this, _: &actions::OpenNotices, window, cx| {
            this.add_pane(PaneKind::Notices, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::CancelAll, _, cx| {
            this.desk.update(cx, Desk::cancel_all);
        }))
        .on_action(cx.listener(|this, _: &actions::KillSwitch, window, cx| {
            confirm_kill_switch(this.desk.clone(), window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::ResetKillSwitch, _, cx| {
            this.desk.update(cx, Desk::reset_kill_switch);
        }))
        .on_action(cx.listener(|this, _: &actions::Refresh, _, cx| {
            this.desk.update(cx, Desk::refresh);
        }))
    }
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = connection_label(self.desk.read(cx));
        let palette = self
            .palette
            .clone()
            .map(|state| self.palette(&state, cx).into_any_element());
        let theme = cx.theme();
        h_flex()
            .id("workspace")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .map(|root| Self::with_actions(root, cx))
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme::SANS)
            .child(self.sidebar(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.top_bar(cx))
                    .child(div().flex_1().min_h_0().child(self.area.clone()))
                    .child(
                        StatusBar::new()
                            .left(
                                div()
                                    .id("connection")
                                    .test_support()
                                    .font_features(theme::tabular())
                                    .child(status),
                            )
                            .right(concat!("bunting-desk ", env!("CARGO_PKG_VERSION"))),
                    ),
            )
            .when_some(palette, |root, palette| {
                root.child(
                    div()
                        .id("palette-backdrop")
                        .absolute()
                        .inset_0()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_palette(window, cx);
                        })),
                )
                .child(
                    h_flex()
                        .absolute()
                        .top(px(56.))
                        .left_0()
                        .right_0()
                        .justify_center()
                        .child(
                            div()
                                .id("palette")
                                .test_support()
                                .occlude()
                                .w(px(560.))
                                .bg(cx.theme().popover)
                                .rounded_lg()
                                .shadow_lg()
                                .child(palette),
                        ),
                )
            })
    }
}

fn section_label(text: &'static str, cx: &App) -> Div {
    div()
        .px_2()
        .pt_3()
        .pb_1()
        .text_xs()
        .font_weight(FontWeight::MEDIUM)
        .text_color(cx.theme().muted_foreground)
        .child(text.to_uppercase())
}

fn sidebar_row(
    id: impl Into<SharedString>,
    icon: IconName,
    label: impl Into<SharedString>,
    active: bool,
    cx: &App,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    let theme = cx.theme();
    h_flex()
        .id(gpui_kit::ElementId::Name(id.into()))
        .test_support()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(theme.radius)
        .cursor_pointer()
        .text_sm()
        .text_color(if active {
            theme.sidebar_accent_foreground
        } else {
            theme.sidebar_foreground
        })
        .when(active, |row| row.bg(theme.sidebar_accent))
        .hover(|row| row.bg(theme.sidebar_accent))
        .child(Icon::new(icon).small().text_color(theme.muted_foreground))
        .child(label.into())
}
