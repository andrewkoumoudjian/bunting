//! Workspaces: named layouts of panes, started from a preset, rearranged
//! freely and saved between runs (RIT saves its workspace on exit; this
//! keeps every named workspace and saves on every change).

use crate::desk::Desk;
use crate::panes::PaneKind;
use gpui_kit::component::dock::{DockAreaState, DockLayout};
use gpui_kit::{App, Entity, Window, px};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Saved layouts from another version of the pane tree are not loaded.
pub const LAYOUT_VERSION: usize = 3;
/// Workspaces kept.
pub const MAX_WORKSPACES: usize = 16;

/// A starting arrangement of panes.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Preset {
    /// Books in the centre, ticket and tape on the right, the blotter and
    /// account tabs along the bottom.
    #[default]
    Trading,
    /// Books and a ladder beside two tickets and trader info, blotter and
    /// tape below.
    MarketMaking,
    /// Every venue's book beside the consolidated tape and a ticket.
    Arbitrage,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Trading, Self::MarketMaking, Self::Arbitrage];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Trading => "Trading",
            Self::MarketMaking => "Market Making",
            Self::Arbitrage => "Arbitrage",
        }
    }

    /// The preset's panes, newly built on `desk`.
    #[expect(
        clippy::too_many_lines,
        reason = "each preset's tree reads best whole, beside the others"
    )]
    pub fn layout(self, desk: &Entity<Desk>, window: &mut Window, cx: &mut App) -> DockLayout {
        let mut pane = |kind: PaneKind| kind.build(desk.clone(), window, cx);
        match self {
            Self::Trading => {
                let books = pane(PaneKind::BookTrader);
                let ticket = pane(PaneKind::OrderEntry);
                let tape = pane(PaneKind::Tape);
                let bottom = [
                    pane(PaneKind::Blotter),
                    pane(PaneKind::Fills),
                    pane(PaneKind::Portfolio),
                    pane(PaneKind::TraderInfo),
                    pane(PaneKind::News),
                    pane(PaneKind::Deals),
                    pane(PaneKind::Notices),
                ];
                let mut tabs = DockLayout::tabs();
                for each in bottom {
                    tabs = tabs.panel_view(each, cx);
                }
                DockLayout::v_split()
                    .child(
                        DockLayout::h_split()
                            .child(DockLayout::tabs().panel_view(books, cx), None)
                            .child(
                                DockLayout::v_split()
                                    .child(DockLayout::tabs().panel_view(ticket, cx), None)
                                    .child(DockLayout::tabs().panel_view(tape, cx), Some(px(260.))),
                                Some(px(380.)),
                            ),
                        None,
                    )
                    .child(tabs, Some(px(250.)))
            }
            Self::MarketMaking => {
                let books = pane(PaneKind::BookTrader);
                let ladder = pane(PaneKind::LadderTrader);
                let first_ticket = pane(PaneKind::OrderEntry);
                let second_ticket = pane(PaneKind::OrderEntry);
                let info = pane(PaneKind::TraderInfo);
                let portfolio = pane(PaneKind::Portfolio);
                let blotter = pane(PaneKind::Blotter);
                let fills = pane(PaneKind::Fills);
                let tape = pane(PaneKind::Tape);
                let news = pane(PaneKind::News);
                let deals = pane(PaneKind::Deals);
                let notices = pane(PaneKind::Notices);
                DockLayout::v_split()
                    .child(
                        DockLayout::h_split()
                            .child(
                                DockLayout::tabs()
                                    .panel_view(books, cx)
                                    .panel_view(ladder, cx),
                                None,
                            )
                            .child(
                                DockLayout::tabs().panel_view(first_ticket, cx),
                                Some(px(300.)),
                            )
                            .child(
                                DockLayout::tabs().panel_view(second_ticket, cx),
                                Some(px(300.)),
                            )
                            .child(
                                DockLayout::tabs()
                                    .panel_view(info, cx)
                                    .panel_view(portfolio, cx),
                                Some(px(280.)),
                            ),
                        None,
                    )
                    .child(
                        DockLayout::h_split()
                            .child(
                                DockLayout::tabs()
                                    .panel_view(blotter, cx)
                                    .panel_view(fills, cx)
                                    .panel_view(deals, cx),
                                None,
                            )
                            .child(
                                DockLayout::tabs()
                                    .panel_view(tape, cx)
                                    .panel_view(news, cx)
                                    .panel_view(notices, cx),
                                Some(px(520.)),
                            ),
                        Some(px(260.)),
                    )
            }
            Self::Arbitrage => {
                let books = pane(PaneKind::BookTrader);
                let tape = pane(PaneKind::Tape);
                let ticket = pane(PaneKind::OrderEntry);
                let bottom = [
                    pane(PaneKind::Blotter),
                    pane(PaneKind::Fills),
                    pane(PaneKind::Portfolio),
                    pane(PaneKind::News),
                    pane(PaneKind::Notices),
                ];
                let mut tabs = DockLayout::tabs();
                for each in bottom {
                    tabs = tabs.panel_view(each, cx);
                }
                DockLayout::v_split()
                    .child(
                        DockLayout::h_split()
                            .child(DockLayout::tabs().panel_view(books, cx), None)
                            .child(DockLayout::tabs().panel_view(tape, cx), Some(px(340.)))
                            .child(DockLayout::tabs().panel_view(ticket, cx), Some(px(340.))),
                        None,
                    )
                    .child(tabs, Some(px(240.)))
            }
        }
    }
}

