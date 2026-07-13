// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use ratatui::style::{Color, Modifier, Style};

pub const fn text() -> Style {
    Style::new().fg(Color::Reset)
}

pub const fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub const fn label() -> Style {
    Style::new().fg(Color::Gray)
}

pub const fn border() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub const fn active_border() -> Style {
    Style::new().fg(Color::Gray)
}

pub const fn selected() -> Style {
    Style::new().fg(Color::Black).bg(Color::LightMagenta)
}

pub const fn bid() -> Style {
    Style::new().fg(Color::LightGreen)
}

pub const fn ask() -> Style {
    Style::new().fg(Color::LightRed)
}

pub const fn accent() -> Style {
    Style::new()
        .fg(Color::LightCyan)
        .add_modifier(Modifier::BOLD)
}

pub const fn warning() -> Style {
    Style::new().fg(Color::Yellow)
}

pub const fn online() -> Style {
    Style::new().fg(Color::LightGreen)
}

pub const fn offline() -> Style {
    Style::new().fg(Color::LightRed)
}
