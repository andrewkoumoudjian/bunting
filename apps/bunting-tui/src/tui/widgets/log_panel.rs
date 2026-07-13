// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Filesystem log discovery was replaced with bounded in-memory FIX frames.
// Rust guideline compliant 2026-02-21

use crate::tui::ui::styles;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use std::collections::VecDeque;

pub struct LogPanel;

impl LogPanel {
    pub fn render(frame: &mut Frame, area: Rect, logs: &VecDeque<String>, overlay: bool) {
        if overlay {
            frame.render_widget(Clear, area);
        }
        let block = Block::new()
            .title(if overlay {
                " Raw FIX Console [`] "
            } else {
                " Raw FIX 4.4 Session Journal "
            })
            .borders(Borders::ALL)
            .border_style(if overlay {
                styles::warning()
            } else {
                styles::active_border()
            })
            .style(Style::new().bg(Color::Black));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let lines = logs
            .iter()
            .rev()
            .take(usize::from(inner.height))
            .rev()
            .map(|line| {
                let style = if line.starts_with("IN ") {
                    styles::bid()
                } else if line.starts_with("OUT") {
                    styles::accent()
                } else {
                    styles::label()
                };
                Line::from(Span::styled(line, style))
            })
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}
