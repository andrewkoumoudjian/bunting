//! Clean-room product workflow views backed only by observed FIX state.

use crate::{
    protocol::FixClient,
    tui::{app::App, ui::styles, widgets::log_panel::LogPanel},
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table},
};

pub fn account(frame: &mut Frame, area: Rect, client: &FixClient) {
    let [summary, workflows] =
        Layout::vertical([Constraint::Length(9), Constraint::Min(8)]).areas(area);
    let mark = client
        .book
        .bids
        .first()
        .zip(client.book.asks.first())
        .map(|(bid, ask)| bid.0.saturating_add(ask.0) / 2);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "LOCAL NON-AUTHORITATIVE FILL PROJECTION",
                styles::warning(),
            )),
            Line::from(format!("Position: {}", client.portfolio.position)),
            Line::from(format!(
                "Observed-fill cash delta: {}",
                client.portfolio.cash
            )),
            Line::from(format!(
                "Marked delta: {}",
                mark.map_or_else(
                    || "--".to_owned(),
                    |value| client.portfolio.marked_value(value).to_string()
                )
            )),
            Line::from("Server balances, buying power, cost basis, P&L and NLV require U4."),
        ])
        .block(panel(" ACCOUNT SUMMARY ", client, "U4")),
        summary,
    );
    capability_table(
        frame,
        workflows,
        " ACCOUNT, RISK AND ANALYTICS ",
        client,
        &[
            ("Positions", "AN/AP", "AP"),
            ("Cash / buying power / cost / P&L / NLV", "U3/U4", "U4"),
            ("Risk limits / exposure / penalties", "U3/U4", "U4"),
            ("Trade blotter / transaction log", "U9", "U9"),
            (
                "OHLC / volume / MA / EMA / RSI / annotations",
                "AD/AE",
                "AE",
            ),
        ],
    );
}

pub fn simulation(frame: &mut Frame, area: Rect, client: &FixClient) {
    capability_table(
        frame,
        area,
        " SIMULATION INFORMATION AND INSTITUTIONAL WORKFLOWS ",
        client,
        &[
            (
                "Run clock, period, lifecycle and run selection",
                "U1/U2",
                "U2",
            ),
            ("Instrument security master and capabilities", "x/y", "y"),
            ("News", "U5", "U5"),
            ("Tenders", "U6", "U6"),
            ("OTC / spreads / transport / composites", "U7", "U7"),
            ("Assets / leases / facilities / conversions", "U8", "U8"),
            ("Score / ranking / reports / downloads", "U9", "U9"),
        ],
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
    let authorized = false;
    let message = if configured.privileged() {
        "Hidden: the current FIX mapping does not return a verified instructor/admin claim."
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
                Span::styled(if authorized { "yes" } else { "no" }, styles::offline()),
            ]),
            Line::from(""),
            Line::from(Span::styled(message, styles::warning())),
            Line::from("Participants, positions, P&L, risk, news, groups and rankings"),
            Line::from("remain deny-by-default until the backend projects an authorized audience."),
            Line::from("Run controls require UA; monitoring/compliance requires UB."),
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

fn capability_table(
    frame: &mut Frame,
    area: Rect,
    title: &'static str,
    client: &FixClient,
    rows: &[(&str, &str, &str)],
) {
    let rows = rows.iter().map(|(workflow, source, response)| {
        let available = client.observed_message_types.contains(*response);
        Row::new([
            Cell::from(*workflow),
            Cell::from(*source),
            Cell::from(if available {
                "OBSERVED"
            } else {
                "BACKEND UNAVAILABLE"
            })
            .style(if available {
                styles::online()
            } else {
                styles::warning()
            }),
        ])
    });
    frame.render_widget(
        Table::new(
            rows,
            [
                Constraint::Percentage(58),
                Constraint::Percentage(14),
                Constraint::Percentage(28),
            ],
        )
        .header(Row::new(["WORKFLOW", "SOURCE", "STATE"]).style(styles::label()))
        .block(
            Block::new()
                .title(title)
                .borders(Borders::ALL)
                .border_style(styles::border()),
        ),
        area,
    );
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
    fn golden_unavailable_simulation_layout_is_explicit() {
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
        assert!(rendered.contains("SIMULATION INFORMATION"));
        assert!(rendered.contains("BACKEND UNAVAILABLE"));
        assert!(rendered.contains("Tenders"));
    }
}
