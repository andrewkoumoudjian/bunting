// Modified from Longbridge Terminal at commit 05c9bbf7fd1c4ab5c34d5316fedf6e1ed5f1fcc3.
// Copyright 2026 Longbridge. Licensed under Apache-2.0.
// Rust guideline compliant 2026-02-21

use crate::{
    protocol::FixClient,
    tui::{
        app::{App, Tab},
        popup::PopupKind,
        ui::{rect, styles},
        views,
        widgets::log_panel::LogPanel,
    },
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Margin},
    widgets::{Block, Borders},
};

pub fn draw(frame: &mut Frame, app: &App, client: &FixClient) {
    let area = frame.area();
    frame.render_widget(
        Block::new()
            .borders(Borders::ALL)
            .border_style(styles::border()),
        area,
    );
    let inner = area.inner(Margin::new(1, 1));
    let [navbar, content, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(8),
        Constraint::Length(1),
    ])
    .areas(inner);

    views::navbar::render(frame, navbar, app.tab);
    match app.tab {
        Tab::Market => views::market::render(frame, content, app, client),
        Tab::Orders => views::orders::render(frame, content, client),
        Tab::Account => views::workspace::account(frame, content, client),
        Tab::Simulation => views::workspace::simulation(frame, content, client),
        Tab::Collaboration => views::workspace::collaboration(frame, content, client),
        Tab::Administration => views::workspace::administration(frame, content, client),
        Tab::Session => views::workspace::session(frame, content, app, client),
    }
    views::footer::render(frame, footer, &app.status, client);

    if app.popup == PopupKind::FixLog {
        LogPanel::render(
            frame,
            rect::centered_percent(88, 72, inner),
            &client.logs,
            true,
        );
    } else {
        views::popup::render(frame, inner, app);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::TerminalConfig;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn every_workspace_tab_matches_its_full_frame_snapshot() {
        let profile = TerminalConfig::default().profile("local").unwrap();
        let client =
            FixClient::new("local".to_owned(), profile, Some("test-only".to_owned())).unwrap();
        let snapshots = [
            (Tab::Market, 2_860_379_810_998_369_312_u64),
            (Tab::Orders, 15_298_884_515_992_764_846),
            (Tab::Account, 6_625_834_207_265_573_254),
            (Tab::Simulation, 17_137_701_615_183_501_023),
            (Tab::Collaboration, 521_444_836_489_403_141),
            (Tab::Administration, 2_461_448_359_647_326_975),
            (Tab::Session, 1_400_212_887_613_168_380),
        ];
        let mut actual_snapshots = Vec::new();
        for (tab, _) in snapshots {
            let mut app = App::default();
            app.tab = tab;
            let mut terminal = Terminal::new(TestBackend::new(120, 32)).unwrap();
            terminal.draw(|frame| draw(frame, &app, &client)).unwrap();
            let actual = terminal.backend().buffer().content().iter().fold(
                0xcbf2_9ce4_8422_2325_u64,
                |hash, cell| {
                    cell.symbol().as_bytes().iter().fold(hash, |value, byte| {
                        value.wrapping_mul(0x0000_0100_0000_01b3) ^ u64::from(*byte)
                    })
                },
            );
            actual_snapshots.push((tab.name(), actual));
        }
        assert_eq!(
            actual_snapshots,
            snapshots.map(|(tab, hash)| (tab.name(), hash))
        );
    }
}
