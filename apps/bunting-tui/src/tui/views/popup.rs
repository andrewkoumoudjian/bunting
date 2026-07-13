// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::tui::{app::App, popup::PopupKind, ui::styles};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, Clear, Paragraph},
};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    match app.popup {
        PopupKind::Help => super::help::render(frame, area),
        PopupKind::Command => render_command(frame, area, app),
        PopupKind::None | PopupKind::FixLog => {}
    }
}

fn render_command(frame: &mut Frame, area: Rect, app: &App) {
    let width = 76_u16.min(area.width.saturating_sub(4));
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(3) / 2,
        width,
        3,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(app.input.as_str()).block(
            Block::new()
                .title(" Bunting command · Enter submit · Esc close ")
                .borders(Borders::ALL)
                .border_style(styles::active_border())
                .style(Style::new().bg(Color::Black)),
        ),
        popup,
    );
    let cursor_offset = u16::try_from(app.input.chars().count()).unwrap_or(u16::MAX);
    frame.set_cursor_position((
        popup.x + cursor_offset.min(popup.width.saturating_sub(2)) + 1,
        popup.y + 1,
    ));
}
