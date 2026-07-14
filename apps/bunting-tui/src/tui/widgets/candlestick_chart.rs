// Modified from cli-candlestick-chart's Longbridge TUI renderer at commit
// 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright (c) 2021 Julien-R44 <julien@ripouteau.com>. Licensed under MIT.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};
use unicode_width::UnicodeWidthStr;

/// Renders the ANSI output produced by Longbridge's candlestick component into a Ratatui buffer.
pub struct AnsiChart<'a>(pub &'a str);

impl Widget for AnsiChart<'_> {
    fn render(self, area: Rect, buffer: &mut Buffer) {
        for (row, line) in self
            .0
            .lines()
            .skip_while(|line| line.is_empty())
            .take(usize::from(area.height))
            .enumerate()
        {
            let y = area
                .y
                .saturating_add(u16::try_from(row).unwrap_or(u16::MAX));
            let mut x = area.x;
            let mut style = Style::default();
            let mut remainder = line;
            while let Some(escape) = remainder.find("\u{1b}[") {
                write_text(&remainder[..escape], area, y, &mut x, style, buffer);
                let sequence = &remainder[escape + 2..];
                let Some(end) = sequence.find('m') else {
                    break;
                };
                let codes = sequence[..end]
                    .split(';')
                    .filter_map(|code| code.parse::<u8>().ok())
                    .collect::<Vec<_>>();
                apply_graphics_mode(&mut style, &codes);
                remainder = &sequence[end + 1..];
            }
            if x < area.right() {
                write_text(remainder, area, y, &mut x, style, buffer);
            }
        }
    }
}

fn write_text(text: &str, area: Rect, y: u16, x: &mut u16, style: Style, buffer: &mut Buffer) {
    let remaining = usize::from(area.right().saturating_sub(*x));
    if remaining > 0 {
        buffer.set_stringn(*x, y, text, remaining, style);
        let width = UnicodeWidthStr::width(text).min(remaining);
        *x = x.saturating_add(u16::try_from(width).unwrap_or(u16::MAX));
    }
}

fn apply_graphics_mode(style: &mut Style, codes: &[u8]) {
    let mut index = 0;
    while index < codes.len() {
        match codes[index] {
            0 => *style = Style::default(),
            1 => *style = style.add_modifier(Modifier::BOLD),
            22 => *style = style.remove_modifier(Modifier::BOLD),
            30..=37 => style.fg = Some(ansi_color(codes[index] - 30, false)),
            39 => style.fg = None,
            40..=47 => style.bg = Some(ansi_color(codes[index] - 40, false)),
            49 => style.bg = None,
            90..=97 => style.fg = Some(ansi_color(codes[index] - 90, true)),
            100..=107 => style.bg = Some(ansi_color(codes[index] - 100, true)),
            38 | 48 => {
                let foreground = codes[index] == 38;
                if codes.get(index + 1) == Some(&2)
                    && let Some((&red, rest)) = codes.get(index + 2).zip(codes.get(index + 3..))
                    && let [green, blue, ..] = rest
                {
                    let color = Color::Rgb(red, *green, *blue);
                    if foreground {
                        style.fg = Some(color);
                    } else {
                        style.bg = Some(color);
                    }
                    index += 4;
                } else if codes.get(index + 1) == Some(&5)
                    && let Some(color) = codes.get(index + 2)
                {
                    if foreground {
                        style.fg = Some(Color::Indexed(*color));
                    } else {
                        style.bg = Some(Color::Indexed(*color));
                    }
                    index += 2;
                }
            }
            _ => {}
        }
        index += 1;
    }
}

const fn ansi_color(code: u8, bright: bool) -> Color {
    match (code, bright) {
        (0, false) => Color::Black,
        (1, false) => Color::Red,
        (2, false) => Color::Green,
        (3, false) => Color::Yellow,
        (4, false) => Color::Blue,
        (5, false) => Color::Magenta,
        (6, false) => Color::Cyan,
        (7, false) => Color::Gray,
        (0, true) => Color::DarkGray,
        (1, true) => Color::LightRed,
        (2, true) => Color::LightGreen,
        (3, true) => Color::LightYellow,
        (4, true) => Color::LightBlue,
        (5, true) => Color::LightMagenta,
        (6, true) => Color::LightCyan,
        _ => Color::White,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_truecolor_chart_output_without_escape_text() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
        AnsiChart("\u{1b}[38;2;52;208;88mup\u{1b}[0m").render(buffer.area, &mut buffer);
        assert_eq!(buffer[(0, 0)].symbol(), "u");
        assert_eq!(buffer[(0, 0)].fg, Color::Rgb(52, 208, 88));
        assert_eq!(buffer[(1, 0)].symbol(), "p");
    }
}
