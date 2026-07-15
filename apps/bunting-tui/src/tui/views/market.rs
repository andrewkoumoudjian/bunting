// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::{
    protocol::FixClient,
    tui::{app::App, ui::styles, widgets::candlestick_chart::AnsiChart},
};
use cli_candlestick_chart::{Candle, Chart};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

const SAMPLES_PER_CANDLE: usize = 4;

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
    let [instrument_area, portfolio_area] =
        Layout::horizontal([Constraint::Percentage(68), Constraint::Percentage(32)])
            .areas(summary_area);
    render_instrument(frame, instrument_area, client);
    render_portfolio(frame, portfolio_area, client);

    let [chart_area, reports_area] =
        Layout::horizontal([Constraint::Percentage(75), Constraint::Percentage(25)])
            .areas(lower_area);
    render_price_chart(frame, chart_area, client);
    render_reports(frame, reports_area, client);
    render_actions(frame, actions_area, app);
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
            .title(" LIVE FIX ORDER BOOK [↑↓ · ENTER TAKE · B/S PLACE · X QTY] ")
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
            Span::styled(
                format!(
                    "{} · {} · FIX 4.4",
                    client.profile_name,
                    client.profile().transport.label()
                ),
                styles::label(),
            ),
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

fn render_portfolio(frame: &mut Frame, area: Rect, client: &FixClient) {
    let mark = client
        .book
        .bids
        .first()
        .zip(client.book.asks.first())
        .map(|(bid, ask)| bid.0.saturating_add(ask.0) / 2);
    let marked_value = mark.map(|price| client.portfolio.marked_value(price));
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Position  ", styles::label()),
                Span::raw(client.portfolio.position.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Cash      ", styles::label()),
                Span::raw(client.portfolio.cash.to_string()),
            ]),
            Line::from(vec![
                Span::styled("Mark      ", styles::label()),
                Span::raw(mark.map_or_else(|| "--".to_owned(), |value| value.to_string())),
            ]),
            Line::from(vec![
                Span::styled("P&L       ", styles::label()),
                Span::styled(
                    marked_value.map_or_else(|| "--".to_owned(), |value| value.to_string()),
                    if marked_value.is_some_and(|value| value < 0) {
                        styles::ask()
                    } else {
                        styles::bid()
                    },
                ),
            ]),
            Line::from(format!(
                "Bought {} · Sold {}",
                client.portfolio.bought, client.portfolio.sold
            )),
            Line::from(format!(
                "Last fill {}",
                client
                    .portfolio
                    .last_fill_price
                    .map_or_else(|| "--".to_owned(), |value| value.to_string())
            )),
        ])
        .block(
            Block::new()
                .title(" PORTFOLIO · FIX FILLS ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
}

fn render_price_chart(frame: &mut Frame, area: Rect, client: &FixClient) {
    let candles = candles(&client.prices);
    if candles.is_empty() {
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
    let block = Block::new()
        .title(" FIX ORDER BOOK · LONGBRIDGE CANDLESTICK CHART ")
        .borders(Borders::ALL)
        .border_style(styles::border());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width < 18 || inner.height < 8 {
        frame.render_widget(Paragraph::new("Chart area is too small"), inner);
        return;
    }
    let mut chart = Chart::new_with_size(candles, (inner.width, inner.height));
    chart.set_name("BUNTING FIX BOOK".to_owned());
    let output = chart.render();
    frame.render_widget(AnsiChart(&output), inner);
}

fn candles(samples: &std::collections::VecDeque<crate::protocol::PriceSample>) -> Vec<Candle> {
    let samples = samples.iter().copied().collect::<Vec<_>>();
    samples
        .chunks(SAMPLES_PER_CANDLE)
        .filter_map(|samples| {
            let first = samples.first()?;
            let last = samples.last()?;
            let midpoint = |sample: &crate::protocol::PriceSample| {
                let bid = i32::try_from(sample.bid).ok().map(f64::from)?;
                let ask = i32::try_from(sample.ask).ok().map(f64::from)?;
                Some(bid.midpoint(ask))
            };
            let high = samples
                .iter()
                .filter_map(|sample| i32::try_from(sample.ask).ok().map(f64::from))
                .fold(f64::NEG_INFINITY, f64::max);
            let low = samples
                .iter()
                .filter_map(|sample| i32::try_from(sample.bid).ok().map(f64::from))
                .fold(f64::INFINITY, f64::min);
            let volume = samples.iter().try_fold(0_i64, |total, sample| {
                total
                    .checked_add(sample.bid_quantity)?
                    .checked_add(sample.ask_quantity)
            })?;
            Some(Candle::new(
                midpoint(first)?,
                high,
                low,
                midpoint(last)?,
                Some(i32::try_from(volume).ok().map(f64::from)?),
                None,
            ))
        })
        .collect()
}

fn render_actions(frame: &mut Frame, area: Rect, app: &App) {
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" [X] QTY {} ", app.order_quantity),
                styles::warning(),
            ),
            Span::raw("  "),
            Span::styled(" [ENTER] TAKE ", styles::accent()),
            Span::raw("  "),
            Span::styled(" [B] BUY ", styles::bid()),
            Span::raw("  "),
            Span::styled(" [S] SELL ", styles::ask()),
            Span::raw("  [/] COMMAND "),
            Span::styled(" limit/market mapping active ", styles::dim()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::PriceSample;
    use std::collections::VecDeque;

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "integer book ticks and midpoint inputs produce exact binary halves"
    )]
    fn builds_ohlc_candles_from_fix_midpoints() {
        let samples = VecDeque::from([
            PriceSample {
                bid: 99,
                ask: 101,
                bid_quantity: 4,
                ask_quantity: 6,
            },
            PriceSample {
                bid: 101,
                ask: 103,
                bid_quantity: 5,
                ask_quantity: 5,
            },
            PriceSample {
                bid: 98,
                ask: 100,
                bid_quantity: 3,
                ask_quantity: 7,
            },
            PriceSample {
                bid: 100,
                ask: 102,
                bid_quantity: 2,
                ask_quantity: 8,
            },
        ]);
        let candles = candles(&samples);
        assert_eq!(candles.len(), 1);
        assert_eq!(candles[0].open, 100.0);
        assert_eq!(candles[0].high, 103.0);
        assert_eq!(candles[0].low, 98.0);
        assert_eq!(candles[0].close, 101.0);
        assert_eq!(candles[0].volume, Some(40.0));

        let mut chart = Chart::new_with_size(candles, (80, 20));
        chart.set_name("BUNTING FIX BOOK".to_owned());
        let rendered = chart.render();
        assert!(rendered.contains("Price:"));
        assert!(rendered.contains("Cum. Vol"));
    }
}
