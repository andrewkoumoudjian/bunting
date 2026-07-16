//! Clean-room product workflow views backed only by observed FIX state.

use crate::{
    protocol::FixClient,
    tui::{app::App, ui::styles, widgets::log_panel::LogPanel},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Row, Table},
};

#[expect(
    clippy::too_many_lines,
    reason = "the account workspace renders its three authoritative projection panels together"
)]
pub fn account(frame: &mut Frame, area: Rect, client: &FixClient) {
    let [summary, holdings, risk] = Layout::vertical([
        Constraint::Length(9),
        Constraint::Min(7),
        Constraint::Length(7),
    ])
    .areas(area);
    let lines = client.authoritative_account.as_ref().map_or_else(
        || {
            vec![
                Line::from(Span::styled("AWAITING AUTHORITATIVE AP", styles::warning())),
                Line::from("The terminal does not infer balances from local fills."),
            ]
        },
        |account| {
            vec![
                Line::from(Span::styled(
                    "AUTHORITATIVE COMMITTED ACCOUNT",
                    styles::online(),
                )),
                Line::from(format!(
                    "Participant: {}  Sequence: {}",
                    account.participant_id, account.committed_sequence
                )),
                Line::from(format!(
                    "Order cash: {}  Reserved: {}",
                    account.order_cash, account.order_reserved_cash
                )),
                Line::from(format!(
                    "NLV by currency: {:?}",
                    account.net_liquidation_value
                )),
                Line::from(format!(
                    "PnL: {}  Commission: {}",
                    account.policies.pnl, account.policies.commission
                )),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(lines).block(panel(" ACCOUNT SUMMARY ", client, "AP")),
        summary,
    );
    let holding_rows = client
        .authoritative_account
        .iter()
        .flat_map(|account| &account.holdings)
        .map(|holding| {
            Row::new([
                holding.instrument_id.to_string(),
                holding.position.to_string(),
                holding.reserved.to_string(),
                holding.cost_basis.to_string(),
                holding.realized_pnl.to_string(),
                holding.unrealized_pnl.to_string(),
            ])
        });
    frame.render_widget(
        Table::new(
            holding_rows,
            [
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(10),
                Constraint::Length(14),
                Constraint::Length(14),
                Constraint::Min(14),
            ],
        )
        .header(
            Row::new([
                "INSTR",
                "POSITION",
                "RESERVED",
                "COST",
                "REALIZED",
                "UNREALIZED",
            ])
            .style(styles::label()),
        )
        .block(panel(" POSITIONS AND P&L ", client, "AP")),
        holdings,
    );
    let risk_lines = client.risk.as_ref().map_or_else(
        || vec![Line::from("Awaiting authenticated UB risk projection")],
        |risk| {
            vec![
                Line::from(format!("Policy: {}", risk.policies.risk)),
                Line::from(format!(
                    "Max order={}  Max open={}  Max absolute position={}",
                    risk.limits.max_order_quantity,
                    risk.limits.max_open_order_quantity,
                    risk.limits.max_absolute_position
                )),
                Line::from(format!("Latest score: {:?}", risk.latest_score)),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(risk_lines).block(panel(" RISK AND SCORE ", client, "UB")),
        risk,
    );
}

pub fn simulation(frame: &mut Frame, area: Rect, client: &FixClient) {
    let [run, activity] =
        Layout::vertical([Constraint::Length(8), Constraint::Min(10)]).areas(area);
    let run_lines = client.discovery.as_ref().map_or_else(
        || {
            vec![Line::from(Span::styled(
                "AWAITING SECURITY LIST",
                styles::warning(),
            ))]
        },
        |view| {
            vec![
                Line::from(format!(
                    "Run {}  Scenario {}.{}  Sequence {}",
                    view.run_id, view.scenario_id, view.scenario_version, view.committed_sequence
                )),
                Line::from(format!(
                    "Lifecycle: {:?}  Logical time: {} ns",
                    view.lifecycle, view.logical_time
                )),
                Line::from(format!(
                    "Listings: {}",
                    view.listings
                        .iter()
                        .map(|item| item.symbol.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
                Line::from(format!(
                    "News={}  Tenders={}  Score={}",
                    client.news.len(),
                    client.tenders.len(),
                    client.score.map_or("--".to_owned(), |score| format!(
                        "{} / rank {}",
                        score.score, score.rank
                    ))
                )),
            ]
        },
    );
    frame.render_widget(
        Paragraph::new(run_lines).block(panel(" RUN AND LEADERBOARD ", client, "y")),
        run,
    );
    let [news, tenders] =
        Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
            .areas(activity);
    let news_rows = client.news.iter().rev().map(|item| {
        Row::new([
            item.news_id.to_string(),
            item.published_at.to_string(),
            item.headline.clone(),
        ])
    });
    frame.render_widget(
        Table::new(
            news_rows,
            [
                Constraint::Length(8),
                Constraint::Length(14),
                Constraint::Min(20),
            ],
        )
        .header(Row::new(["ID", "TIME", "HEADLINE"]).style(styles::label()))
        .block(panel(" NEWS ", client, "B")),
        news,
    );
    let tender_rows = client.tenders.iter().map(|tender| {
        Row::new([
            tender.tender_id.to_string(),
            tender.instrument_id.to_string(),
            format!("{:?}", tender.side),
            tender.quantity.to_string(),
            tender.price.to_string(),
            tender.status.clone(),
        ])
    });
    frame.render_widget(
        Table::new(
            tender_rows,
            [
                Constraint::Length(7),
                Constraint::Length(7),
                Constraint::Length(6),
                Constraint::Length(8),
                Constraint::Length(8),
                Constraint::Min(10),
            ],
        )
        .header(Row::new(["ID", "INSTR", "SIDE", "QTY", "PRICE", "STATUS"]).style(styles::label()))
        .block(panel(" TENDERS · /tender accept|decline ID ", client, "U6")),
        tenders,
    );
}

pub fn collaboration(frame: &mut Frame, area: Rect, client: &FixClient) {
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled("CAPABILITY NOT NEGOTIATED", styles::warning())),
            Line::from("Chat, private chat and voice are external collaboration adapters."),
            Line::from("This terminal will expose them only after the selected competition"),
            Line::from("profile advertises an authenticated, bounded collaboration capability."),
            Line::from("Compliance notifications require authorized UB traffic."),
            Line::from("No local control simulates delivery or server success."),
            Line::from(""),
            status_line("Compliance notifications", client, "UB"),
        ])
        .block(
            Block::new()
                .title(" COLLABORATION ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
}

pub fn administration(frame: &mut Frame, area: Rect, client: &FixClient) {
    let configured = client.profile().role;
    let authorized = client
        .verified_role
        .as_deref()
        .is_some_and(|role| matches!(role, "instructor" | "administrator"));
    let message = if configured.privileged() {
        "Verified by the FIX Logon binding. Use /run, /news, /score and /fine workflows."
    } else {
        "Hidden: this profile requests a participant/team role."
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Configured role: ", styles::label()),
                Span::raw(configured.as_str()),
            ]),
            Line::from(vec![
                Span::styled("Verified privileged role: ", styles::label()),
                Span::styled(
                    if authorized { "yes" } else { "no" },
                    if authorized {
                        styles::online()
                    } else {
                        styles::offline()
                    },
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(message, styles::warning())),
            Line::from("Participants, positions, P&L, risk, news, groups and rankings"),
            Line::from("remain deny-by-default until the backend projects an authorized audience."),
            Line::from("/run start|pause|resume|advance N|terminate REASON"),
            Line::from(
                "/news ID public HEADLINE · /score · /fine PARTICIPANT CURRENCY AMOUNT REASON",
            ),
        ])
        .block(
            Block::new()
                .title(" INSTRUCTOR / ASSESSOR / ADMINISTRATION ")
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
}

pub fn session(frame: &mut Frame, area: Rect, app: &App, client: &FixClient) {
    let [health, log] = Layout::vertical([Constraint::Length(11), Constraint::Min(6)]).areas(area);
    let snapshot = client.session_snapshot();
    let profile = client.profile();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(format!(
                "Profile: {}  Endpoint: {}  Transport: {}",
                client.profile_name,
                profile.endpoint,
                profile.transport.label()
            )),
            Line::from(format!(
                "CompIDs: {} -> {}  User: {}  Role request: {}",
                profile.sender_comp_id,
                profile.target_comp_id,
                profile.username,
                profile.role.as_str()
            )),
            Line::from(format!(
                "Team: {}  Run: {}  Workspace: {}",
                profile.team_id.as_deref().unwrap_or("--"),
                profile.run_id.as_deref().unwrap_or("--"),
                app.active_workspace
            )),
            Line::from(format!(
                "State: {:?}  inbound={} outbound={} reconnect={} journal={}",
                snapshot.state,
                snapshot.incoming_sequence,
                snapshot.outgoing_sequence,
                snapshot.reconnect_generation,
                snapshot.journal.len()
            )),
            Line::from(format!(
                "Committed cursor: {}  stale={}  reset={}",
                client.committed_sequence,
                client.stale,
                client.reset_reason.as_deref().unwrap_or("--")
            )),
            Line::from(format!("Status: {}", client.status)),
            Line::from(
                "R reconnect/refresh · /reconnect · /session reset (profile-authorized only)",
            ),
            Line::from("/workspace save|load|remove NAME · ` full redacted FIX journal"),
        ])
        .block(
            Block::new()
                .title(" CONNECTION HEALTH AND RECOVERY ")
                .borders(Borders::ALL)
                .border_style(styles::active_border()),
        ),
        health,
    );
    LogPanel::render(frame, log, &client.logs, false);
}

fn panel(title: &'static str, client: &FixClient, response: &str) -> Block<'static> {
    Block::new()
        .title(title)
        .title_bottom(status_line("server projection", client, response))
        .borders(Borders::ALL)
        .border_style(styles::border())
}

fn status_line(label: &str, client: &FixClient, response: &str) -> Line<'static> {
    let available = client.observed_message_types.contains(response);
    Line::from(vec![
        Span::raw(format!(" {label}: ")),
        Span::styled(
            if available {
                "observed"
            } else {
                "backend unavailable"
            },
            if available {
                styles::online()
            } else {
                styles::warning()
            },
        ),
        Span::raw(" "),
    ])
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::TerminalConfig;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn unavailable_simulation_layout_is_explicit() {
        let profile = TerminalConfig::default().profile("local").unwrap();
        let client =
            FixClient::new("local".to_owned(), profile, Some("test-only".to_owned())).unwrap();
        let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
        terminal
            .draw(|frame| simulation(frame, frame.area(), &client))
            .unwrap();
        let rendered =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut output, cell| {
                    output.push_str(cell.symbol());
                    output
                });
        assert!(rendered.contains("RUN AND LEADERBOARD"));
        assert!(rendered.contains("AWAITING SECURITY LIST"));
        assert!(rendered.contains("TENDERS"));
    }
}
