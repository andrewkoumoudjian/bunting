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
    next_id: u128,
    book_requested: bool,
}

impl Default for App {
    fn default() -> Self {
        Self {
            tab: Tab::default(),
            popup: PopupKind::default(),
            input: String::new(),
            status: String::new(),
            selected_level: 0,
            order_quantity: 1,
            order_ticket: None,
            next_id: 0,
            book_requested: false,
        }
    }
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

    pub fn begin_order(&mut self, side: OrderSide) {
        self.order_ticket = Some(OrderTicket::new(side));
        self.popup = PopupKind::OrderTicket;
    }

    pub fn close_popup(&mut self) {
        self.popup = PopupKind::None;
        self.input.clear();
        self.order_ticket = None;
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
            let action = keys::resolve(
                key,
                matches!(app.popup, PopupKind::Command | PopupKind::OrderTicket),
            );
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

#[cfg(test)]
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

    #[test]
    fn market_ticket_skips_price() {
        let mut ticket = OrderTicket::new(OrderSide::Sell);
        ticket.toggle_type();
        ticket.quantity = "3".to_owned();
        assert_eq!(ticket.values(), Ok(("sell", 3, None)));
        assert_eq!(ticket.focused, TicketField::Quantity);
    }
}
