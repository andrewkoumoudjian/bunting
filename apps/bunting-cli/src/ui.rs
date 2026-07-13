// Header/body/footer layout adapted from makeev/alphai-tui src/ui/mod.rs at
// f814697c6159d76b2dfb503ba5201b8c3fb702ad under the MIT license.

use crate::{
    keymap::{self, Action},
    protocol::{FixClient, book_request, cancel, new_order, replace, status},
    theme::Theme,
};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Row, Table, Wrap},
};
use simfix_session::ConnectionState;
use std::{collections::VecDeque, io, time::Duration};

pub async fn run(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = FixClient::connect(address).await?;
    let mut app = App::default();
    let mut terminal = TerminalSession::new()?;
    loop {
        Box::pin(client.poll()).await?;
        if client.connection_state() == ConnectionState::Established && !app.book_requested {
            client.send(book_request(app.allocate_id())).await?;
            app.book_requested = true;
        }
        app.status.clone_from(&client.status);
        terminal.terminal.draw(|frame| draw(frame, &app, &client))?;
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
            && handle_key(keymap::resolve(key), &mut app, &mut client).await?
        {
            break;
        }
    }
    Ok(())
}

#[derive(Default)]
struct App {
    input: String,
    status: String,
    next_id: u128,
    book_requested: bool,
    show_help: bool,
}

impl App {
    fn allocate_id(&mut self) -> u128 {
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.next_id
    }
}

async fn handle_key(
    action: Action,
    app: &mut App,
    client: &mut FixClient,
) -> Result<bool, Box<dyn std::error::Error>> {
    match action {
        Action::Quit => return Ok(true),
        Action::Submit => {
            let input = std::mem::take(&mut app.input);
            match parse_command(&input, app) {
                Ok(Command::Send(message)) => client.send(message).await?,
                Ok(Command::Logout) => client.logout().await?,
                Ok(Command::Quit) => return Ok(true),
                Ok(Command::None) => {}
                Err(error) => app.status = error,
            }
        }
        Action::Clear => app.input.clear(),
        Action::Backspace => {
            app.input.pop();
        }
        Action::ToggleHelp => app.show_help = !app.show_help,
        Action::Character(character) => {
            if app.input.len() < 256 {
                app.input.push(character);
            }
        }
        Action::None => {}
    }
    Ok(false)
}

enum Command {
    Send(simfix_wire::FixMessage),
    Logout,
    Quit,
    None,
}

fn parse_command(input: &str, app: &mut App) -> Result<Command, String> {
    let parts: Vec<_> = input.split_whitespace().collect();
    let Some(command) = parts.first().copied() else {
        return Ok(Command::None);
    };
    let number = |index: usize, name: &str| {
        parts
            .get(index)
            .ok_or_else(|| format!("missing {name}"))?
            .parse::<i64>()
            .map_err(|_| format!("invalid {name}"))
    };
    let identifier = |index: usize, name: &str| {
        parts
            .get(index)
            .ok_or_else(|| format!("missing {name}"))?
            .parse::<u128>()
            .map_err(|_| format!("invalid {name}"))
    };
    match command.to_ascii_lowercase().as_str() {
        "buy" | "sell" => {
            let price = number(1, "price")?;
            let quantity = number(2, "quantity")?;
            let id = app.allocate_id();
            Ok(Command::Send(new_order(id, command, quantity, Some(price))))
        }
        "market" => {
            let side = parts.get(1).copied().ok_or("missing side")?;
            if !matches!(side, "buy" | "sell") {
                return Err("side must be buy or sell".to_owned());
            }
            let quantity = number(2, "quantity")?;
            let id = app.allocate_id();
            Ok(Command::Send(new_order(id, side, quantity, None)))
        }
        "cancel" => {
            let id = identifier(1, "order id")?;
            let request_id = app.allocate_id();
            Ok(Command::Send(cancel(id, request_id)))
        }
        "replace" => {
            let old_id = identifier(1, "old order id")?;
            let new_id = identifier(2, "new order id")?;
            let price = number(3, "price")?;
            let quantity = number(4, "quantity")?;
            Ok(Command::Send(replace(old_id, new_id, quantity, price)))
        }
        "status" => Ok(Command::Send(status(identifier(1, "order id")?))),
        "book" => Ok(Command::Send(book_request(app.allocate_id()))),
        "logout" => Ok(Command::Logout),
        "quit" | "exit" => Ok(Command::Quit),
        _ => Err(format!("unknown command: {command}")),
    }
}

