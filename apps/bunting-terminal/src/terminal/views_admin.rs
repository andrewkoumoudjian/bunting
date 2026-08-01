fn render_competition(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (run, scenario, lifecycle, logical_time, listings) = {
        let terminal = terminal.read(cx);
        terminal
            .client
            .as_ref()
            .and_then(|client| client.discovery.as_ref())
            .map_or_else(
                || {
                    (
                        "-".to_owned(),
                        "-".to_owned(),
                        "-".to_owned(),
                        "-".to_owned(),
                        "-".to_owned(),
                    )
                },
                |view| {
                    (
                        view.run_id.to_string(),
                        format!("{} v{}", view.scenario_id, view.scenario_version),
                        format!("{:?}", view.lifecycle),
                        view.logical_time.to_string(),
                        view.listings
                            .iter()
                            .map(|listing| listing.symbol.as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                },
            )
    };

    let start_terminal = terminal.clone();
    let pause_terminal = terminal.clone();
    let resume_terminal = terminal.clone();
    let score_terminal = terminal.clone();

    v_flex()
        .size_full()
        .min_h_0()
        .p_3()
        .gap_3()
        .child(
            h_flex()
                .gap_5()
                .child(metric("RUN", run, cx.theme().foreground, cx))
                .child(metric("SCENARIO", scenario, cx.theme().info, cx))
                .child(metric("STATE", lifecycle, cx.theme().warning, cx))
                .child(metric(
                    "LOGICAL NS",
                    logical_time,
                    cx.theme().muted_foreground,
                    cx,
                )),
        )
        .child(
            div()
                .p_2()
                .rounded(cx.theme().radius)
                .bg(cx.theme().muted)
                .text_sm()
                .child(format!("LISTINGS  {listings}")),
        )
        .child(section_label("INSTRUCTOR / ADMINISTRATOR", cx))
        .child(
            h_flex()
                .flex_wrap()
                .gap_2()
                .child(
                    Button::new("run-start")
                        .success()
                        .small()
                        .label("START")
                        .on_click(move |_, _, cx| {
                            start_terminal.update(cx, |terminal, cx| {
                                terminal.run_action("start", cx);
                            });
                        }),
                )
                .child(
                    Button::new("run-pause")
                        .secondary()
                        .small()
                        .label("PAUSE")
                        .on_click(move |_, _, cx| {
                            pause_terminal.update(cx, |terminal, cx| {
                                terminal.run_action("pause", cx);
                            });
                        }),
                )
                .child(
                    Button::new("run-resume")
                        .primary()
                        .small()
                        .label("RESUME")
                        .on_click(move |_, _, cx| {
                            resume_terminal.update(cx, |terminal, cx| {
                                terminal.run_action("resume", cx);
                            });
                        }),
                )
                .child(
                    Button::new("run-score")
                        .secondary()
                        .small()
                        .label("SCORE")
                        .on_click(move |_, _, cx| {
                            score_terminal.update(cx, |terminal, cx| {
                                terminal.run_action("score", cx);
                            });
                        }),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("Controls remain subject to server identity, role and lifecycle validation."),
        )
        .into_any_element()
}

fn render_session(terminal: Entity<Terminal>, cx: &mut Context<MarketPanel>) -> AnyElement {
    let (snapshot, endpoint, book_sequence, committed_sequence, logs) = {
        let terminal = terminal.read(cx);
        terminal.client.as_ref().map_or_else(
            || {
                (
                    terminal.snapshot(),
                    "-".to_owned(),
                    "-".to_owned(),
                    "-".to_owned(),
                    Vec::new(),
                )
            },
            |client| {
                (
                    terminal.snapshot(),
                    client.profile().endpoint.clone(),
                    client.book_sequence.clone(),
                    client.committed_sequence.clone(),
                    client.logs.iter().rev().take(128).cloned().collect::<Vec<_>>(),
                )
            },
        )
    };

    let mut log_rows = v_flex().gap_1();
    for line in logs {
        let color = if line.starts_with("OUT") {
            cx.theme().info
        } else {
            cx.theme().muted_foreground
        };
        log_rows = log_rows.child(div().text_xs().text_color(color).child(line));
    }

    v_flex()
        .size_full()
        .min_h_0()
        .child(
            h_flex()
                .flex_none()
                .gap_5()
                .px_3()
                .py_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().list_head)
                .child(metric(
                    "PROFILE",
                    snapshot.profile,
                    cx.theme().foreground,
                    cx,
                ))
                .child(metric("ENDPOINT", endpoint, cx.theme().info, cx))
                .child(metric(
                    "BOOK SEQ",
                    book_sequence,
                    cx.theme().warning,
                    cx,
                ))
                .child(metric(
                    "COMMIT",
                    committed_sequence,
                    cx.theme().success,
                    cx,
                )),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_3()
                .font_family(cx.theme().mono_font_family.clone())
                .child(log_rows),
        )
        .into_any_element()
}

fn status_dot(connected: bool, cx: &App) -> AnyElement {
    div()
        .size(px(7.))
        .rounded_full()
        .bg(if connected {
            cx.theme().success
        } else {
            cx.theme().danger
        })
        .into_any_element()
}

fn section_label(label: &'static str, cx: &App) -> AnyElement {
    div()
        .text_xs()
        .font_weight(gpui::FontWeight::BOLD)
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

fn metric(
    label: &'static str,
    value: impl Into<SharedString>,
    color: Hsla,
    cx: &App,
) -> AnyElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .text_color(color)
                .child(value.into()),
        )
        .into_any_element()
}

fn book_table_row(side: &'static str, price: i64, quantity: i64, color: Hsla) -> TableRow {
    TableRow::new()
        .child(TableCell::new().text_color(color).child(side))
        .child(
            TableCell::new()
                .text_right()
                .text_color(color)
                .child(price.to_string()),
        )
        .child(TableCell::new().text_right().child(quantity.to_string()))
}

fn empty_state(
    title: &'static str,
    detail: impl Into<SharedString>,
    show_spinner: bool,
    cx: &App,
) -> AnyElement {
    v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .when(show_spinner, |element| {
            element.child(Spinner::new().small().color(cx.theme().muted_foreground))
        })
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::BOLD)
                .child(title),
        )
        .child(
            div()
                .max_w(px(520.))
                .text_center()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.into()),
        )
        .into_any_element()
}

fn value_or_dash(value: Option<i64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| value.to_string())
}