/// One named workspace and its last layout.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Saved {
    pub name: String,
    pub preset: Preset,
    /// `None` until the workspace is first shown.
    pub layout: Option<DockAreaState>,
}

impl Saved {
    #[must_use]
    pub fn new(name: impl Into<String>, preset: Preset) -> Self {
        Self {
            name: name.into(),
            preset,
            layout: None,
        }
    }

    /// One workspace per preset: what a first run shows.
    #[must_use]
    pub fn defaults() -> Vec<Self> {
        Preset::ALL
            .into_iter()
            .map(|preset| Self::new(preset.name(), preset))
            .collect()
    }
}

#[derive(Deserialize, Serialize)]
struct File {
    version: usize,
    active: usize,
    workspaces: Vec<Saved>,
}

/// Where workspaces are saved: one JSON file, replaced whole on each save.
#[derive(Clone, Debug)]
pub struct LayoutStore {
    path: PathBuf,
}

impl LayoutStore {
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `$XDG_CONFIG_HOME/bunting/desk/workspaces.json` (or the platform's
    /// configuration directory).
    #[must_use]
    pub fn default_location() -> Self {
        Self::at(
            config_dir()
                .join("bunting")
                .join("desk")
                .join("workspaces.json"),
        )
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved workspaces and the active one, if a readable file of this
    /// version exists.
    #[must_use]
    pub fn load(&self) -> Option<(Vec<Saved>, usize)> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        let file: File = serde_json::from_str(&text).ok()?;
        if file.version != LAYOUT_VERSION || file.workspaces.is_empty() {
            return None;
        }
        let mut workspaces = file.workspaces;
        workspaces.truncate(MAX_WORKSPACES);
        let active = file.active.min(workspaces.len() - 1);
        Some((workspaces, active))
    }

    /// Writes every workspace.
    ///
    /// # Errors
    /// Returns why the file could not be written.
    pub fn save(&self, workspaces: &[Saved], active: usize) -> std::io::Result<()> {
        let file = File {
            version: LAYOUT_VERSION,
            active,
            workspaces: workspaces.to_vec(),
        };
        let text = serde_json::to_string_pretty(&file).map_err(std::io::Error::other)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write beside the file and rename, so a crash never leaves half.
        let partial = self.path.with_extension("json.partial");
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, &self.path)
    }
}

fn config_dir() -> PathBuf {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if cfg!(target_os = "windows") {
        return var("APPDATA").unwrap_or_else(std::env::temp_dir);
    }
    if cfg!(target_os = "macos") {
        return var("HOME").map_or_else(std::env::temp_dir, |home| {
            home.join("Library").join("Application Support")
        });
    }
    var("XDG_CONFIG_HOME")
        .or_else(|| var("HOME").map(|home| home.join(".config")))
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::unwrap_used)]
    fn workspaces_round_trip_through_the_file() {
        let dir = std::env::temp_dir().join(format!("bunting-desk-layouts-{}", std::process::id()));
        let store = LayoutStore::at(dir.join("workspaces.json"));
        let mut workspaces = Saved::defaults();
        workspaces.push(Saved::new("Mine", Preset::Arbitrage));
        store.save(&workspaces, 3).unwrap();
        assert_eq!(store.load(), Some((workspaces, 3)));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
