pub struct MarketPanel {
    kind: PanelKind,
    terminal: Entity<Terminal>,
    focus_handle: FocusHandle,
    _terminal_observer: Subscription,
}

impl MarketPanel {
    pub fn new(kind: PanelKind, terminal: Entity<Terminal>, cx: &mut Context<Self>) -> Self {
        let terminal_observer = cx.observe(&terminal, |_, _, cx| cx.notify());
        Self {
            kind,
            terminal,
            focus_handle: cx.focus_handle(),
            _terminal_observer: terminal_observer,
        }
    }

    pub fn kind(&self) -> PanelKind {
        self.kind
    }
}

impl EventEmitter<PanelEvent> for MarketPanel {}

impl Focusable for MarketPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Panel for MarketPanel {
    fn panel_name(&self) -> &'static str {
        self.kind.panel_name()
    }

    fn tab_name(&self, _: &App) -> Option<SharedString> {
        Some(self.kind.tab_name().into())
    }

    fn title(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.terminal.read(cx).snapshot();
        h_flex()
            .gap_2()
            .child(status_dot(snapshot.connected, cx))
            .child(self.kind.title())
    }

    fn closable(&self, _: &App) -> bool {
        self.kind != PanelKind::Chart
    }

    fn zoomable(&self, _: &App) -> Option<PanelControl> {
        Some(PanelControl::Both)
    }

    fn inner_padding(&self, _: &App) -> bool {
        false
    }
}

impl Render for MarketPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(render_panel_body(self.kind, self.terminal.clone(), cx))
    }
}

fn render_panel_body(
    kind: PanelKind,
    terminal: Entity<Terminal>,
    cx: &mut Context<MarketPanel>,
) -> AnyElement {
    match kind {
        PanelKind::Chart => render_chart(terminal, cx),
        PanelKind::OrderBook => render_order_book(terminal, cx),
        PanelKind::OrderTicket => render_order_ticket(terminal, cx),
        PanelKind::Orders => render_orders(terminal, cx),
        PanelKind::Account => render_account(terminal, cx),
        PanelKind::News => render_news(terminal, cx),
        PanelKind::Tenders => render_tenders(terminal, cx),
        PanelKind::Risk => render_risk(terminal, cx),
        PanelKind::Competition => render_competition(terminal, cx),
        PanelKind::Session => render_session(terminal, cx),
    }
}

