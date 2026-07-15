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
        Layout::horizontal([Constraint::Percentage(76), Constraint::Percentage(24)]).areas(area);
    let tabs = Tabs::new([
        Line::from(" MKT[1] "),
        Line::from(" ORD[2] "),
        Line::from(" ACCT[3] "),
        Line::from(" SIM[4] "),
        Line::from(" COLLAB[5] "),
        Line::from(" ADMIN[6] "),
        Line::from(" SESSION[7] "),
    ])
    .style(styles::text())
    .highlight_style(styles::selected())
    .divider("|")
    .select(match tab {
        Tab::Market => 0,
        Tab::Orders => 1,
        Tab::Account => 2,
        Tab::Simulation => 3,
        Tab::Collaboration => 4,
        Tab::Administration => 5,
        Tab::Session => 6,
    });
    frame.render_widget(tabs, tabs_area);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("? Help ", styles::dim()),
            Span::styled("` Console ", styles::dim()),
            Span::styled("/ Command ", styles::dim()),
            Span::styled("Q", styles::dim()),
        ]))
        .alignment(Alignment::Right),
        actions_area,
    );
}
