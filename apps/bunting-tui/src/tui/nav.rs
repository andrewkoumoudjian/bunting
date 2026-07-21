// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::tui::{
    app::{App, OrderSide, OrderType, Tab, TicketField},
    keys::Action,
    popup::PopupKind,
};
use crate::{
    io_task::OutboundCmd,
    protocol::{FixClient, book_request, cancel, competition_action, new_order},
};
use bunting_market_events::NewsAudience;
use bunting_market_types::{CurrencyId, MoneyMinor, NewsId, ParticipantId};
use simfix_mapping::{ApplyFinePayload, PublishNewsPayload, RunAdvancePayload, RunReasonPayload};
use tokio::sync::mpsc;

const MAX_COMMAND_BYTES: usize = 256;
const MAX_ORDER_NUMBER_BYTES: usize = 20;

#[expect(
    clippy::too_many_lines,
    reason = "the navigation reducer keeps every key action in one exhaustive match"
)]
pub fn handle(
    action: Action,
    app: &mut App,
    client: &mut FixClient,
    outbound: &mpsc::Sender<OutboundCmd>,
) -> bool {
    match action {
        Action::Quit => return true,
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
        Action::TabAccount => {
            app.close_popup();
            app.tab = Tab::Account;
        }
        Action::TabSimulation => {
            app.close_popup();
            app.tab = Tab::Simulation;
        }
        Action::TabCollaboration => {
            app.close_popup();
            app.tab = Tab::Collaboration;
        }
        Action::TabAdministration => {
            app.close_popup();
            app.tab = Tab::Administration;
        }
        Action::TabSession => {
            app.close_popup();
            app.tab = Tab::Session;
        }
        Action::BeginCommand => app.begin_command(""),
        Action::BeginQuantity => app.begin_command("qty "),
        Action::BeginBuy if app.popup == PopupKind::None && app.tab == Tab::Market => {
            submit_selected_level(app, client, outbound, OrderSide::Buy);
        }
        Action::BeginSell if app.popup == PopupKind::None && app.tab == Tab::Market => {
            submit_selected_level(app, client, outbound, OrderSide::Sell);
        }
        Action::BeginBuy => app.begin_order(OrderSide::Buy),
        Action::BeginSell => app.begin_order(OrderSide::Sell),
        Action::BeginCancel => app.begin_command("cancel "),
        Action::Refresh => {
            if client.connection_state() == simfix_session::ConnectionState::Disconnected {
                enqueue(app, outbound, OutboundCmd::Reconnect);
            } else {
                let request_id = app.allocate_id();
                enqueue(app, outbound, OutboundCmd::Send(book_request(request_id)));
            }
        }
        Action::SelectPrevious => {
            app.selected_level = app.selected_level.saturating_sub(1);
        }
        Action::SelectNext => {
            let last = client
                .book
                .bids
                .len()
                .saturating_add(client.book.asks.len())
                .saturating_sub(1);
            app.selected_level = app.selected_level.saturating_add(1).min(last);
        }
        Action::Submit if app.popup == PopupKind::None && app.tab == Tab::Market => {
            if let Some((book_side, _, _)) = selected_level(app, client) {
                let side = if book_side == "ASK" {
                    OrderSide::Buy
                } else {
                    OrderSide::Sell
                };
                submit_selected_level(app, client, outbound, side);
            }
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
                    return false;
                };
                match ticket.values() {
                    Ok((side, quantity, price)) => {
                        let id = app.allocate_id();
                        app.popup = PopupKind::None;
                        enqueue(
                            app,
                            outbound,
                            OutboundCmd::Send(new_order(id, side, quantity, price)),
                        );
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
                Ok(Command::Send(message)) => {
                    enqueue(app, outbound, OutboundCmd::Send(message));
                }
                Ok(Command::SetQuantity(quantity)) => {
                    app.order_quantity = quantity;
                    client.status = format!("order quantity set to {quantity}");
                }
                Ok(Command::Logout) => {
                    enqueue(app, outbound, OutboundCmd::Logout);
                }
                Ok(Command::Reconnect) => {
                    enqueue(app, outbound, OutboundCmd::Reconnect);
                }
                Ok(Command::ResetSession) => {
                    enqueue(app, outbound, OutboundCmd::ResetSession);
                }
                Ok(Command::SaveWorkspace(name)) => match app.save_workspace(&name) {
                    Ok(()) => app.status = format!("workspace {name} saved"),
                    Err(error) => app.status = error,
                },
                Ok(Command::LoadWorkspace(name)) => match app.load_workspace(&name) {
                    Ok(()) => app.status = format!("workspace {name} loaded"),
                    Err(error) => app.status = error,
                },
                Ok(Command::RemoveWorkspace(name)) => match app.remove_workspace(&name) {
                    Ok(()) => app.status = format!("workspace {name} removed"),
                    Err(error) => app.status = error,
                },
                Ok(Command::Quit) => return true,
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
    false
}

fn selected_level<'a>(app: &App, client: &'a FixClient) -> Option<(&'a str, i64, i64)> {
    client
        .book
        .asks
        .iter()
        .rev()
        .map(|(price, quantity)| ("ASK", *price, *quantity))
        .chain(
            client
                .book
                .bids
                .iter()
                .map(|(price, quantity)| ("BID", *price, *quantity)),
        )
        .nth(app.selected_level)
}

fn submit_selected_level(
    app: &mut App,
    client: &mut FixClient,
    outbound: &mpsc::Sender<OutboundCmd>,
    side: OrderSide,
) {
    let Some((_, price, _)) = selected_level(app, client) else {
        "select a live order-book level first".clone_into(&mut app.status);
        return;
    };
    let id = app.allocate_id();
    enqueue(
        app,
        outbound,
        OutboundCmd::Send(new_order(
            id,
            side.as_fix_name(),
            app.order_quantity,
            Some(price),
        )),
    );
}

fn enqueue(app: &mut App, outbound: &mpsc::Sender<OutboundCmd>, command: OutboundCmd) {
    match outbound.try_send(command) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            "FIX outbound queue is full; command was not sent".clone_into(&mut app.status);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {
            "FIX I/O task is unavailable; command was not sent".clone_into(&mut app.status);
        }
    }
}

enum Command {
    Send(simfix_wire::FixMessage),
    SetQuantity(i64),
    Logout,
    Reconnect,
    ResetSession,
    SaveWorkspace(String),
    LoadWorkspace(String),
    RemoveWorkspace(String),
    Quit,
    None,
}

#[expect(
    clippy::too_many_lines,
    reason = "the command palette parser keeps all documented operator and participant workflows together"
)]
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
        "book" | "refresh" => Ok(Command::Send(book_request(app.allocate_id()))),
        "tender" => {
            let action = parts.get(1).copied().ok_or("missing tender action")?;
            if !matches!(action, "accept" | "decline") {
                return Err("usage: tender accept|decline ID".to_owned());
            }
            let tender_id = parts
                .get(2)
                .ok_or("missing tender id")?
                .parse::<u64>()
                .map_err(|_| "invalid tender id")?;
            Ok(Command::Send(competition_action(
                "U6",
                action,
                Some(tender_id),
                None,
            )))
        }
        "run" => {
            let action = parts.get(1).copied().ok_or("missing run action")?;
            let payload = match action {
                "start" | "pause" | "resume" | "score" => None,
                "advance" => Some(
                    serde_json::to_string(&RunAdvancePayload {
                        steps: parts
                            .get(2)
                            .ok_or("missing steps")?
                            .parse::<u32>()
                            .map_err(|_| "invalid steps")?,
                    })
                    .map_err(|_| "cannot encode advance payload")?,
                ),
                "terminate" => Some(
                    serde_json::to_string(&RunReasonPayload {
                        reason: parts.get(2..).unwrap_or_default().join(" "),
                    })
                    .map_err(|_| "cannot encode terminate payload")?,
                ),
                _ => {
                    return Err(
                        "usage: run start|pause|resume|advance N|terminate REASON".to_owned()
                    );
                }
            };
            Ok(Command::Send(competition_action(
                "UA", action, None, payload,
            )))
        }
        "news" => {
            let news_id = parts
                .get(1)
                .ok_or("missing news id")?
                .parse::<u64>()
                .map_err(|_| "invalid news id")?;
            let audience = parse_news_audience(parts.get(2).copied().ok_or("missing audience")?)?;
            let headline = parts.get(3..).unwrap_or_default().join(" ");
            if headline.is_empty() {
                return Err("missing headline".to_owned());
            }
            let payload = serde_json::to_string(&PublishNewsPayload {
                news_id: NewsId::new(news_id.into()),
                audience,
                body: headline.clone(),
                headline,
            })
            .map_err(|_| "cannot encode news payload")?;
            Ok(Command::Send(competition_action(
                "UA",
                "publish_news",
                None,
                Some(payload),
            )))
        }
        "score" => Ok(Command::Send(competition_action("UA", "score", None, None))),
        "fine" => {
            let participant_id = identifier(1, "participant id")?;
            let currency_id = identifier(2, "currency id")?;
            let amount = number(3, "amount")?;
            let reason = parts.get(4..).unwrap_or_default().join(" ");
            if reason.is_empty() {
                return Err("missing fine reason".to_owned());
            }
            let payload = serde_json::to_string(&ApplyFinePayload {
                participant_id: ParticipantId::new(participant_id),
                currency_id: CurrencyId::new(currency_id),
                amount: MoneyMinor::new(i128::from(amount)),
                reason,
            })
            .map_err(|_| "cannot encode fine payload")?;
            Ok(Command::Send(competition_action(
                "UB",
                "fine",
                None,
                Some(payload),
            )))
        }
        "qty" | "quantity" => {
            let quantity = number(1, "quantity")?;
            if quantity <= 0 {
                return Err("quantity must be greater than zero".to_owned());
            }
            Ok(Command::SetQuantity(quantity))
        }
        "logout" => Ok(Command::Logout),
        "connect" | "reconnect" => Ok(Command::Reconnect),
        "session" if parts.get(1) == Some(&"reset") => Ok(Command::ResetSession),
        "workspace" => match (parts.get(1), parts.get(2)) {
            (Some(&"save"), Some(name)) => Ok(Command::SaveWorkspace((*name).to_owned())),
            (Some(&"load"), Some(name)) => Ok(Command::LoadWorkspace((*name).to_owned())),
            (Some(&"remove"), Some(name)) => Ok(Command::RemoveWorkspace((*name).to_owned())),
            _ => Err("usage: workspace save|load|remove NAME".to_owned()),
        },
        "quit" | "exit" => Ok(Command::Quit),
        _ => Err(format!("unknown command: {command}")),
    }
}