fn draw(frame: &mut Frame, app: &App, client: &FixClient) {
    let [header, body, command, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(3),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let mut theme_warnings = Vec::new();
    let theme = Theme::from_config(None, &mut theme_warnings);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" BUNTING ", Style::new().bold().fg(theme.accent)),
            Span::raw("local market · FIX 4.4/TCP · ").dim(),
            Span::styled(&app.status, Style::new().fg(theme.flat)),
        ])),
        header,
    );
    let [book, logs] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);
    render_book(frame, book, client, theme);
    LogPanel::render(frame, logs, &client.logs);
    frame.render_widget(
        Paragraph::new(format!("> {}", app.input))
            .block(Block::default().title(" command ").borders(Borders::ALL)),
        command,
    );
    frame.render_widget(
        Paragraph::new("Enter run · Del clear · F1 help · Esc/Ctrl-C quit"),
        footer,
    );
    if app.show_help {
        render_help(frame);
    }
}

fn render_book(frame: &mut Frame, area: Rect, client: &FixClient, theme: Theme) {
    let depth = client.book.bids.len().max(client.book.asks.len());
    let rows = (0..depth).map(|index| {
        let bid = client.book.bids.get(index).copied();
        let ask = client.book.asks.get(index).copied();
        Row::new(vec![
            bid.map_or_else(String::new, |(_, quantity)| quantity.to_string()),
            bid.map_or_else(String::new, |(price, _)| price.to_string()),
            ask.map_or_else(String::new, |(price, _)| price.to_string()),
            ask.map_or_else(String::new, |(_, quantity)| quantity.to_string()),
        ])
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(
        Row::new(["Bid Qty", "Bid", "Ask", "Ask Qty"]).style(Style::new().bold().fg(theme.accent)),
    )
    .column_spacing(1)
    .block(
        Block::default()
            .title(" BUNT order book ")
            .borders(Borders::ALL),
    );
    frame.render_widget(table, area);
}

fn render_help(frame: &mut Frame) {
    let area = centered(frame.area(), 78, 14);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(
            [
                "buy PRICE QTY        place a limit bid",
                "sell PRICE QTY       place a limit ask",
                "market buy|sell QTY  place a market order",
                "cancel ORDER_ID      cancel an active order",
                "replace OLD NEW PRICE QTY  cancel/replace through FIX G",
                "status ORDER_ID      request FIX order status",
                "book                 refresh FIX market-data snapshot",
                "logout               orderly FIX logout",
                "quit                 exit the terminal",
            ]
            .join("\n"),
        )
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(" Bunting commands · F1 closes ")
                .borders(Borders::ALL)
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        area,
    );
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

// Adapted from longbridge/longbridge-terminal src/tui/widgets/log_panel.rs at
// 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3, Apache-2.0.
// Modified by Bunting: filesystem discovery was replaced with bounded in-memory
// FIX frames, and direction/session colors replace generic tracing levels.
struct LogPanel;

impl LogPanel {
    fn render(frame: &mut Frame, area: Rect, logs: &VecDeque<String>) {
        let block = Block::default()
            .title(" FIX session log ")
            .borders(Borders::ALL)
            .border_style(Style::new().fg(Color::Yellow));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let lines: Vec<Line<'_>> = logs
            .iter()
            .rev()
            .take(usize::from(inner.height))
            .rev()
            .map(|line| {
                let color = if line.starts_with("IN ") {
                    Color::Green
                } else if line.starts_with("OUT") {
                    Color::Cyan
                } else {
                    Color::Gray
                };
                Line::from(Span::styled(line, Style::new().fg(color)))
            })
            .collect();
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl TerminalSession {
    fn new() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self {
            terminal: Terminal::new(CrosstermBackend::new(stdout))?,
        })
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.terminal.backend_mut(), LeaveAlternateScreen);
        let _ = self.terminal.show_cursor();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parser_covers_human_order_actions() {
        let mut app = App::default();
        assert!(matches!(
            parse_command("buy 100 5", &mut app),
            Ok(Command::Send(_))
        ));
        assert!(matches!(
            parse_command("market sell 2", &mut app),
            Ok(Command::Send(_))
        ));
        assert!(matches!(
            parse_command("cancel 1", &mut app),
            Ok(Command::Send(_))
        ));
        assert!(matches!(
            parse_command("replace 1 2 101 3", &mut app),
            Ok(Command::Send(_))
        ));
        assert!(matches!(
            parse_command("status 2", &mut app),
            Ok(Command::Send(_))
        ));
        assert!(matches!(
            parse_command("book", &mut app),
            Ok(Command::Send(_))
        ));
    }
}
