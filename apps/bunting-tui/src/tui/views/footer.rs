// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::{protocol::FixClient, tui::ui::styles};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::Paragraph,
};
use simfix_session::ConnectionState;

pub fn render(frame: &mut Frame, area: Rect, client: &FixClient) {
    let [market_area, connection_area] =
        Layout::horizontal([Constraint::Percentage(90), Constraint::Percentage(10)]).areas(area);
    let bid = client.book.bids.first().map(|level| level.0);
    let ask = client.book.asks.first().map(|level| level.0);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("BUNT ", styles::accent()),
            Span::styled(format!("bid {} ", display_price(bid)), styles::bid()),
            Span::styled(format!("ask {} ", display_price(ask)), styles::ask()),
            Span::styled(
                format!(
                    "| book seq {} | reports {} | FIX frames {}",
                    client.book_sequence,
                    client.executions.len(),
                    client.logs.len()
                ),
                styles::dim(),
            ),
        ])),
        market_area,
    );
    let established = client.connection_state() == ConnectionState::Established;
    frame.render_widget(
        Paragraph::new(Span::styled(
            if established {
                "■■■"
            } else {
                "□□□"
            },
            if established {
                styles::online()
            } else {
                styles::offline()
            },
        ))
        .alignment(Alignment::Right),
        connection_area,
    );
}

fn display_price(price: Option<i64>) -> String {
    price.map_or_else(|| "--".to_owned(), |value| value.to_string())
}
