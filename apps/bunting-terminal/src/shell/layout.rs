impl AppShell {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminal = cx.new(|cx| Terminal::new(window, cx));
        let snapshot = terminal.read(cx).snapshot();
        let dock_area = cx.new(|cx| {
            DockArea::new(
                "bunting-market-workspace",
                Some(DOCK_VERSION),
                window,
                cx,
            )
        });
        let panels = PanelKind::ALL
            .into_iter()
            .map(|kind| {
                let panel = cx.new(|cx| MarketPanel::new(kind, terminal.clone(), cx));
                (kind, panel)
            })
            .collect::<HashMap<_, _>>();

        let _terminal_observer = cx.observe(&terminal, |this, terminal, cx| {
            this.snapshot = terminal.read(cx).snapshot();
            cx.notify();
        });

        let local_server = LocalServerController::new();
        let server_snapshot = local_server.snapshot();
        let mut shell = Self {
            terminal,
            dock_area,
            panels,
            snapshot,
            local_server,
            server_snapshot,
            _terminal_observer,
        };
        shell.reset_workspace(WorkspacePreset::Trading, window, cx);
        shell.schedule_local_server_poll(cx);
        shell
    }

    fn panel(&self, kind: PanelKind) -> Entity<MarketPanel> {
        self.panels
            .get(&kind)
            .cloned()
            .expect("every market panel is initialized")
    }

    fn tabs(
        &self,
        kinds: &[PanelKind],
        active_ix: usize,
        dock_area: &gpui::WeakEntity<DockArea>,
        window: &mut Window,
        cx: &mut App,
    ) -> DockItem {
        let panels = kinds
            .iter()
            .map(|kind| Arc::new(self.panel(*kind)) as Arc<dyn PanelView>)
            .collect::<Vec<_>>();
        DockItem::tabs(panels, dock_area, window, cx).active_index(active_ix, cx)
    }

    fn reset_workspace(
        &mut self,
        preset: WorkspacePreset,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.terminal.update(cx, |terminal, cx| {
            terminal.set_active_preset(preset, cx);
        });

        let weak_dock_area = self.dock_area.downgrade();
        let (center, left, right, bottom, left_width, right_width, bottom_height) = match preset {
            WorkspacePreset::Trading => (
                DockItem::tab(self.panel(PanelKind::Chart), &weak_dock_area, window, cx),
                self.tabs(
                    &[PanelKind::OrderBook, PanelKind::Tenders],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                self.tabs(
                    &[PanelKind::OrderTicket, PanelKind::Account, PanelKind::Risk],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                self.tabs(
                    &[PanelKind::Orders, PanelKind::News, PanelKind::Session],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                px(330.),
                px(380.),
                px(280.),
            ),
            WorkspacePreset::Research => (
                DockItem::tab(self.panel(PanelKind::Chart), &weak_dock_area, window, cx),
                self.tabs(
                    &[PanelKind::News, PanelKind::OrderBook],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                self.tabs(
                    &[PanelKind::Account, PanelKind::Risk],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                self.tabs(
                    &[PanelKind::Session, PanelKind::Orders],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                px(370.),
                px(360.),
                px(250.),
            ),
            WorkspacePreset::Competition => (
                DockItem::tab(self.panel(PanelKind::Chart), &weak_dock_area, window, cx),
                self.tabs(
                    &[PanelKind::Competition, PanelKind::Tenders],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                self.tabs(
                    &[PanelKind::Account, PanelKind::Risk, PanelKind::OrderTicket],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                self.tabs(
                    &[PanelKind::Orders, PanelKind::News, PanelKind::Session],
                    0,
                    &weak_dock_area,
                    window,
                    cx,
                ),
                px(390.),
                px(370.),
                px(280.),
            ),
        };

        self.dock_area.update(cx, |area, cx| {
            area.set_version(DOCK_VERSION, window, cx);
            area.set_center(center, window, cx);
            area.set_left_dock(left, Some(left_width), true, window, cx);
            area.set_right_dock(right, Some(right_width), true, window, cx);
            area.set_bottom_dock(bottom, Some(bottom_height), true, window, cx);
            area.set_dock_collapsible(
                Edges {
                    left: true,
                    bottom: true,
                    right: true,
                    ..Default::default()
                },
                window,
                cx,
            );
        });
    }

    fn schedule_local_server_poll(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                let Some(this) = this.upgrade() else {
                    break;
                };
                this.update(cx, |shell, cx| {
                    let became_ready = shell.local_server.poll();
                    shell.server_snapshot = shell.local_server.snapshot();
                    if became_ready {
                        shell.terminal.update(cx, |terminal, cx| {
                            terminal.set_status(
                                "Local WASM server ready; reconnecting the FIX session",
                                cx,
                            );
                            terminal.reconnect(cx);
                        });
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn start_local_server(&mut self, cx: &mut Context<Self>) {
        let result = self.local_server.start();
        self.server_snapshot = self.local_server.snapshot();
        match result {
            Ok(status) => {
                self.terminal
                    .update(cx, |terminal, cx| terminal.set_status(status, cx));
                if self.local_server.is_ready() {
                    self.terminal
                        .update(cx, |terminal, cx| terminal.reconnect(cx));
                }
            }
            Err(error) => {
                self.terminal
                    .update(cx, |terminal, cx| terminal.set_status(error, cx));
            }
        }
        cx.notify();
    }

    fn stop_local_server(&mut self, cx: &mut Context<Self>) {
        let result = self.local_server.stop();
        self.server_snapshot = self.local_server.snapshot();
        let status = result.unwrap_or_else(|error| error);
        self.terminal
            .update(cx, |terminal, cx| terminal.set_status(status, cx));
        cx.notify();
    }

    fn toggle_local_server(&mut self, cx: &mut Context<Self>) {
        if self.local_server.is_owned() {
            self.stop_local_server(cx);
        } else {
            self.start_local_server(cx);
        }
    }

    fn execute_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.terminal.read(cx).command_input();
        let command = input.read(cx).value().trim().to_ascii_uppercase();
        match command.as_str() {
            "TRADING" | "TRADE" => self.reset_workspace(WorkspacePreset::Trading, window, cx),
            "RESEARCH" | "RES" => self.reset_workspace(WorkspacePreset::Research, window, cx),
            "COMPETITION" | "COMP" | "RIT" => {
                self.reset_workspace(WorkspacePreset::Competition, window, cx);
            }
            "SERVER" | "START SERVER" | "SERVER START" | "WASM" => {
                self.start_local_server(cx);
            }
            "STOP SERVER" | "SERVER STOP" => self.stop_local_server(cx),
            "RECONNECT" => self.terminal.update(cx, |terminal, cx| terminal.reconnect(cx)),
            "REFRESH" | "GO" | "BNT" | "BOOK" => {
                self.terminal.update(cx, |terminal, cx| terminal.refresh(cx));
            }
            _ => self.terminal.update(cx, |terminal, cx| {
                terminal.set_status(format!("Unknown command: {command}"), cx);
            }),
        }
    }

    fn switch_workspace(
        &mut self,
        preset: WorkspacePreset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reset_workspace(preset, window, cx);
    }
}
