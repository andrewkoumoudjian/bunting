fn render_account(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (holdings, cash, fallback) = {
        let terminal = terminal.read(cx);
        if let Some(account) = terminal
            .client
            .as_ref()
            .and_then(|client| client.authoritative_account.as_ref())
        {
            let holdings = account
                .holdings
                .iter()
                .take(64)
                .map(|holding| {
                    (
                        holding.instrument_id.to_string(),
                        holding.position.to_string(),
                        holding.reserved.to_string(),
                        holding.realized_pnl.to_string(),
                        holding.unrealized_pnl.to_string(),
                    )
                })
                .collect::<Vec<_>>();
            let cash = account
                .cash
                .iter()
                .take(8)
                .map(|balance| {
                    format!(
                        "CCY {}  SETTLED {}  RESERVED {}  MARGIN {}",
                        balance.currency_id, balance.settled, balance.reserved, balance.margin
                    )
                })
                .collect::<Vec<_>>();
            (holdings, cash, None)
        } else if let Some(client) = &terminal.client {
            let marked = client
                .book
                .bids
                .first()
                .map_or(client.portfolio.cash, |level| {
                    client.portfolio.marked_value(level.0)
                });
            (
                Vec::new(),
                Vec::new(),
                Some(format!(
                    "Local projection • position {} • cash {} • marked {}",
                    client.portfolio.position, client.portfolio.cash, marked
                )),
            )
        } else {
            (Vec::new(), Vec::new(), None)
        }
    };

    let mut body = TableBody::new();
    for (instrument, position, reserved, realized, unrealized) in holdings {
        body = body.child(
            TableRow::new()
                .child(TableCell::new().child(instrument))
                .child(TableCell::new().text_right().child(position))
                .child(TableCell::new().text_right().child(reserved))
                .child(TableCell::new().text_right().child(realized))
                .child(TableCell::new().text_right().child(unrealized)),
        );
    }

    let mut cash_rows = v_flex().gap_1();
    for balance in cash {
        cash_rows = cash_rows.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(balance),
        );
    }

    v_flex()
        .size_full()
        .min_h_0()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(
                    Table::new()
                        .border_0()
                        .rounded_none()
                        .child(
                            TableHeader::new().child(
                                TableRow::new()
                                    .child(TableHead::new().child("INSTR"))
                                    .child(TableHead::new().text_right().child("POSITION"))
                                    .child(TableHead::new().text_right().child("RESERVED"))
                                    .child(TableHead::new().text_right().child("REALIZED"))
                                    .child(TableHead::new().text_right().child("UNREALIZED")),
                            ),
                        )
                        .child(body),
                ),
        )
        .when_some(fallback, |element, fallback| {
            element.child(
                div()
                    .p_3()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(fallback),
            )
        })
        .child(
            div()
                .flex_none()
                .p_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(cash_rows),
        )
        .into_any_element()
}

fn render_news(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let news = {
        let terminal = terminal.read(cx);
        terminal.client.as_ref().map_or_else(Vec::new, |client| {
            client
                .news
                .iter()
                .rev()
                .take(64)
                .map(|item| (item.headline.clone(), item.body.clone()))
                .collect::<Vec<_>>()
        })
    };

    let mut list = v_flex().size_full();
    for (headline, body) in news {
        list = list.child(
            v_flex()
                .gap_1()
                .p_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::BOLD)
                        .child(headline),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(body),
                ),
        );
    }

    div()
        .size_full()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(list)
        .into_any_element()
}

fn render_tenders(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let tenders = {
        let terminal = terminal.read(cx);
        terminal.client.as_ref().map_or_else(Vec::new, |client| {
            client
                .tenders
                .iter()
                .take(32)
                .map(|tender| {
                    (
                        tender.tender_id.get(),
                        format!("{:?}", tender.side),
                        tender.quantity.to_string(),
                        tender.price.to_string(),
                        tender.status.clone(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };

    let mut list = v_flex().gap_2().p_2();
    for (tender_id, side, quantity, price, status) in tenders {
        let accept_terminal = terminal.clone();
        let decline_terminal = terminal.clone();
        list = list.child(
            h_flex()
                .gap_3()
                .p_2()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().secondary)
                .child(metric("ID", tender_id.to_string(), cx.theme().foreground, cx))
                .child(metric("SIDE", side, cx.theme().info, cx))
                .child(metric("QTY", quantity, cx.theme().foreground, cx))
                .child(metric("PX", price, cx.theme().warning, cx))
                .child(metric("STATUS", status, cx.theme().muted_foreground, cx))
                .child(div().flex_1())
                .child(
                    Button::new(format!("accept-tender-{tender_id}"))
                        .success()
                        .xsmall()
                        .label("ACCEPT")
                        .on_click(move |_, _, cx| {
                            accept_terminal.update(cx, |terminal, cx| {
                                terminal.tender_action(tender_id, "accept", cx);
                            });
                        }),
                )
                .child(
                    Button::new(format!("decline-tender-{tender_id}"))
                        .danger()
                        .xsmall()
                        .label("DECLINE")
                        .on_click(move |_, _, cx| {
                            decline_terminal.update(cx, |terminal, cx| {
                                terminal.tender_action(tender_id, "decline", cx);
                            });
                        }),
                ),
        );
    }

    div()
        .size_full()
        .min_h_0()
        .overflow_y_scrollbar()
        .child(list)
        .into_any_element()
}

fn render_risk(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (risk, score) = {
        let terminal = terminal.read(cx);
        let risk = terminal
            .client
            .as_ref()
            .and_then(|client| client.risk.as_ref())
            .and_then(|risk| serde_json::to_string_pretty(risk).ok())
            .unwrap_or_else(|| "Risk projection is not available yet.".to_owned());
        let score = terminal
            .client
            .as_ref()
            .and_then(|client| client.score)
            .map_or_else(
                || "SCORE —   RANK —".to_owned(),
                |score| format!("SCORE {}   RANK {}", score.score, score.rank),
            );
        (risk, score)
    };

    v_flex()
        .size_full()
        .min_h_0()
        .p_3()
        .gap_3()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(cx.theme().warning)
                .child(score),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_3()
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().muted)
                .font_family(cx.theme().mono_font_family.clone())
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(risk),
        )
        .into_any_element()
}

