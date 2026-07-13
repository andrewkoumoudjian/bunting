// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::protocol::{FixClient, book_request, cancel, new_order, replace, status};
use crate::tui::{
    app::{App, Tab},
    keys::Action,
    popup::PopupKind,
};
use std::io;

const MAX_COMMAND_BYTES: usize = 256;

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
        Action::BeginBuy => app.begin_command("buy "),
        Action::BeginSell => app.begin_command("sell "),
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
        Action::Character(character)
            if app.popup == PopupKind::Command && app.input.len() < MAX_COMMAND_BYTES =>
        {
            app.input.push(character);
        }
        Action::None | Action::Submit | Action::Backspace | Action::Character(_) => {}
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
