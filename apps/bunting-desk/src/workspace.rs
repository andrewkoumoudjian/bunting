//! The window frame (ADR 0051 decision 1): a title bar with the screen
//! name, the command field and the connection; a collapsible sidebar of
//! Screens, Windows and Run; the canvas of floating windows; and a status
//! line. A screen is a saved view of the canvas; every canvas change is
//! saved (debounced) and restored at the next start.

use crate::actions::{self, CONTEXT};
use crate::canvas::{Canvas, CanvasEvent};
use crate::desk::Desk;
use crate::format;
use crate::layouts::{LayoutStore, MAX_SCREENS, Preset, Screen};
use crate::panels::{confirm_kill_switch, connection_label};
use crate::panes::PaneKind;
use crate::theme;
use bunting_trading_state::Connection;
use gpui_kit::assets::IconName as CatalogIcon;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Div, Entity, FocusHandle, Focusable,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    Render, SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, Subscription,
    TestSupportExt as _, WeakEntity, Window, div, prelude::*, px,
};
use std::time::Duration;

/// Element IDs of the panes the Trading template opens, in template order.
pub const PANELS: [&str; 7] = [
    "books",
    "ticket",
    "tape",
    "orders",
    "fills",
    "positions",
    "account",
];

/// Screen changes are written this long after the last one.
const SAVE_DELAY: Duration = Duration::from_millis(500);
const SIDEBAR_WIDTH: f32 = 240.;
const TITLE_BAR_HEIGHT: f32 = 36.;
const STATUS_BAR_HEIGHT: f32 = 24.;
/// Commands in the palette's screen group before the screens themselves.
const FIXED_SCREEN_COMMANDS: usize = 6;

