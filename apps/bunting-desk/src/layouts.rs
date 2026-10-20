//! Screens: named canvases of floating windows, started from a template,
//! arranged freely and saved between runs (ADR 0051 decision 3). This is
//! layout file version 4; a file of any older (or unknown) version is
//! renamed to `layouts.v3.json` beside it — kept, never deleted — and the
//! desk starts from the templates.

use crate::canvas::SavedWindow;
use crate::canvas::geometry::Rect;
use crate::panes::PaneKind;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The screen file version this build reads and writes.
pub const LAYOUT_VERSION: usize = 4;
/// Screens kept.
pub const MAX_SCREENS: usize = 16;

/// A starting arrangement of windows, placed as fractions of the canvas.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum Preset {
    /// Books, ticket and tape above; blotter, fills, portfolio and trader
    /// info below, each its own window.
    #[default]
    Trading,
    /// Books and a ladder beside two tickets, account panes and the logs.
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

    /// The template's windows as fractions of the canvas (x, y, w, h in
    /// 0..1). Edges line up and nothing overlaps, as a careful human would
    /// lay them out.
    #[must_use]
    pub fn fractions(self) -> Vec<(PaneKind, Rect)> {
        let at = |x: f32, y: f32, w: f32, h: f32| Rect::new(x, y, w, h);
        match self {
            Self::Trading => vec![
                (PaneKind::BookTrader, at(0., 0., 0.54, 0.60)),
                (PaneKind::OrderEntry, at(0.54, 0., 0.23, 0.60)),
                (PaneKind::Tape, at(0.77, 0., 0.23, 0.60)),
                (PaneKind::Blotter, at(0., 0.60, 0.54, 0.40)),
                (PaneKind::Fills, at(0.54, 0.60, 0.23, 0.40)),
                (PaneKind::Portfolio, at(0.77, 0.60, 0.23, 0.20)),
                (PaneKind::TraderInfo, at(0.77, 0.80, 0.23, 0.20)),
            ],
            Self::MarketMaking => vec![
                (PaneKind::BookTrader, at(0., 0., 0.34, 0.55)),
                (PaneKind::LadderTrader, at(0.34, 0., 0.18, 0.55)),
                (PaneKind::OrderEntry, at(0.52, 0., 0.16, 0.55)),
                (PaneKind::OrderEntry, at(0.68, 0., 0.16, 0.55)),
                (PaneKind::TraderInfo, at(0.84, 0., 0.16, 0.28)),
                (PaneKind::Portfolio, at(0.84, 0.28, 0.16, 0.27)),
                (PaneKind::Blotter, at(0., 0.55, 0.30, 0.45)),
                (PaneKind::Fills, at(0.30, 0.55, 0.18, 0.45)),
                (PaneKind::Deals, at(0.48, 0.55, 0.18, 0.45)),
                (PaneKind::Tape, at(0.66, 0.55, 0.17, 0.45)),
                (PaneKind::News, at(0.83, 0.55, 0.17, 0.23)),
                (PaneKind::Notices, at(0.83, 0.78, 0.17, 0.22)),
            ],
            Self::Arbitrage => vec![
                (PaneKind::BookTrader, at(0., 0., 0.50, 0.60)),
                (PaneKind::Tape, at(0.50, 0., 0.25, 0.60)),
                (PaneKind::OrderEntry, at(0.75, 0., 0.25, 0.60)),
                (PaneKind::Blotter, at(0., 0.60, 0.40, 0.40)),
                (PaneKind::Fills, at(0.40, 0.60, 0.20, 0.40)),
                (PaneKind::Portfolio, at(0.60, 0.60, 0.20, 0.40)),
                (PaneKind::News, at(0.80, 0.60, 0.20, 0.20)),
                (PaneKind::Notices, at(0.80, 0.80, 0.20, 0.20)),
            ],
        }
    }

    /// The template placed on a canvas of this size, keeping the canvas
    /// gutter between and around windows.
    #[must_use]
    pub fn template(self, canvas: Rect) -> Vec<(PaneKind, Rect)> {
        self.fractions()
            .into_iter()
            .map(|(kind, fraction)| {
                (
                    kind,
                    crate::canvas::geometry::place_fraction(fraction, canvas),
                )
            })
            .collect()
    }
}

/// One named screen and its windows.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Screen {
    pub name: String,
    pub preset: Preset,
    /// `None` until the screen is first shown (its template applies).
    pub windows: Option<Vec<SavedWindow>>,
}

impl Screen {
    #[must_use]
    pub fn new(name: impl Into<String>, preset: Preset) -> Self {
        Self {
            name: name.into(),
            preset,
            windows: None,
        }
    }

    /// One screen per template: what a first run shows.
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
    screens: Vec<Screen>,
}

/// Just the version line, read before deciding what to do with a file.
#[derive(Deserialize)]
struct VersionProbe {
    version: Option<usize>,
}

