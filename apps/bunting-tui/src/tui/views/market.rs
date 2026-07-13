// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::{
    protocol::FixClient,
    tui::{app::App, ui::styles},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

pub fn render(frame: &mut Frame, area: Rect, app: &App, client: &FixClient) {
    let [levels_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(31), Constraint::Percentage(69)]).areas(area);
    render_levels(frame, levels_area, app, client);

    let [summary_area, lower_area] =
        Layout::vertical([Constraint::Length(9), Constraint::Min(10)]).areas(detail_area);
    let [instrument_area, depth_area] =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
            .areas(summary_area);
    render_instrument(frame, instrument_area, client);
    render_depth(frame, depth_area, client);

    let [ladder_area, reports_area] =
        Layout::horizontal([Constraint::Percentage(72), Constraint::Percentage(28)])
            .areas(lower_area);
    render_ladder(frame, ladder_area, client);
    render_reports(frame, reports_area, client);
}

fn render_levels(frame: &mut Frame, area: Rect, app: &App, client: &FixClient) {
    let rows = client
        .book
        .asks
        .iter()
        .rev()
        .map(|(price, quantity)| ("ASK", *price, *quantity, styles::ask()))
        .chain(
            client
                .book
                .bids
                .iter()
                .map(|(price, quantity)| ("BID", *price, *quantity, styles::bid())),
        )
        .enumerate()
        .map(|(index, (side, price, quantity, style))| {
            let row = Row::new([
                Cell::from(side),
                Cell::from(index.to_string()),
                Cell::from(price.to_string()),
                Cell::from(quantity.to_string()),
            ])
            .style(style);
            if index == app.selected_level {
                row.style(styles::selected())
            } else {
                row
            }
        });
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Min(8),
        ],
    )
    .header(Row::new(["SIDE", "LVL", "PRICE", "QTY"]).style(styles::label()))
    .block(
        Block::new()
            .title(" BUNTING MARKET [↑↓] ")
            .borders(Borders::ALL)
            .border_style(styles::border()),
    )
    .column_spacing(1);
    frame.render_widget(table, area);
}

fn render_instrument(frame: &mut Frame, area: Rect, client: &FixClient) {
    let best_bid = client.book.bids.first().copied();
    let best_ask = client.book.asks.first().copied();
    let spread = best_bid
        .zip(best_ask)
        .map(|(bid, ask)| ask.0.saturating_sub(bid.0));
    let lines = vec![
        Line::from(vec![
            Span::styled("BUNT ", styles::accent()),
            Span::styled("Bunting Local Market · FIX 4.4", styles::label()),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Status: ", styles::label()),
            Span::styled("Trading", styles::online()),
            Span::raw("            "),
            Span::styled("Session: ", styles::label()),
            Span::styled(&client.status, styles::text()),
        ]),
        Line::from(vec![
            Span::styled("Best bid: ", styles::label()),
            Span::styled(display_level(best_bid), styles::bid()),
            Span::raw("    "),
            Span::styled("Best ask: ", styles::label()),
            Span::styled(display_level(best_ask), styles::ask()),
            Span::raw("    "),
            Span::styled("Spread: ", styles::label()),
            Span::raw(spread.map_or_else(|| "--".to_owned(), |value| value.to_string())),
        ]),
        Line::from(vec![
            Span::styled("Book sequence: ", styles::label()),
            Span::raw(&client.book_sequence),
            Span::raw("    "),
            Span::styled("Depth: ", styles::label()),
            Span::raw(format!(
                "{} bids / {} asks",
                client.book.bids.len(),
                client.book.asks.len()
            )),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .borders(Borders::ALL)
                .border_style(styles::active_border()),
        ),
        area,
    );
}

fn render_depth(frame: &mut Frame, area: Rect, client: &FixClient) {
    let depth = client.book.bids.len().max(client.book.asks.len());
    let rows = (0..depth).map(|index| {
        let bid = client.book.bids.get(index).copied();
        let ask = client.book.asks.get(index).copied();
        Row::new([
            Cell::from(bid.map_or_else(String::new, |level| level.1.to_string())),
            Cell::from(bid.map_or_else(String::new, |level| level.0.to_string()))
                .style(styles::bid()),
            Cell::from(ask.map_or_else(String::new, |level| level.0.to_string()))
                .style(styles::ask()),
            Cell::from(ask.map_or_else(String::new, |level| level.1.to_string())),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Length(7),
                Constraint::Min(6),
                Constraint::Min(6),
                Constraint::Length(7),
            ],
        )
        .header(Row::new(["QTY", "BID", "ASK", "QTY"]).style(styles::label()))
        .block(
            Block::new()
                .title(" ORDER BOOK ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
}

fn render_ladder(frame: &mut Frame, area: Rect, client: &FixClient) {
    let max_quantity = client
        .book
        .bids
        .iter()
        .chain(&client.book.asks)
        .map(|level| level.1)
        .max()
        .unwrap_or(1)
        .max(1);
    let max_bar = usize::from(area.width.saturating_sub(24).min(48));
    let lines = client
        .book
        .asks
        .iter()
        .rev()
        .map(|level| ("ASK", *level, styles::ask()))
        .chain(
            client
                .book
                .bids
                .iter()
                .map(|level| ("BID", *level, styles::bid())),
        )
        .map(|(side, (price, quantity), style)| {
            let scaled =
                quantity.saturating_mul(i64::try_from(max_bar).unwrap_or(i64::MAX)) / max_quantity;
            let bar = "█".repeat(usize::try_from(scaled).unwrap_or(max_bar).min(max_bar));
            Line::from(vec![
                Span::styled(format!("{side:>3} {price:>8} {quantity:>8} "), style),
                Span::styled(bar, style),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::new()
                .title(" ENGINE DEPTH LADDER ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
}

fn render_reports(frame: &mut Frame, area: Rect, client: &FixClient) {
    let rows = client.executions.iter().rev().map(|report| {
        Row::new([
            Cell::from(report.order_id.as_str()),
            Cell::from(report.kind.as_str()),
            Cell::from(report.order_status.as_str()),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Min(7),
                Constraint::Length(5),
                Constraint::Length(6),
            ],
        )
        .header(Row::new(["ORDER", "EXEC", "STATUS"]).style(styles::label()))
        .block(
            Block::new()
                .title(" EXECUTION REPORTS ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
}

fn display_level(level: Option<(i64, i64)>) -> String {
    level.map_or_else(
        || "--".to_owned(),
        |(price, quantity)| format!("{price} × {quantity}"),
    )
}
