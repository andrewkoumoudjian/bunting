impl AppShell {
    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let command_input = self.terminal.read(cx).command_input();
        let active_preset = self.terminal.read(cx).active_preset();

        TitleBar::new()
            .child(
                h_flex()
                    .h_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(cx.theme().foreground)
                            .child("Bunting"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Market Terminal"),
                    ),
            )
            .child(
                h_flex()
                    .h_full()
                    .flex_1()
                    .justify_center()
                    .gap_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .w(px(360.))
                            .max_w_full()
                            .child(Input::new(&command_input).small()),
                    )
                    .child(
                        Button::new("run-command")
                            .small()
                            .primary()
                            .label("Run")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.execute_command(window, cx);
                            })),
                    ),
            )
            .child(
                h_flex()
                    .h_full()
                    .gap_1()
                    .items_center()
                    .px_2()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .children(WorkspacePreset::ALL.into_iter().map(|preset| {
                        Button::new(format!("workspace-{}", preset.label().to_ascii_lowercase()))
                            .xsmall()
                            .when(active_preset == preset, |button| button.primary())
                            .when(active_preset != preset, |button| button.ghost())
                            .label(preset.label())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.switch_workspace(preset, window, cx);
                            }))
                    }))
                    .child(
                        Button::new("refresh-market")
                            .xsmall()
                            .ghost()
                            .label("Refresh")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.terminal
                                    .update(cx, |terminal, cx| terminal.refresh(cx));
                            })),
                    )
                    .child(
                        Button::new("reconnect-market")
                            .xsmall()
                            .ghost()
                            .label("Reconnect")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.terminal
                                    .update(cx, |terminal, cx| terminal.reconnect(cx));
                            })),
                    ),
            )
    }

    fn render_connection_banner(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .id("connection-banner")
            .w_full()
            .px_3()
            .py_2()
            .gap_2()
            .border_b_1()
            .border_color(cx.theme().danger)
            .bg(cx.theme().danger.opacity(0.12))
            .text_xs()
            .text_color(cx.theme().danger_foreground)
            .child(
                div()
                    .font_weight(gpui::FontWeight::BOLD)
                    .child("FIX connection unavailable"),
            )
            .child(
                div()
                    .flex_1()
                    .text_color(cx.theme().muted_foreground)
                    .child(self.snapshot.status.clone()),
            )
            .child(
                Button::new("banner-reconnect")
                    .xsmall()
                    .danger()
                    .label("Reconnect")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.terminal
                            .update(cx, |terminal, cx| terminal.reconnect(cx));
                    })),
            )
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let live_label: SharedString = if self.snapshot.stale {
            "STALE".into()
        } else {
            "LIVE".into()
        };
        let live_color = if self.snapshot.stale {
            cx.theme().danger
        } else {
            cx.theme().success
        };

        StatusBar::new()
            .left(
                h_flex()
                    .gap_1()
                    .child(
                        Button::new("toggle-left-dock")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelLeft)
                            .tooltip("Toggle left dock")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dock_area.update(cx, |area, cx| {
                                    area.toggle_dock(DockPlacement::Left, window, cx);
                                });
                            })),
                    )
                    .child(
                        Button::new("toggle-bottom-dock")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelBottom)
                            .tooltip("Toggle bottom dock")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dock_area.update(cx, |area, cx| {
                                    area.toggle_dock(DockPlacement::Bottom, window, cx);
                                });
                            })),
                    )
                    .child(
                        Button::new("toggle-right-dock")
                            .ghost()
                            .xsmall()
                            .icon(IconName::PanelRight)
                            .tooltip("Toggle right dock")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.dock_area.update(cx, |area, cx| {
                                    area.toggle_dock(DockPlacement::Right, window, cx);
                                });
                            })),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .text_color(live_color)
                            .child("●")
                            .child(live_label),
                    ),
            )
            .child(
                div()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(self.snapshot.status.clone()),
            )
            .right(
                h_flex()
                    .gap_3()
                    .child(format!("PROFILE {}", self.snapshot.profile))
                    .child(format!("ROLE {}", self.snapshot.role))
                    .child(format!("FIX {}", self.snapshot.connection))
                    .child(format!("COMMIT {}", self.snapshot.sequence)),
            )
    }
}

impl Render for AppShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet_layer = Root::render_sheet_layer(window, cx);
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let notification_layer = Root::render_notification_layer(window, cx);

        v_flex()
            .id("bunting-workspace")
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_title_bar(cx))
            .when(self.snapshot.stale, |this| {
                this.child(self.render_connection_banner(cx))
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.dock_area.clone()),
            )
            .child(self.render_status_bar(cx))
            .children(sheet_layer)
            .children(dialog_layer)
            .children(notification_layer)
    }
}

pub fn root(shell: Entity<AppShell>, window: &mut Window, cx: &mut App) -> Entity<Root> {
    cx.new(|cx| Root::new(shell, window, cx))
}