/// The window's content.
pub struct Workspace {
    desk: Entity<Desk>,
    canvas: Entity<Canvas>,
    screens: Vec<Screen>,
    active: usize,
    store: Option<LayoutStore>,
    save_pending: bool,
    focus: FocusHandle,
    sidebar_visible: bool,
    /// The screen being renamed inline, its field and its subscription.
    renaming: Option<(usize, Entity<InputState>, Subscription)>,
    /// The command palette while it is open.
    palette: Option<Entity<CommandState>>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    /// Opens the screens saved in `store` (or one per template), showing
    /// the last active one. Without a store nothing is saved.
    #[expect(
        clippy::float_arithmetic,
        reason = "the canvas seed size is the viewport minus the frame, in pixels"
    )]
    pub fn new(
        desk: Entity<Desk>,
        store: Option<LayoutStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (screens, active) = store
            .as_ref()
            .and_then(LayoutStore::load)
            .unwrap_or_else(|| (Screen::defaults(), 0));
        let viewport = window.viewport_size();
        let canvas_size = (
            (viewport.width.as_f32() - SIDEBAR_WIDTH).max(320.),
            (viewport.height.as_f32() - TITLE_BAR_HEIGHT - STATUS_BAR_HEIGHT).max(240.),
        );
        let shared = desk.clone();
        let canvas = cx.new(|_| Canvas::new(shared, canvas_size));
        let subscriptions = vec![
            cx.observe(&desk, |_, _, cx| cx.notify()),
            cx.subscribe(&canvas, |this, _, event: &CanvasEvent, cx| match event {
                CanvasEvent::Changed => {
                    this.schedule_save(cx);
                    cx.notify();
                }
            }),
            cx.observe(&canvas, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            desk,
            canvas,
            screens,
            active,
            store,
            save_pending: false,
            focus: cx.focus_handle(),
            sidebar_visible: true,
            renaming: None,
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
    pub const fn canvas(&self) -> &Entity<Canvas> {
        &self.canvas
    }

    /// The screens' names, and which one is shown.
    #[must_use]
    pub fn screens(&self) -> (Vec<String>, usize) {
        (
            self.screens.iter().map(|each| each.name.clone()).collect(),
            self.active,
        )
    }

    /// Whether the sidebar is shown.
    #[must_use]
    pub const fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    /// Shows screen `index`, keeping the current one's windows.
    pub fn switch_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.screens.len() || index == self.active {
            return;
        }
        self.remember(cx);
        self.show(index, window, cx);
        self.save(cx);
        cx.notify();
    }

    /// Adds a screen laid out as `preset`'s template and shows it.
    pub fn add_screen(&mut self, preset: Preset, window: &mut Window, cx: &mut Context<Self>) {
        if self.screens.len() >= MAX_SCREENS {
            return;
        }
        self.remember(cx);
        let name = self.fresh_name(preset.name());
        self.screens.push(Screen::new(name, preset));
        self.show(self.screens.len() - 1, window, cx);
        self.save(cx);
        cx.notify();
    }

    /// Deletes screen `index` (asked first from the sidebar).
    pub fn delete_screen(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.screens.len() <= 1 || index >= self.screens.len() {
            return;
        }
        self.screens.remove(index);
        if index < self.active {
            self.active -= 1;
        } else if index == self.active {
            let next = index.min(self.screens.len() - 1);
            self.show(next, window, cx);
        }
        self.save(cx);
        cx.notify();
    }

    /// Puts the current screen back to its template.
    pub fn reset_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(screen) = self.screens.get_mut(self.active) {
            screen.windows = None;
        }
        self.show(self.active, window, cx);
        self.save(cx);
    }

    /// Opens another window of `kind`, cascaded from the focused one.
    pub fn add_pane(&mut self, kind: PaneKind, window: &mut Window, cx: &mut Context<Self>) {
        self.canvas.update(cx, |canvas, cx| {
            canvas.open(kind, window, cx);
        });
        cx.notify();
    }

    /// Saves every screen now.
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

    /// Shows the screen `step` places along, wrapping around.
    fn step_screen(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.screens.len();
        if count > 1 {
            let next = if forward {
                (self.active + 1) % count
            } else {
                (self.active + count - 1) % count
            };
            self.switch_to(next, window, cx);
        }
    }

    fn fresh_name(&self, base: &str) -> String {
        if !self.screens.iter().any(|screen| screen.name == base) {
            return base.to_owned();
        }
        for number in 2..=MAX_SCREENS {
            let candidate = format!("{base} {number}");
            if !self.screens.iter().any(|screen| screen.name == candidate) {
                return candidate;
            }
        }
        format!("{base} {}", self.screens.len() + 1)
    }

    /// The palette's entries: every command, with its key where it has one.
    #[expect(
        clippy::too_many_lines,
        reason = "the palette's groups read best whole, beside each other"
    )]
    fn palette(&self, state: &Entity<CommandState>, cx: &mut Context<Self>) -> impl IntoElement {
        let mut screens = CommandGroup::new()
            .label("Screen")
            .item(
                CommandItem::new()
                    .label("New screen from the Trading template")
                    .icon(IconName::Plus)
                    .action(Box::new(actions::NewWorkspace)),
            )
            .item(
                CommandItem::new()
                    .label("Next screen")
                    .action(Box::new(actions::NextWorkspace)),
            )
            .item(
                CommandItem::new()
                    .label("Previous screen")
                    .action(Box::new(actions::PreviousWorkspace)),
            )
            .item(
                CommandItem::new()
                    .label("Save screens")
                    .action(Box::new(actions::SaveLayout)),
            )
            .item(
                CommandItem::new()
                    .label("Reset screen to its template")
                    .action(Box::new(actions::ResetLayout)),
            )
            .item(
                CommandItem::new()
                    .label("Toggle sidebar")
                    .action(Box::new(actions::ToggleSidebar)),
            );
        // Entries after the fixed ones show a screen; confirming one
        // switches (see `on_confirm` below).
        for (index, screen) in self.screens.iter().enumerate() {
            screens = screens.item(
                CommandItem::new()
                    .label(format!("Show {}", screen.name))
                    .icon(IconName::LayoutDashboard)
                    .checked(index == self.active)
                    .keywords(["switch", "screen"]),
            );
        }
        let windows = CommandGroup::new()
            .label("Window")
            .item(
                CommandItem::new()
                    .label("Tile windows")
                    .action(Box::new(actions::TileWindows)),
            )
            .item(
                CommandItem::new()
                    .label("Cascade windows")
                    .action(Box::new(actions::CascadeWindows)),
            )
            .item(
                CommandItem::new()
                    .label("Close window")
                    .action(Box::new(actions::CloseWindow)),
            )
            .item(
                CommandItem::new()
                    .label("Close all windows")
                    .action(Box::new(actions::CloseAllWindows)),
            )
            .item(
                CommandItem::new()
                    .label("Reopen closed window")
                    .action(Box::new(actions::ReopenClosedWindow)),
            )
            .item(
                CommandItem::new()
                    .label("Focus next window")
                    .action(Box::new(actions::FocusNextWindow)),
            )
            .item(
                CommandItem::new()
                    .label("Focus previous window")
                    .action(Box::new(actions::FocusPreviousWindow)),
            )
            .item(
                CommandItem::new()
                    .label("Maximize or restore window")
                    .action(Box::new(actions::MaximizeWindow)),
            );
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
            .group(screens)
            .group(windows)
            .group(panes)
            .group(trading)
            .group(crate::appearance::palette_group(cx))
            .placeholder("Type a command")
            .max_h(px(420.))
            .on_confirm(move |path, window, cx| {
                close.update(cx, |this, cx| {
                    this.close_palette(window, cx);
                    if path.section == 0 && path.row >= FIXED_SCREEN_COMMANDS {
                        this.switch_to(path.row - FIXED_SCREEN_COMMANDS, window, cx);
                    }
                });
            })
            .on_cancel(move |window, cx| {
                cancel.update(cx, |this, cx| this.close_palette(window, cx));
            })
    }

    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = index;
        let Some(screen) = self.screens.get(index) else {
            return;
        };
        let name = screen.name.clone();
        let preset = screen.preset;
        let saved = screen.windows.clone();
        let layout_size = screen.canvas_size();
        self.canvas.update(cx, |canvas, cx| {
            canvas.set_screen(
                name,
                &preset.fractions(),
                saved.as_deref(),
                layout_size,
                window,
                cx,
            );
        });
    }

    /// Copies the shown canvas into its screen. A screen whose template
    /// has not been placed yet (no painted bounds) keeps `windows: None`
    /// rather than saving an empty or seed-sized layout.
    fn remember(&mut self, cx: &App) {
        let canvas = self.canvas.read(cx);
        if canvas.has_pending_template() {
            return;
        }
        let windows = canvas.dump();
        let layout_size = canvas.base_size();
        if let Some(screen) = self.screens.get_mut(self.active) {
            screen.windows = Some(windows);
            if let Some((w, h)) = layout_size {
                screen.canvas_w = Some(w);
                screen.canvas_h = Some(h);
            }
        }
    }

    fn save(&self, _cx: &App) {
        if let Some(store) = &self.store
            && let Err(error) = store.save(&self.screens, self.active)
        {
            eprintln!(
                "bunting-desk: screens not saved to {}: {error}",
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
            .pb_3()
            .child(
                section_header("Screens", cx).child(
                    Button::new("workspace-new")
                        .icon(IconName::Plus)
                        .ghost()
                        .xsmall()
                        .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, {
                            let workspace = cx.entity();
                            move |menu, _, _| {
                                let mut menu = menu;
                                for preset in Preset::ALL {
                                    let workspace = workspace.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(preset.name())
                                            .icon(IconName::Frame)
                                            .on_click(move |_, window, cx| {
                                                workspace.update(cx, |this, cx| {
                                                    this.add_screen(preset, window, cx);
                                                });
                                            }),
                                    );
                                }
                                menu
                            }
                        }),
                ),
            )
            .child(self.screens_section(cx))
            .child(
                section_header("Windows", cx).child(
                    Button::new("windows-add")
                        .icon(IconName::Plus)
                        .ghost()
                        .xsmall()
                        .dropdown_menu_with_anchor(gpui_kit::Anchor::TopRight, |menu, _, _| {
                            let mut menu = menu;
                            for kind in PaneKind::ALL {
                                menu = menu.menu_with_icon(
                                    kind.title(),
                                    kind.icon(),
                                    actions::open(kind),
                                );
                            }
                            menu
                        }),
                ),
            )
            .child(self.windows_section(cx))
            .child(section_header("Run", cx))
            .child(self.run_section(cx))
    }

    fn screens_section(&self, cx: &mut Context<Self>) -> Div {
        let mut list = v_flex().gap_px();
        for (index, screen) in self.screens.iter().enumerate() {
            if let Some((renaming, state, _)) = &self.renaming
                && *renaming == index
            {
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("workspace-rename-{index}")))
                        .test_support()
                        .gap_1()
                        .items_center()
                        .px_1()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                this.cancel_rename(window, cx);
                                cx.stop_propagation();
                            }
                        }))
                        .child(div().flex_1().min_w_0().child(Input::new(state).small()))
                        .child(
                            Button::new(SharedString::from(format!(
                                "workspace-rename-save-{index}"
                            )))
                            .label("Save")
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| this.finish_rename(cx))),
                        ),
                );
                continue;
            }
            let active = index == self.active;
            // The rename/delete actions live inside the row so its
            // highlight spans the full sidebar width like every other row;
            // they show on row hover and their press must not also
            // trigger the row's switch.
            list = list.child(
                sidebar_row(
                    SharedString::from(format!("workspace-{index}")),
                    IconName::LayoutDashboard,
                    screen.name.clone(),
                    active,
                    false,
                    None,
                    cx,
                )
                .group("screen-row")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.switch_to(index, window, cx);
                }))
                .child(
                    h_flex()
                        .id(SharedString::from(format!("workspace-actions-{index}")))
                        .items_center()
                        .gap(px(2.))
                        .opacity(0.)
                        .group_hover("screen-row", |style| style.opacity(1.))
                        .on_mouse_down(
                            gpui_kit::MouseButton::Left,
                            cx.listener(|_, _: &gpui_kit::MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                            }),
                        )
                        .child(
                            Button::new(SharedString::from(format!(
                                "workspace-rename-open-{index}"
                            )))
                            .icon(CatalogIcon::Pencil)
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.begin_rename(index, window, cx);
                                },
                            )),
                        )
                        .child(
                            Button::new(SharedString::from(format!("workspace-delete-{index}")))
                                .icon(CatalogIcon::Trash)
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.confirm_delete(index, window, cx);
                                })),
                        ),
                ),
            );
        }
        list
    }

    fn windows_section(&self, cx: &mut Context<Self>) -> Div {
        let mut list = v_flex().gap_px();
        for info in self.canvas.read(cx).window_list() {
            let id = info.id;
            let label = info.kind.title().to_owned();
            list = list.child(
                sidebar_row(
                    SharedString::from(format!("screen-window-{id}")),
                    info.kind.icon(),
                    label,
                    info.focused,
                    info.minimised,
                    info.minimised.then_some(IconName::Minus),
                    cx,
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.canvas.update(cx, |canvas, cx| {
                        canvas.focus_window(id, window, cx);
                    });
                    cx.notify();
                })),
            );
        }
        list
    }

    /// Run status, logical time and halts, as the venue last committed them.
    fn run_section(&self, cx: &App) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
        let theme = cx.theme();
        let desk = self.desk.read(cx);
        let model = desk.model();
        let status = model.run_status().unwrap_or("—").to_owned();
        let time = if model.run_time_ns() == 0 {
            "—".to_owned()
        } else {
            format::logical(model.run_time_ns())
        };
        let halted: Vec<String> = desk
            .instruments()
            .into_iter()
            .filter(|instrument| model.is_halted(instrument.instrument_id))
            .map(|instrument| instrument.symbol)
            .collect();
        let halts = if halted.is_empty() {
            "None".to_owned()
        } else {
            halted.join(", ")
        };
        let row = |label: &str, value: String| {
            h_flex()
                .h(crate::ui::rem_px(
                    crate::appearance::metrics(cx).sidebar_row_height,
                ))
                .mx_1()
                .pl(px(12.))
                .pr(px(8.))
                .items_center()
                .justify_between()
                .text_size(crate::ui::rem(13.))
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(label.to_owned()),
                )
                .child(
                    div()
                        .font_features(theme::tabular())
                        .text_color(theme.sidebar_foreground)
                        .child(value),
                )
        };
        v_flex()
            .id("run-section")
            .test_support()
            .gap_px()
            .child(row("Status", status))
            .child(row("Run time", time))
            .child(row("Halts", halts))
    }

    fn begin_rename(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(screen) = self.screens.get(index) else {
            return;
        };
        let name = screen.name.clone();
        let state = cx.new(|cx| InputState::new(window, cx).default_value(name));
        state.update(cx, |state, cx| state.focus(window, cx));
        // Enter and focus loss commit; Escape cancels (the row's key
        // handler, which clears `renaming` before any later blur).
        let subscription = cx.subscribe(&state, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_rename(cx);
            }
        });
        self.renaming = Some((index, state, subscription));
        cx.notify();
    }

    fn cancel_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.renaming = None;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some((index, state, _)) = self.renaming.take() else {
            return;
        };
        let name = state.read(cx).value().trim().to_owned();
        if !name.is_empty()
            && let Some(screen) = self.screens.get_mut(index)
        {
            screen.name = name;
            self.schedule_save(cx);
        }
        cx.notify();
    }

    fn confirm_delete(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.screens.len() <= 1 {
            return;
        }
        let Some(screen) = self.screens.get(index) else {
            return;
        };
        let name = screen.name.clone();
        let workspace = cx.entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let workspace = workspace.clone();
            alert
                .confirm()
                .title(format!("Delete the screen “{name}”?"))
                .description(
                    "Its windows and their arrangement are removed from the saved screens.",
                )
                .ok_text("Delete")
                .on_ok(move |_, window, cx| {
                    workspace.update(cx, |this, cx| this.delete_screen(index, window, cx));
                    true
                })
        });
    }

    // ---- title bar -----------------------------------------------------------

    fn title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let name = self
            .screens
            .get(self.active)
            .map_or_else(String::new, |screen| screen.name.clone());
        let key_hint = Kbd::binding_for_action(&actions::TogglePalette, None, window);
        h_flex()
            .id("top-bar")
            .test_support()
            .h(crate::ui::rem(TITLE_BAR_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .px_3()
            .gap_2()
            .bg(theme.title_bar)
            .border_b_1()
            .border_color(theme.title_bar_border)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .justify_start()
                    .gap_2()
                    .child(
                        Button::new("toggle-sidebar")
                            .icon(IconName::PanelLeft)
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_visible = !this.sidebar_visible;
                                cx.notify();
                            })),
                    )
                    .child(theme::serif(name).text_size(crate::ui::rem(15.)).truncate()),
            )
            .child(
                // A field that opens the command palette; the real command
                // line with codes is U3 (ADR 0051 decision 4).
                h_flex()
                    .id("open-palette")
                    .test_support()
                    .h(crate::ui::rem_px(
                        crate::appearance::metrics(cx).control_height,
                    ))
                    .w(px(400.))
                    .items_center()
                    .gap_2()
                    .px_2()
                    .rounded(px(6.))
                    .border_1()
                    .border_color(theme.input)
                    .bg(theme.secondary)
                    .text_size(crate::ui::rem(12.))
                    .text_color(theme.muted_foreground)
                    .child(
                        Icon::new(IconName::Search)
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .child(div().flex_1().child("Type a command or code"))
                    .child(key_hint.unwrap_or_else(|| {
                        Kbd::new(gpui_kit::Keystroke::parse("secondary-k").unwrap_or_default())
                    }))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_palette(window, cx))),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .justify_end()
                    .gap_3()
                    .child(
                        // RIT's Kill All Orders: cancels every live order of
                        // yours. The participant kill switch is separate.
                        Button::new("kill-all-orders")
                            .label("Kill all orders")
                            .xsmall()
                            .outline()
                            .danger()
                            .tooltip(
                                "Cancel all your live orders (F12). The kill switch is a separate, confirmed action",
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.desk.update(cx, Desk::cancel_all);
                            })),
                    )
                    .child(self.connection_pill(cx)),
            )
    }

    /// Connection state and each venue's measured order round trip.
    fn connection_pill(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let desk = self.desk.read(cx);
        let model = desk.model();
        let (color, text): (Hsla, String) = match model.connection() {
            Connection::Connected { .. } => (theme.success, desk.label().to_owned()),
            Connection::Connecting { attempt } => {
                (theme.warning, format!("Connecting (attempt {attempt})"))
            }
            Connection::Disconnected { .. } => (theme.danger, "Disconnected".to_owned()),
            Connection::Idle => (theme.muted_foreground, "Not connected".to_owned()),
        };
        let mut pill = h_flex()
            .id("connection-pill")
            .test_support()
            .items_center()
            .gap_2()
            .text_xs()
            .font_features(theme::tabular())
            .child(div().size(px(8.)).flex_shrink_0().rounded_full().bg(color))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.foreground)
                    .child(text),
            );
        for (venue, trip) in model.venue_round_trips() {
            pill = pill.child(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!("V{venue} {}", format::duration(trip.last_us))),
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
    #[expect(
        clippy::too_many_lines,
        reason = "one handler per action, listed whole so none is missing"
    )]
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
            this.add_screen(Preset::Trading, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::NextWorkspace, window, cx| {
            this.step_screen(true, window, cx);
        }))
        .on_action(
            cx.listener(|this, _: &actions::PreviousWorkspace, window, cx| {
                this.step_screen(false, window, cx);
            }),
        )
        .on_action(cx.listener(|this, _: &actions::ToggleSidebar, _, cx| {
            this.sidebar_visible = !this.sidebar_visible;
            cx.notify();
        }))
        .on_action(cx.listener(|this, _: &actions::TileWindows, _, cx| {
            this.canvas.update(cx, Canvas::tile_windows);
        }))
        .on_action(cx.listener(|this, _: &actions::CascadeWindows, _, cx| {
            this.canvas.update(cx, Canvas::cascade_windows);
        }))
        .on_action(cx.listener(|this, _: &actions::CloseWindow, window, cx| {
            this.canvas
                .update(cx, |canvas, cx| canvas.close_focused(window, cx));
        }))
        .on_action(
            cx.listener(|this, _: &actions::CloseAllWindows, window, cx| {
                this.canvas
                    .update(cx, |canvas, cx| canvas.close_all(window, cx));
            }),
        )
        .on_action(
            cx.listener(|this, _: &actions::ReopenClosedWindow, window, cx| {
                this.canvas.update(cx, |canvas, cx| {
                    canvas.reopen_closed(window, cx);
                });
            }),
        )
        .on_action(
            cx.listener(|this, _: &actions::FocusNextWindow, window, cx| {
                this.canvas.update(cx, |canvas, cx| {
                    canvas.cycle_focus(true, window, cx);
                });
            }),
        )
        .on_action(
            cx.listener(|this, _: &actions::FocusPreviousWindow, window, cx| {
                this.canvas.update(cx, |canvas, cx| {
                    canvas.cycle_focus(false, window, cx);
                });
            }),
        )
        .on_action(cx.listener(|this, _: &actions::MaximizeWindow, _, cx| {
            this.canvas.update(cx, Canvas::maximise_focused);
        }))
        .on_action(
            cx.listener(|this, _: &actions::OpenBookTrader, window, cx| {
                this.add_pane(PaneKind::BookTrader, window, cx);
            }),
        )
        .on_action(
            cx.listener(|this, _: &actions::OpenLadderTrader, window, cx| {
                this.add_pane(PaneKind::LadderTrader, window, cx);
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
        .on_action(cx.listener(|this, _: &actions::OpenNews, window, cx| {
            this.add_pane(PaneKind::News, window, cx);
        }))
        .on_action(cx.listener(|this, _: &actions::OpenDeals, window, cx| {
            this.add_pane(PaneKind::Deals, window, cx);
        }))
        .on_action(
            cx.listener(|this, _: &actions::OpenTransactions, window, cx| {
                this.add_pane(PaneKind::Transactions, window, cx);
            }),
        )
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = connection_label(self.desk.read(cx));
        let palette = self
            .palette
            .clone()
            .map(|state| self.palette(&state, cx).into_any_element());
        // Every rem in the desk and in gpui-kit scales with the user's UI
        // font size: 16 px rems at the default 13 px (ADR 0051 decision 7).
        window.set_rem_size(crate::ui::rem_size(cx));
        let theme = cx.theme();
        v_flex()
            .id("workspace")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .map(|root| Self::with_actions(root, cx))
            .map(crate::appearance::with_appearance_actions)
            .relative()
            .size_full()
            .bg(theme.background)
            .text_color(theme.foreground)
            .font_family(theme::SANS)
            .child(self.title_bar(window, cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .when(self.sidebar_visible, |row| row.child(self.sidebar(cx)))
                    .child(div().flex_1().min_w_0().h_full().child(self.canvas.clone())),
            )
            .child(
                StatusBar::new()
                    .h(crate::ui::rem(STATUS_BAR_HEIGHT))
                    .left(
                        div()
                            .id("connection")
                            .test_support()
                            .font_features(theme::tabular())
                            .child(status),
                    )
                    .child(div().text_xs().child(format!(
                        "{} commands · Ctrl+Tab screens",
                        theme::command_key()
                    )))
                    .right(concat!("bunting-desk ", env!("CARGO_PKG_VERSION"))),
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

/// A sidebar section header: 28 px, sentence-case 12 px semibold muted,
/// with room for a trailing ghost action on the same row.
fn section_header(text: &'static str, cx: &App) -> Div {
    h_flex()
        .h(crate::ui::rem_px(
            crate::appearance::metrics(cx).sidebar_row_height,
        ))
        .items_center()
        .justify_between()
        .pl(px(12.))
        .pr(px(8.))
        .mt_2()
        .text_size(crate::ui::rem(12.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(div().child(text))
}

fn sidebar_row(
    id: impl Into<SharedString>,
    icon: IconName,
    label: impl Into<SharedString>,
    active: bool,
    muted: bool,
    trailing: Option<IconName>,
    cx: &App,
) -> gpui_kit::base::ObservedElement<gpui_kit::Stateful<Div>> {
    let theme = cx.theme();
    h_flex()
        .id(gpui_kit::ElementId::Name(id.into()))
        .test_support()
        .h(crate::ui::rem_px(
            crate::appearance::metrics(cx).sidebar_row_height,
        ))
        .items_center()
        .gap(px(8.))
        .mx_1()
        .pl(px(12.))
        .pr(px(8.))
        .rounded(px(4.))
        .text_size(crate::ui::rem(13.))
        .text_color(if muted {
            theme.muted_foreground
        } else if active {
            theme.sidebar_accent_foreground
        } else {
            theme.sidebar_foreground
        })
        .when(active, |row| {
            row.bg(theme.sidebar_accent).font_weight(FontWeight::MEDIUM)
        })
        .hover(|row| row.bg(theme.sidebar_accent))
        .child(Icon::new(icon).small().text_color(theme.muted_foreground))
        .child(div().flex_1().min_w_0().truncate().child(label.into()))
        .when_some(trailing, |row, icon| {
            row.child(Icon::new(icon).small().text_color(theme.muted_foreground))
        })
}