/// Where screens are saved: one JSON file, replaced whole on each save.
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
    /// configuration directory). The file name predates screens; the
    /// version inside governs.
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

    /// The saved screens and the active one, if a readable file of this
    /// version exists. A file of any other version is renamed to
    /// `layouts.v3.json` beside it, never deleted, and `None` returned so
    /// the desk starts from the defaults.
    #[must_use]
    pub fn load(&self) -> Option<(Vec<Screen>, usize)> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        let version = serde_json::from_str::<VersionProbe>(&text)
            .ok()
            .and_then(|probe| probe.version);
        if version != Some(LAYOUT_VERSION) {
            self.set_aside_legacy();
            return None;
        }
        let file: File = serde_json::from_str(&text).ok()?;
        if file.screens.is_empty() {
            return None;
        }
        let mut screens = file.screens;
        screens.truncate(MAX_SCREENS);
        let active = file.active.min(screens.len() - 1);
        Some((screens, active))
    }

    /// Renames the current file to `layouts.v3.json` beside it (numbered if
    /// that name is taken), keeping the old layouts recoverable by hand.
    fn set_aside_legacy(&self) {
        let dir = self
            .path
            .parent()
            .map_or_else(|| PathBuf::from("."), std::borrow::ToOwned::to_owned);
        let mut target = dir.join("layouts.v3.json");
        let mut number = 1;
        while target.exists() {
            number += 1;
            target = dir.join(format!("layouts.v3.{number}.json"));
        }
        if let Err(error) = std::fs::rename(&self.path, &target) {
            eprintln!(
                "bunting-desk: old layouts not set aside to {}: {error}",
                target.display()
            );
        }
    }

    /// Writes every screen.
    ///
    /// # Errors
    /// Returns why the file could not be written.
    pub fn save(&self, screens: &[Screen], active: usize) -> std::io::Result<()> {
        let file = File {
            version: LAYOUT_VERSION,
            active,
            screens: screens.to_vec(),
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

    fn temp_store(tag: &str) -> (PathBuf, LayoutStore) {
        let dir = std::env::temp_dir().join(format!(
            "bunting-desk-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |elapsed| elapsed.as_nanos())
        ));
        let store = LayoutStore::at(dir.join("workspaces.json"));
        (dir, store)
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn screens_round_trip_through_the_file() {
        let (dir, store) = temp_store("round-trip");
        let mut screens = Screen::defaults();
        screens.push(Screen::new("Mine", Preset::Arbitrage));
        screens[3].windows = Some(vec![SavedWindow {
            kind: PaneKind::BookTrader.name().to_owned(),
            rect: Rect::new(10., 20., 640., 420.),
            z: 0,
            minimised: false,
            maximised: true,
        }]);
        store.save(&screens, 3).unwrap();
        assert_eq!(store.load(), Some((screens, 3)));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn a_version_3_file_is_renamed_not_deleted() {
        let (dir, store) = temp_store("legacy");
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = r#"{"version":3,"active":0,"workspaces":[]}"#;
        std::fs::write(store.path(), legacy).unwrap();
        assert_eq!(store.load(), None);
        assert!(!store.path().exists(), "the old file moved aside");
        let aside = dir.join("layouts.v3.json");
        assert_eq!(std::fs::read_to_string(&aside).unwrap(), legacy);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    #[allow(clippy::unwrap_used)]
    fn an_unknown_version_file_is_also_set_aside() {
        let (dir, store) = temp_store("unknown");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(store.path(), r#"{"version":99}"#).unwrap();
        assert_eq!(store.load(), None);
        assert!(dir.join("layouts.v3.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn templates_never_overlap_and_stay_on_the_canvas() {
        let canvas = Rect::new(0., 0., 1600., 1000.);
        for preset in Preset::ALL {
            let rects: Vec<Rect> = preset
                .template(canvas)
                .into_iter()
                .map(|(_, rect)| rect)
                .collect();
            for (index, rect) in rects.iter().enumerate() {
                assert!(rect.x >= 0. && rect.y >= 0., "{preset:?}");
                assert!(rect.right() <= canvas.right() + 0.01, "{preset:?}");
                assert!(rect.bottom() <= canvas.bottom() + 0.01, "{preset:?}");
                for other in &rects[index + 1..] {
                    assert!(!rect.overlaps(*other), "{preset:?} overlaps");
                }
            }
        }
    }

    #[test]
    fn trading_opens_its_logs_as_separate_windows() {
        let kinds: Vec<PaneKind> = Preset::Trading
            .fractions()
            .into_iter()
            .map(|(kind, _)| kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                PaneKind::BookTrader,
                PaneKind::OrderEntry,
                PaneKind::Tape,
                PaneKind::Blotter,
                PaneKind::Fills,
                PaneKind::Portfolio,
                PaneKind::TraderInfo,
            ]
        );
    }
}
