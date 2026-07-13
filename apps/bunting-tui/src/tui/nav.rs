// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::protocol::{FixClient, book_request, cancel, new_order, replace, status};
use crate::tui::{
    app::{App, OrderSide, OrderType, Tab, TicketField},
    keys::Action,
    popup::PopupKind,
};
use std::io;

const MAX_COMMAND_BYTES: usize = 256;
const MAX_ORDER_NUMBER_BYTES: usize = 20;

#[expect(
    clippy::too_many_lines,
    reason = "the navigation reducer keeps every key action in one exhaustive match"
)]
pub async fn handle(action: Action, app: &mut App, client: &mut FixClient) -> io::Result<bool> {
    match action {
        Action::Quit => return Ok(true),
        Action::Escape => app.close_popup(),
        Action::ToggleHelp => {
            app.popup = if app.popup == PopupKind::Help {
                PopupKind::None
            } else {
                PopupKind::Help
            };
        }
        Action::ToggleLog => {
            app.popup = if app.popup == PopupKind::FixLog {
                PopupKind::None
            } else {
                PopupKind::FixLog
            };
        }
        Action::TabMarket => {
            app.close_popup();
            app.tab = Tab::Market;
        }
        Action::TabOrders => {
            app.close_popup();
            app.tab = Tab::Orders;
        }
        Action::TabFix => {
            app.close_popup();
            app.tab = Tab::Fix;
        }
        Action::BeginCommand => app.begin_command(""),
        Action::BeginBuy => app.begin_order(OrderSide::Buy),
        Action::BeginSell => app.begin_order(OrderSide::Sell),
        Action::BeginCancel => app.begin_command("cancel "),
        Action::BeginReplace => app.begin_command("replace "),
        Action::Refresh => {
            let request_id = app.allocate_id();
            Box::pin(client.send(book_request(request_id))).await?;
        }
        Action::SelectPrevious => {
            app.selected_level = app.selected_level.saturating_sub(1);
        }
        Action::SelectNext => {
            let last = client
                .book
                .bids
                .len()
                .max(client.book.asks.len())
                .saturating_sub(1);
            app.selected_level = app.selected_level.saturating_add(1).min(last);
        }
        Action::NextField if app.popup == PopupKind::OrderTicket => {
            if let Some(ticket) = &mut app.order_ticket {
                ticket.next_field();
            }
        }
        Action::PreviousField if app.popup == PopupKind::OrderTicket => {
            if let Some(ticket) = &mut app.order_ticket {
                ticket.previous_field();
            }
        }
        Action::Left | Action::Right if app.popup == PopupKind::OrderTicket => {
            if let Some(ticket) = &mut app.order_ticket
                && ticket.focused == TicketField::Type
            {
                ticket.toggle_type();
            }
        }
        Action::Submit if app.popup == PopupKind::OrderTicket => {
            let should_submit = app
                .order_ticket
                .as_ref()
                .is_some_and(|ticket| ticket.focused == TicketField::Submit);
            if should_submit {
                let Some(ticket) = app.order_ticket.take() else {
                    return Ok(false);
                };
                match ticket.values() {
                    Ok((side, quantity, price)) => {
                        let id = app.allocate_id();
                        app.popup = PopupKind::None;
                        Box::pin(client.send(new_order(id, side, quantity, price))).await?;
                    }
                    Err(error) => {
                        app.status = error;
                        app.order_ticket = Some(ticket);
                    }
                }
            } else if let Some(ticket) = &mut app.order_ticket {
                ticket.next_field();
            }
        }
        Action::Submit if app.popup == PopupKind::Command => {
            let input = std::mem::take(&mut app.input);
            app.popup = PopupKind::None;
            match parse_command(&input, app) {
                Ok(Command::Send(message)) => Box::pin(client.send(message)).await?,
                Ok(Command::Logout) => Box::pin(client.logout()).await?,
                Ok(Command::Quit) => return Ok(true),
                Ok(Command::None) => {}
                Err(error) => app.status = error,
            }
        }
        Action::Backspace if app.popup == PopupKind::Command => {
            app.input.pop();
        }
        Action::Backspace if app.popup == PopupKind::OrderTicket => {
            if let Some(ticket) = &mut app.order_ticket {
                match ticket.focused {
                    TicketField::Price => {
                        ticket.price.pop();
                    }
                    TicketField::Quantity => {
                        ticket.quantity.pop();
                    }
                    TicketField::Type | TicketField::Submit => {}
                }
            }
        }
        Action::Character(character)
            if app.popup == PopupKind::Command && app.input.len() < MAX_COMMAND_BYTES =>
        {
            app.input.push(character);
        }
        Action::Character(character) if app.popup == PopupKind::OrderTicket => {
            if let Some(ticket) = &mut app.order_ticket {
                match (ticket.focused, character) {
                    (TicketField::Price, digit)
                        if digit.is_ascii_digit()
                            && ticket.price.len() < MAX_ORDER_NUMBER_BYTES =>
                    {
                        ticket.price.push(digit);
                    }
                    (TicketField::Quantity, digit)
                        if digit.is_ascii_digit()
                            && ticket.quantity.len() < MAX_ORDER_NUMBER_BYTES =>
                    {
                        ticket.quantity.push(digit);
                    }
                    (TicketField::Type, 'l' | 'L') => ticket.order_type = OrderType::Limit,
                    (TicketField::Type, 'm' | 'M') => ticket.order_type = OrderType::Market,
                    _ => {}
                }
            }
        }
        Action::None
        | Action::Submit
        | Action::Backspace
        | Action::Character(_)
        | Action::NextField
        | Action::PreviousField
        | Action::Left
        | Action::Right => {}
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
            let order_id = identifier(1, "order id")?;
            let request_id = app.allocate_id();
            Ok(Command::Send(cancel(order_id, request_id)))
        }
        "replace" => {
            let old_id = identifier(1, "old order id")?;
            let new_id = identifier(2, "new order id")?;
            let price = number(3, "price")?;
            let quantity = number(4, "quantity")?;
            Ok(Command::Send(replace(old_id, new_id, quantity, price)))
        }
        "status" => Ok(Command::Send(status(identifier(1, "order id")?))),
        "book" | "refresh" => Ok(Command::Send(book_request(app.allocate_id()))),
        "logout" => Ok(Command::Logout),
        "quit" | "exit" => Ok(Command::Quit),
        _ => Err(format!("unknown command: {command}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_parser_covers_human_fix_actions() {
        let mut app = App::default();
        for command in [
            "buy 100 5",
            "sell 101 5",
            "market sell 2",
            "cancel 1",
            "replace 1 2 101 3",
            "status 2",
            "book",
        ] {
            assert!(matches!(
                parse_command(command, &mut app),
                Ok(Command::Send(_))
            ));
        }
    }
}
