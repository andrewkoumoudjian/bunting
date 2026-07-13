// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::tui::ui::{rect, styles};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
};

pub fn render(frame: &mut Frame, area: Rect) {
    let popup = rect::centered(86, 29, area);
    let lines = vec![
        Line::from(""),
        Line::styled(
            format!("Bunting Terminal v{}", env!("CARGO_PKG_VERSION")),
            Style::new().add_modifier(Modifier::BOLD),
        ),
        Line::from("FIX 4.4/TCP operator workstation backed by bunting-engine"),
        Line::from(""),
        Line::from("General ----------------------------------------------------------"),
        Line::from("? / F1          Show or close this help"),
        Line::from("`               Toggle raw FIX console overlay"),
        Line::from("1 / 2 / 3       Market, orders, or FIX-session tab"),
        Line::from("/               Open command entry"),
        Line::from("R               Refresh the engine book through FIX V"),
        Line::from("Q / Ctrl-C      Quit"),
        Line::from(""),
        Line::from("Trading ----------------------------------------------------------"),
        Line::from("B               Limit buy popup: buy PRICE QTY"),
        Line::from("S               Limit sell popup: sell PRICE QTY"),
        Line::from("C               Cancel popup: cancel ORDER_ID"),
        Line::from("M               Replace popup: replace OLD NEW PRICE QTY"),
        Line::from("market SIDE QTY Submit a market buy or sell"),
        Line::from("status ORDER_ID Request current order status"),
        Line::from("logout          Orderly FIX logout"),
        Line::from(""),
        Line::from(Span::styled(
            "Every action is sent through the FIX client; the TUI never mutates the engine.",
            styles::dim(),
        )),
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).style(styles::text()).block(
            Block::new()
                .title(Span::styled(" Help ", styles::accent()))
                .borders(Borders::ALL)
                .border_style(styles::active_border())
                .padding(Padding::horizontal(2))
                .style(Style::new().bg(Color::Black)),
        ),
        popup,
    );
}
