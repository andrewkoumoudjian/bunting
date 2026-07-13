// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::tui::{app::Tab, ui::styles};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Paragraph, Tabs},
};

pub fn render(frame: &mut Frame, area: Rect, tab: Tab) {
    let [tabs_area, actions_area] =
        Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)]).areas(area);
    let tabs = Tabs::new([
        Line::from(" BUNTING [1] "),
        Line::from(" ORDERS [2] "),
        Line::from(" FIX SESSION [3] "),
    ])
    .style(styles::text())
    .highlight_style(styles::selected())
    .divider("|")
    .select(match tab {
        Tab::Market => 0,
        Tab::Orders => 1,
        Tab::Fix => 2,
    });
    frame.render_widget(tabs, tabs_area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("B/S Trade ", styles::dim()),
            Span::styled("? Help ", styles::dim()),
            Span::styled("` Console ", styles::dim()),
            Span::styled("/ Command ", styles::dim()),
            Span::styled("Q Quit", styles::dim()),
        ]))
        .alignment(Alignment::Right),
        actions_area,
    );
}
