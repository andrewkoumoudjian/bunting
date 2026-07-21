// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::tui::{keys, nav, popup::PopupKind, render};
use crate::{
    config::{ConnectionProfile, TerminalConfig, WorkspaceLayout},
    io_task::{IoTask, OutboundCmd, UiEvent},
    protocol::{FixClient, book_request, competition_requests},
};
use crossterm::{
    event::{Event, EventStream},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures_util::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, path::PathBuf, time::Duration};
use tokio::time::{MissedTickBehavior, interval};

const FRAME_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Tab {
    #[default]
    Market,
    Orders,
    Account,
    Simulation,
    Collaboration,
    Administration,
    Session,
}

impl Tab {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Orders => "orders",
            Self::Account => "account",
            Self::Simulation => "simulation",
            Self::Collaboration => "collaboration",
            Self::Administration => "administration",
            Self::Session => "session",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "market" => Some(Self::Market),
            "orders" => Some(Self::Orders),
            "account" => Some(Self::Account),
            "simulation" => Some(Self::Simulation),
            "collaboration" => Some(Self::Collaboration),
            "administration" | "admin" => Some(Self::Administration),
            "session" | "fix" => Some(Self::Session),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

impl OrderSide {
    pub const fn as_fix_name(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum OrderType {
    #[default]
    Limit,
    Market,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TicketField {
    Type,
    #[default]
    Price,
    Quantity,
    Submit,
}

#[derive(Debug, Eq, PartialEq)]
pub struct OrderTicket {
    pub side: OrderSide,
    pub order_type: OrderType,
    pub price: String,
    pub quantity: String,
    pub focused: TicketField,
}

impl OrderTicket {
    pub fn new(side: OrderSide) -> Self {
        Self {
            side,
            order_type: OrderType::Limit,
            price: String::new(),
            quantity: String::new(),
            focused: TicketField::Price,
        }
    }

    pub fn next_field(&mut self) {
        self.focused = match self.focused {
            TicketField::Type => TicketField::Price,
            TicketField::Price if self.order_type == OrderType::Market => TicketField::Quantity,
            TicketField::Price => TicketField::Quantity,
            TicketField::Quantity => TicketField::Submit,
            TicketField::Submit => TicketField::Type,
        };
    }

    pub fn previous_field(&mut self) {
        self.focused = match self.focused {
            TicketField::Type => TicketField::Submit,
            TicketField::Price => TicketField::Type,
            TicketField::Quantity if self.order_type == OrderType::Market => TicketField::Type,
            TicketField::Quantity => TicketField::Price,
            TicketField::Submit => TicketField::Quantity,
        };
    }

    pub fn toggle_type(&mut self) {
        self.order_type = match self.order_type {
            OrderType::Limit => OrderType::Market,
            OrderType::Market => OrderType::Limit,
        };
        if self.order_type == OrderType::Market && self.focused == TicketField::Price {
            self.focused = TicketField::Quantity;
        }
    }

    pub fn values(&self) -> Result<(&'static str, i64, Option<i64>), String> {
        let quantity = positive_number(&self.quantity, "quantity")?;
        let price = match self.order_type {
            OrderType::Limit => Some(positive_number(&self.price, "price")?),
            OrderType::Market => None,
        };
        Ok((self.side.as_fix_name(), quantity, price))
    }
}

fn positive_number(value: &str, name: &str) -> Result<i64, String> {
    let value = value
        .parse::<i64>()
        .map_err(|_| format!("enter a valid {name}"))?;
    if value <= 0 {
        return Err(format!("{name} must be greater than zero"));
    }
    Ok(value)
}

#[derive(Debug)]
pub struct App {
    pub tab: Tab,
    pub popup: PopupKind,
    pub input: String,
    pub status: String,
    pub selected_level: usize,
    pub order_quantity: i64,
    pub order_ticket: Option<OrderTicket>,
    pub active_workspace: String,
    pub config: TerminalConfig,
    pub config_path: PathBuf,
    next_id: u128,
}

impl Default for App {
    fn default() -> Self {
        Self::new(
            TerminalConfig::default(),
            PathBuf::from("bunting-terminal.json"),
        )
    }
}

impl App {
    pub fn new(config: TerminalConfig, config_path: PathBuf) -> Self {
        let workspace = config
            .workspaces
            .get("default")
            .cloned()
            .unwrap_or_default();
        Self {
            tab: Tab::from_name(&workspace.initial_view).unwrap_or_default(),
            popup: PopupKind::default(),
            input: String::new(),
            status: String::new(),
            selected_level: 0,
            order_quantity: 1,
            order_ticket: None,
            active_workspace: "default".to_owned(),
            config,
            config_path,
            next_id: 0,
        }
    }

    pub fn allocate_id(&mut self) -> u128 {
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.next_id
    }

    pub fn begin_command(&mut self, prefix: &str) {
        prefix.clone_into(&mut self.input);
        self.popup = PopupKind::Command;
    }

    pub fn begin_order(&mut self, side: OrderSide) {
        self.order_ticket = Some(OrderTicket::new(side));
        self.popup = PopupKind::OrderTicket;
    }

    pub fn close_popup(&mut self) {
        self.popup = PopupKind::None;
        self.input.clear();
        self.order_ticket = None;
    }

    pub fn save_workspace(&mut self, name: &str) -> Result<(), String> {
        validate_workspace_name(name)?;
        let current = self
            .config
            .workspaces
            .get(&self.active_workspace)
            .cloned()
            .unwrap_or_default();
        self.config.workspaces.insert(
            name.to_owned(),
            WorkspaceLayout {
                initial_view: self.tab.name().to_owned(),
                ..current
            },
        );
        name.clone_into(&mut self.active_workspace);
        self.config
            .save(&self.config_path)
            .map_err(|error| format!("workspace save failed: {error}"))
    }

    pub fn load_workspace(&mut self, name: &str) -> Result<(), String> {
        validate_workspace_name(name)?;
        let workspace = self
            .config
            .workspaces
            .get(name)
            .ok_or_else(|| format!("unknown workspace: {name}"))?;
        self.tab = Tab::from_name(&workspace.initial_view)
            .ok_or_else(|| format!("workspace {name} has an unknown initial_view"))?;
        name.clone_into(&mut self.active_workspace);
        Ok(())
    }

    pub fn remove_workspace(&mut self, name: &str) -> Result<(), String> {
        if name == "default" {
            return Err("the default workspace cannot be removed".to_owned());
        }
        if self.config.workspaces.remove(name).is_none() {
            return Err(format!("unknown workspace: {name}"));
        }
        if self.active_workspace == name {
            "default".clone_into(&mut self.active_workspace);
        }
        self.config
            .save(&self.config_path)
            .map_err(|error| format!("workspace removal failed: {error}"))
    }
}

fn validate_workspace_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 32
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("workspace names use 1-32 letters, numbers, '-' or '_'".to_owned());
    }
    Ok(())
}

pub async fn run(
    profile_name: String,
    profile: ConnectionProfile,
    credential_override: Option<String>,
    config: TerminalConfig,
    config_path: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    let connection = FixClient::new(profile_name, profile, credential_override)?;
    let mut client = connection.view_clone()?;
    let mut io_task = IoTask::spawn(connection);
    let mut app = App::new(config, config_path);
    let mut terminal = TerminalSession::new()?;
    let mut input = EventStream::new();
    let mut ticker = interval(FRAME_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut dirty = true;

    loop {
        tokio::select! {
            event = input.next() => match event {
                Some(Ok(Event::Key(key))) => {
                    let action = keys::resolve(
                        key,
                        matches!(app.popup, PopupKind::Command | PopupKind::OrderTicket),
                    );
                    if nav::handle(action, &mut app, &mut client, &io_task.outbound) {
                        break;
                    }
                    dirty = true;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(Box::new(error)),
                None => break,
            },
            event = io_task.events.recv() => {
                let Some(UiEvent::Snapshot {
                    client: snapshot,
                    mut recovery_request,
                    mut competition_request,
                }) = event else {
                    break;
                };
                client = *snapshot;
                while let Ok(UiEvent::Snapshot {
                    client: snapshot,
                    recovery_request: next_recovery,
                    competition_request: next_competition,
                }) = io_task.events.try_recv() {
                    client = *snapshot;
                    recovery_request |= next_recovery;
                    competition_request |= next_competition;
                }
                if recovery_request {
                    let request_id = app.allocate_id();
                    enqueue(&mut app, &io_task.outbound, OutboundCmd::Send(book_request(request_id)));
                }
                if competition_request {
                    let request_id = app.allocate_id();
                    for request in competition_requests(request_id) {
                        enqueue(&mut app, &io_task.outbound, OutboundCmd::Send(request));
                    }
                }
                dirty = true;
            }
            _ = ticker.tick() => {
                if dirty {
                    terminal
                        .terminal
                        .draw(|frame| render::draw(frame, &app, &client))?;
                    dirty = false;
                }
            }
        }
    }
    io_task.shutdown().await;
    Ok(())
}

fn enqueue(app: &mut App, outbound: &tokio::sync::mpsc::Sender<OutboundCmd>, command: OutboundCmd) {
    match outbound.try_send(command) {
        Ok(()) => {}
        Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
            "FIX outbound queue is full; command was not sent".clone_into(&mut app.status);
        }
        Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
            "FIX I/O task is unavailable; command was not sent".clone_into(&mut app.status);
        }
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
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn limit_ticket_requires_positive_price_and_quantity() {
        let mut ticket = OrderTicket::new(OrderSide::Buy);
        ticket.price = "101".to_owned();
        ticket.quantity = "5".to_owned();
        assert_eq!(ticket.values(), Ok(("buy", 5, Some(101))));

        ticket.quantity = "0".to_owned();
        assert_eq!(
            ticket.values(),
            Err("quantity must be greater than zero".to_owned())
        );
    }

    fn app() -> App {
        App::new(
            TerminalConfig::default(),
            PathBuf::from("unused-test-config.json"),
        )
    }

    #[test]
    fn workspace_reducer_saves_loads_and_removes_layouts() {
        let mut app = app();
        app.tab = Tab::Account;
        app.config_path =
            std::env::temp_dir().join(format!("bunting-workspace-{}.json", std::process::id()));
        app.save_workspace("risk-desk").unwrap();
        app.tab = Tab::Market;
        app.load_workspace("risk-desk").unwrap();
        assert_eq!(app.tab, Tab::Account);
        app.remove_workspace("risk-desk").unwrap();
        let _ = std::fs::remove_file(&app.config_path);
    }

    #[test]
    fn market_ticket_skips_price() {
        let mut ticket = OrderTicket::new(OrderSide::Sell);
        ticket.toggle_type();
        ticket.quantity = "3".to_owned();
        assert_eq!(ticket.values(), Ok(("sell", 3, None)));
        assert_eq!(ticket.focused, TicketField::Quantity);
    }
}
