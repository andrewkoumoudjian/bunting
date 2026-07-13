// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::{protocol::FixClient, tui::ui::styles};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

pub fn render(frame: &mut Frame, area: Rect, client: &FixClient) {
    let [table_area, actions_area] =
        Layout::vertical([Constraint::Min(8), Constraint::Length(8)]).areas(area);
    let rows = client.executions.iter().rev().map(|report| {
        Row::new([
            Cell::from(report.order_id.as_str()),
            Cell::from(report.kind.as_str()),
            Cell::from(report.order_status.as_str()),
            Cell::from(report.reason.as_str()),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(18),
                Constraint::Length(12),
                Constraint::Length(12),
                Constraint::Min(20),
            ],
        )
        .header(Row::new(["ORDER ID", "EXEC TYPE", "STATUS", "REASON"]).style(styles::label()))
        .block(
            Block::new()
                .title(" FIX EXECUTION REPORT JOURNAL ")
                .borders(Borders::ALL)
                .border_style(styles::active_border()),
        ),
        table_area,
    );
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("B", styles::bid()),
                Span::raw(" buy  ·  "),
                Span::styled("S", styles::ask()),
                Span::raw(" sell  ·  C cancel  ·  M replace  ·  / any command"),
            ]),
            Line::from("buy PRICE QTY | sell PRICE QTY | market buy|sell QTY"),
            Line::from("cancel ORDER_ID | replace OLD NEW PRICE QTY | status ORDER_ID"),
            Line::from(Span::styled(&client.status, styles::dim())),
        ])
        .block(
            Block::new()
                .title(" OPERATOR ACTIONS ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        actions_area,
    );
}
