// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::{
    protocol::FixClient,
    tui::{
        app::{App, Tab},
        popup::PopupKind,
        ui::{rect, styles},
        views,
        widgets::log_panel::LogPanel,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin},
    widgets::{Block, Borders},
};

pub fn draw(frame: &mut Frame, app: &App, client: &FixClient) {
    let area = frame.area();
    frame.render_widget(
        Block::new()
            .borders(Borders::ALL)
            .border_style(styles::border()),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let [navbar, content, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .areas(inner);

    views::navbar::render(frame, navbar, app.tab);
    match app.tab {
        Tab::Market => views::market::render(frame, content, app, client),
        Tab::Orders => views::orders::render(frame, content, client),
        Tab::Fix => LogPanel::render(frame, content, &client.logs, false),
    }
    views::footer::render(frame, footer, client);

    if app.popup == PopupKind::FixLog {
        LogPanel::render(
            frame,
            rect::centered_percent(88, 72, inner),
            &client.logs,
            true,
        );
    } else {
        views::popup::render(frame, inner, app);
    }
}
