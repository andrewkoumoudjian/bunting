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
    symbols::Marker,
    text::{Line, Span},
    widgets::{Axis, Block, Borders, Cell, Chart, Dataset, GraphType, Paragraph, Row, Table},
};

pub fn render(frame: &mut Frame, area: Rect, app: &App, client: &FixClient) {
    let [levels_area, detail_area] =
        Layout::horizontal([Constraint::Percentage(27), Constraint::Percentage(73)]).areas(area);
    render_levels(frame, levels_area, app, client);

    let [summary_area, lower_area, actions_area] = Layout::vertical([
        Constraint::Length(8),
        Constraint::Min(8),
        Constraint::Length(3),
    ])
    .areas(detail_area);
    let [instrument_area, depth_area] =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
            .areas(summary_area);
    render_instrument(frame, instrument_area, client);
    render_depth(frame, depth_area, client);

    let [chart_area, reports_area] =
        Layout::horizontal([Constraint::Percentage(75), Constraint::Percentage(25)])
            .areas(lower_area);
    render_price_chart(frame, chart_area, client);
    render_reports(frame, reports_area, client);
    render_actions(frame, actions_area);
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

#[expect(
    clippy::float_arithmetic,
    reason = "Ratatui chart coordinates and padded display bounds require f64 arithmetic"
)]
fn render_price_chart(frame: &mut Frame, area: Rect, client: &FixClient) {
    let bids = chart_points(client.prices.iter().map(|sample| sample.bid));
    let asks = chart_points(client.prices.iter().map(|sample| sample.ask));
    let mids = bids
        .iter()
        .zip(&asks)
        .map(|(bid, ask)| (bid.0, bid.1.midpoint(ask.1)))
        .collect::<Vec<_>>();
    if mids.is_empty() {
        frame.render_widget(
            Paragraph::new("Waiting for the first FIX market-data snapshot...").block(
                Block::new()
                    .title(" LIVE PRICE · FIX SNAPSHOTS ")
                    .borders(Borders::ALL)
                    .border_style(styles::border()),
            ),
            area,
        );
        return;
    }
    let min_price = bids
        .iter()
        .chain(&asks)
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_price = bids
        .iter()
        .chain(&asks)
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let padding = ((max_price - min_price) * 0.12).max(1.0);
    let x_max = mids.last().map_or(1.0, |point| point.0.max(1.0));
    let datasets = vec![
        Dataset::default()
            .name("bid")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(styles::bid())
            .data(&bids),
        Dataset::default()
            .name("mid")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(styles::accent())
            .data(&mids),
        Dataset::default()
            .name("ask")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(styles::ask())
            .data(&asks),
    ];
    let chart = Chart::new(datasets)
        .block(
            Block::new()
                .title(" LIVE PRICE · OBSERVED BID / MID / ASK ")
                .title_bottom(Line::from(format!(
                    " {} FIX snapshots · latest mid {:.1} ",
                    mids.len(),
                    mids.last().map_or(0.0, |point| point.1)
                )))
                .borders(Borders::ALL)
                .border_style(styles::border()),
        )
        .x_axis(
            Axis::default()
                .bounds([0.0, x_max])
                .labels(["oldest", "latest"]),
        )
        .y_axis(
            Axis::default()
                .bounds([min_price - padding, max_price + padding])
                .labels([
                    Line::from(format!("{:.1}", min_price - padding)),
                    Line::from(format!("{:.1}", max_price + padding)),
                ]),
        );
    frame.render_widget(chart, area);
}

fn chart_points(values: impl Iterator<Item = i64>) -> Vec<(f64, f64)> {
    values
        .enumerate()
        .filter_map(|(index, value)| {
            let index = u32::try_from(index).ok().map(f64::from)?;
            let value = i32::try_from(value).ok().map(f64::from)?;
            Some((index, value))
        })
        .collect()
}

fn render_actions(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [B] BUY ", styles::bid()),
            Span::raw("  "),
            Span::styled(" [S] SELL ", styles::ask()),
            Span::raw("  [C] CANCEL   [M] REPLACE   [/] ADVANCED COMMAND "),
        ]))
        .block(
            Block::new()
                .title(" TRADE ACTIONS ")
                .borders(Borders::ALL)
                .border_style(styles::active_border()),
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
