// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::tui::{
    app::{App, OrderSide, OrderType, TicketField},
    popup::PopupKind,
    ui::{rect, styles},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

pub fn render(frame: &mut Frame, area: Rect, app: &App) {
    match app.popup {
        PopupKind::Help => super::help::render(frame, area),
        PopupKind::Command => render_command(frame, area, app),
        PopupKind::OrderTicket => render_order_ticket(frame, area, app),
        PopupKind::None | PopupKind::FixLog => {}
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the order ticket is one compact modal with five directly related regions"
)]
fn render_order_ticket(frame: &mut Frame, area: Rect, app: &App) {
    let Some(ticket) = &app.order_ticket else {
        return;
    };
    let popup = rect::centered(64, 17, area);
    let side_style = match ticket.side {
        OrderSide::Buy => styles::bid(),
        OrderSide::Sell => styles::ask(),
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Block::new()
            .title(Span::styled(
                format!(
                    " {} ORDER TICKET ",
                    ticket.side.as_fix_name().to_uppercase()
                ),
                side_style,
            ))
            .title_bottom(Line::from(" Tab move · ←→ change · Esc close ").centered())
            .borders(Borders::ALL)
            .border_style(side_style)
            .style(Style::new().bg(Color::Black)),
        popup,
    );
    let inner = popup.inner(Margin::new(2, 1));
    let [side_area, type_area, values_area, preview_area, submit_area] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(3),
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Length(2),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("SIDE  ", styles::label()),
            Span::styled(ticket.side.as_fix_name().to_uppercase(), side_style),
            Span::raw("      INSTRUMENT  "),
            Span::styled("BUNT", styles::accent()),
        ])),
        side_area,
    );
    let type_text = match ticket.order_type {
        OrderType::Limit => " LIMIT   market ",
        OrderType::Market => " limit   MARKET ",
    };
    frame.render_widget(
        Paragraph::new(type_text).block(field_block(
            " ORDER TYPE · L/M ",
            ticket.focused == TicketField::Type,
        )),
        type_area,
    );
    let [price_area, quantity_area] =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .areas(values_area);
    let price_text = if ticket.order_type == OrderType::Market {
        "market".to_owned()
    } else {
        ticket.price.clone()
    };
    frame.render_widget(
        Paragraph::new(price_text).block(field_block(
            " PRICE (ticks) ",
            ticket.focused == TicketField::Price,
        )),
        price_area,
    );
    frame.render_widget(
        Paragraph::new(ticket.quantity.as_str()).block(field_block(
            " QUANTITY (lots) ",
            ticket.focused == TicketField::Quantity,
        )),
        quantity_area,
    );
    let price = if ticket.order_type == OrderType::Market {
        "MARKET".to_owned()
    } else if ticket.price.is_empty() {
        "--".to_owned()
    } else {
        ticket.price.clone()
    };
    frame.render_widget(
        Paragraph::new(format!(
            "{} {} BUNT @ {}",
            ticket.side.as_fix_name().to_uppercase(),
            if ticket.quantity.is_empty() {
                "--"
            } else {
                &ticket.quantity
            },
            price
        ))
        .style(side_style),
        preview_area,
    );
    frame.render_widget(
        Paragraph::new("                 [ ENTER SUBMIT ORDER ]").style(
            if ticket.focused == TicketField::Submit {
                styles::selected()
            } else {
                styles::label()
            },
        ),
        submit_area,
    );

    let cursor = match ticket.focused {
        TicketField::Price if ticket.order_type == OrderType::Limit => Some((
            price_area.x + 1 + u16::try_from(ticket.price.len()).unwrap_or(u16::MAX),
            price_area.y + 1,
        )),
        TicketField::Quantity => Some((
            quantity_area.x + 1 + u16::try_from(ticket.quantity.len()).unwrap_or(u16::MAX),
            quantity_area.y + 1,
        )),
        TicketField::Type | TicketField::Price | TicketField::Submit => None,
    };
    if let Some((x, y)) = cursor {
        frame.set_cursor_position((x.min(popup.right().saturating_sub(2)), y));
    }
}

fn field_block(title: &'static str, focused: bool) -> Block<'static> {
    Block::new()
        .title(title)
        .borders(Borders::ALL)
        .border_style(if focused {
            styles::active_border()
        } else {
            styles::border()
        })
}

fn render_command(frame: &mut Frame, area: Rect, app: &App) {
    let width = 76_u16.min(area.width.saturating_sub(4));
    let popup = Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(3) / 2,
        width,
        3,
    );
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(app.input.as_str()).block(
            Block::new()
                .title(" Bunting command · Enter submit · Esc close ")
                .borders(Borders::ALL)
                .border_style(styles::active_border())
                .style(Style::new().bg(Color::Black)),
        ),
        popup,
    );
    let cursor_offset = u16::try_from(app.input.chars().count()).unwrap_or(u16::MAX);
    frame.set_cursor_position((
        popup.x + cursor_offset.min(popup.width.saturating_sub(2)) + 1,
        popup.y + 1,
    ));
}