fn parse_news_audience(value: &str) -> Result<NewsAudience, String> {
    if value.eq_ignore_ascii_case("public") {
        return Ok(NewsAudience::Public);
    }
    let (kind, id) = value
        .split_once(':')
        .ok_or("audience must be public, participant:ID, team:ID, or role:NAME")?;
    match kind.to_ascii_lowercase().as_str() {
        "participant" => id
            .parse::<u128>()
            .map(ParticipantId::new)
            .map(NewsAudience::Participant)
            .map_err(|_| "invalid participant audience id".to_owned()),
        "team" => id
            .parse::<u128>()
            .map(NewsAudience::Team)
            .map_err(|_| "invalid team audience id".to_owned()),
        "role" if !id.is_empty() => Ok(NewsAudience::Role(id.to_owned())),
        _ => Err("audience must be public, participant:ID, team:ID, or role:NAME".to_owned()),
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
            "book",
            "tender accept 7",
            "run pause",
            "run advance 10",
            "news 3 public Market opens",
            "score",
            "fine 1 840 25 conduct",
        ] {
            assert!(matches!(
                parse_command(command, &mut app),
                Ok(Command::Send(_))
            ));
        }
        assert!(matches!(
            parse_command("qty 25", &mut app),
            Ok(Command::SetQuantity(25))
        ));
        assert!(parse_command("replace 1 2 101 3", &mut app).is_err());
        assert!(parse_command("status 2", &mut app).is_err());
    }

    #[test]
    fn operator_commands_encode_the_shared_server_payload_types()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut app = App::default();
        let mut payload = |command: &str| {
            let parsed = parse_command(command, &mut app).map_err(std::io::Error::other)?;
            let Command::Send(message) = parsed else {
                return Err(std::io::Error::other("FIX message expected"));
            };
            message
                .value(10020)
                .map(ToOwned::to_owned)
                .ok_or_else(|| std::io::Error::other("operator payload expected"))
        };

        assert_eq!(
            serde_json::from_str::<RunAdvancePayload>(&payload("run advance 10")?)?,
            RunAdvancePayload { steps: 10 }
        );
        assert_eq!(
            serde_json::from_str::<PublishNewsPayload>(&payload("news 3 public Market opens")?)?,
            PublishNewsPayload {
                news_id: NewsId::new(3),
                audience: NewsAudience::Public,
                headline: "Market opens".to_owned(),
                body: "Market opens".to_owned(),
            }
        );
        assert_eq!(
            serde_json::from_str::<ApplyFinePayload>(&payload("fine 1 840 25 conduct")?)?,
            ApplyFinePayload {
                participant_id: ParticipantId::new(1),
                currency_id: CurrencyId::new(840),
                amount: MoneyMinor::new(25),
                reason: "conduct".to_owned(),
            }
        );
        Ok(())
    }
}
