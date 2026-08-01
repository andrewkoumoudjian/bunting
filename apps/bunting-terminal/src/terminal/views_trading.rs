fn render_chart(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (candles, bid, ask, spread, snapshot) = {
        let terminal = terminal.read(cx);
        let (bid, ask, spread) = terminal.client.as_ref().map_or((None, None, None), |client| {
            let bid = client.book.bids.first().map(|level| level.0);
            let ask = client.book.asks.first().map(|level| level.0);
            (bid, ask, bid.zip(ask).map(|(bid, ask)| ask - bid))
        });
        (
            terminal.quote_candles(),
            bid,
            ask,
            spread,
            terminal.snapshot(),
        )
    };

    v_flex()
        .size_full()
        .min_h_0()
        .child(
            h_flex()
                .h(px(42.))
                .flex_none()
                .gap_5()
                .px_3()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().list_head)
                .child(metric("SYMBOL", "BNT", cx.theme().foreground, cx))
                .child(metric(
                    "BID",
                    value_or_dash(bid),
                    cx.theme().success,
                    cx,
                ))
                .child(metric(
                    "ASK",
                    value_or_dash(ask),
                    cx.theme().danger,
                    cx,
                ))
                .child(metric(
                    "SPREAD",
                    value_or_dash(spread),
                    cx.theme().warning,
                    cx,
                ))
                .child(div().flex_1())
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("FIX L1 • 72 QUOTE WINDOW"),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .p_3()
                .when(candles.is_empty(), |element| {
                    element.child(empty_state(
                        if snapshot.connected {
                            "Waiting for the first market snapshot"
                        } else {
                            "Market data is offline"
                        },
                        snapshot.status,
                        !snapshot.connected,
                        cx,
                    ))
                })
                .when(!candles.is_empty(), |element| {
                    element.child(
                        CandlestickChart::new(candles)
                            .x(|candle| candle.label.clone())
                            .open(|candle| candle.open)
                            .high(|candle| candle.high)
                            .low(|candle| candle.low)
                            .close(|candle| candle.close)
                            .body_width_ratio(0.55)
                            .tick_margin(8),
                    )
                }),
        )
        .into_any_element()
}

fn render_order_book(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (asks, bids, book_sequence, committed_sequence, connected) = {
        let terminal = terminal.read(cx);
        terminal.client.as_ref().map_or_else(
            || (Vec::new(), Vec::new(), "-".to_owned(), "-".to_owned(), false),
            |client| {
                (
                    client
                        .book
                        .iter()
                        .take(12)
                        .rev()
                        .copied()
                        .collect::<Vec<_>>(),
                    client.book.bids.iter().take(12).copied().collect::<Vec<_>>(),
                    client.book_sequence.clone(),
                    client.committed_sequence.clone(),
                    !client.stale,
                )
            },
        )
    };

    let mut body = TableBody::new();
    for (price, quantity) in asks {
        body = body.child(book_table_row("ASK", price, quantity, cx.theme().danger));
    }
    if connected {
        body = body.child(
            TableRow::new()
                .bg(cx.theme().muted)
                .child(TableCell::new().child("SEQ"))
                .child(
                    TableCell::new()
                        .text_right()
                        .child(format!("{book_sequence}/{committed_sequence}")),
                )
                .child(TableCell::new().text_right().child("COMMITTED")),
        );
    }
    for (price, quantity) in bids {
        body = body.child(book_table_row("BID", price, quantity, cx.theme().success));
    }

    let quick_buy = terminal.clone();
    let quick_sell = terminal.clone();

    v_flex()
        .size_full()
        .min_h_0()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    Table::new()
                        .border_0()
                        .rounded_none()
                        .child(
                            TableHeader::new().child(
                                TableRow::new()
                                    .child(TableHead::new().child("SIDE"))
                                    .child(TableHead::new().text_right().child("PRICE"))
                                    .child(TableHead::new().text_right().child("QTY")),
                            ),
                        )
                        .child(body),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_2()
                .p_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(
                    Button::new("quick-buy")
                        .success()
                        .small()
                        .label("BUY ASK")
                        .on_click(move |_, _, cx| {
                            quick_buy.update(cx, |terminal, cx| {
                                terminal.submit_at_best("buy", cx);
                            });
                        }),
                )
                .child(
                    Button::new("quick-sell")
                        .danger()
                        .small()
                        .label("SELL BID")
                        .on_click(move |_, _, cx| {
                            quick_sell.update(cx, |terminal, cx| {
                                terminal.submit_at_best("sell", cx);
                            });
                        }),
                ),
        )
        .into_any_element()
}

