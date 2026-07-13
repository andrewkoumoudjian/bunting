// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::protocol::{FixClient, book_request};
use crate::tui::{keys, nav, popup::PopupKind, render};
use crossterm::{
    event::{self, Event},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use simfix_session::ConnectionState;
use std::{io, time::Duration};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tab {
    #[default]
    Market,
    Orders,
    Fix,
}

#[derive(Debug, Default)]
pub struct App {
    pub tab: Tab,
    pub popup: PopupKind,
    pub input: String,
    pub status: String,
    pub selected_level: usize,
    next_id: u128,
    book_requested: bool,
}

impl App {
    pub fn allocate_id(&mut self) -> u128 {
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.next_id
    }

    pub fn begin_command(&mut self, prefix: &str) {
        prefix.clone_into(&mut self.input);
        self.popup = PopupKind::Command;
    }

    pub fn close_popup(&mut self) {
        self.popup = PopupKind::None;
        self.input.clear();
    }
}

pub async fn run(address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = FixClient::connect(address).await?;
    let mut app = App::default();
    let mut terminal = TerminalSession::new()?;

    loop {
        Box::pin(client.poll()).await?;
        if client.connection_state() == ConnectionState::Established && !app.book_requested {
            let request_id = app.allocate_id();
            client.send(book_request(request_id)).await?;
            app.book_requested = true;
        }
        app.status.clone_from(&client.status);
        terminal
            .terminal
            .draw(|frame| render::draw(frame, &app, &client))?;

        if event::poll(FRAME_INTERVAL)?
            && let Event::Key(key) = event::read()?
        {
            let action = keys::resolve(key, app.popup == PopupKind::Command);
            if nav::handle(action, &mut app, &mut client).await? {
                break;
            }
        }
    }
    Ok(())
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