fn render_order_ticket(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (quantity_input, price_input, market_order, snapshot) = {
        let terminal = terminal.read(cx);
        (
            terminal.quantity_input.clone(),
            terminal.price_input.clone(),
            terminal.market_order,
            terminal.snapshot(),
        )
    };
    let limit_terminal = terminal.clone();
    let market_terminal = terminal.clone();
    let buy_terminal = terminal.clone();
    let sell_terminal = terminal.clone();

    v_flex()
        .size_full()
        .min_h_0()
        .p_3()
        .gap_3()
        .child(
            h_flex()
                .justify_between()
                .child(section_label("ORDER TYPE", cx))
                .child(
                    div()
                        .text_xs()
                        .text_color(if snapshot.connected {
                            cx.theme().success
                        } else {
                            cx.theme().danger
                        })
                        .child(if snapshot.connected { "LIVE" } else { "OFFLINE" }),
                ),
        )
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("limit-order")
                        .small()
                        .label("LIMIT")
                        .when(!market_order, |button| button.primary())
                        .when(market_order, |button| button.secondary())
                        .on_click(move |_, _, cx| {
                            limit_terminal.update(cx, |terminal, cx| {
                                terminal.set_market_order(false, cx);
                            });
                        }),
                )
                .child(
                    Button::new("market-order")
                        .small()
                        .label("MARKET")
                        .when(market_order, |button| button.primary())
                        .when(!market_order, |button| button.secondary())
                        .on_click(move |_, _, cx| {
                            market_terminal.update(cx, |terminal, cx| {
                                terminal.set_market_order(true, cx);
                            });
                        }),
                ),
        )
        .child(section_label("QUANTITY (LOTS)", cx))
        .child(Input::new(&quantity_input))
        .child(section_label("PRICE (TICKS)", cx))
        .child(
            div()
                .when(market_order, |element| {
                    element
                        .h(px(34.))
                        .flex()
                        .items_center()
                        .px_2()
                        .rounded(cx.theme().radius)
                        .bg(cx.theme().muted)
                        .text_color(cx.theme().muted_foreground)
                        .child("Server determines execution price")
                })
                .when(!market_order, |element| {
                    element.child(Input::new(&price_input))
                }),
        )
        .child(div().flex_1())
        .child(
            h_flex()
                .gap_2()
                .child(
                    Button::new("submit-buy")
                        .success()
                        .label("BUY")
                        .on_click(move |_, _, cx| {
                            buy_terminal.update(cx, |terminal, cx| {
                                terminal.submit_order("buy", cx);
                            });
                        }),
                )
                .child(
                    Button::new("submit-sell")
                        .danger()
                        .label("SELL")
                        .on_click(move |_, _, cx| {
                            sell_terminal.update(cx, |terminal, cx| {
                                terminal.submit_order("sell", cx);
                            });
                        }),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Commands enter the bounded FIX session; server risk and lifecycle checks remain authoritative."),
        )
        .into_any_element()
}

fn render_orders(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (cancel_input, executions) = {
        let terminal = terminal.read(cx);
        let executions = terminal.client.as_ref().map_or_else(Vec::new, |client| {
            client
                .executions
                .iter()
                .rev()
                .take(64)
                .map(|execution| {
                    (
                        execution.order_id.clone(),
                        execution.kind.clone(),
                        execution.order_status.clone(),
                        execution.reason.clone(),
                    )
                })
                .collect::<Vec<_>>()
        });
        (terminal.cancel_input.clone(), executions)
    };

    let mut body = TableBody::new();
    for (order_id, event, status, reason) in executions {
        body = body.child(
            TableRow::new()
                .child(TableCell::new().w(px(150.)).child(order_id))
                .child(TableCell::new().w(px(92.)).child(event))
                .child(TableCell::new().w(px(100.)).child(status))
                .child(TableCell::new().child(reason)),
        );
    }
    let cancel_terminal = terminal.clone();

    v_flex()
        .size_full()
        .min_h_0()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(
                    Table::new()
                        .border_0()
                        .rounded_none()
                        .child(
                            TableHeader::new().child(
                                TableRow::new()
                                    .child(TableHead::new().w(px(150.)).child("ORDER ID"))
                                    .child(TableHead::new().w(px(92.)).child("EVENT"))
                                    .child(TableHead::new().w(px(100.)).child("STATUS"))
                                    .child(TableHead::new().child("REASON")),
                            ),
                        )
                        .child(body),
                ),
        )
        .child(
            h_flex()
                .flex_none()
                .gap_2()
                .p_2()
                .border_t_1()
                .border_color(cx.theme().border)
                .child(div().flex_1().child(Input::new(&cancel_input)))
                .child(
                    Button::new("cancel-order")
                        .danger()
                        .small()
                        .label("CANCEL")
                        .on_click(move |_, _, cx| {
                            cancel_terminal.update(cx, |terminal, cx| {
                                terminal.cancel_order(cx);
                            });
                        }),
                ),
        )
        .into_any_element()
}

