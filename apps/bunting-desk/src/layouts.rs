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
    /// Books and a ladder beside a ticket, the logs and account panes.
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
                (PaneKind::BookTrader, at(0., 0., 0.52, 0.54)),
                (PaneKind::OrderEntry, at(0.52, 0., 0.24, 0.54)),
                (PaneKind::Tape, at(0.76, 0., 0.24, 0.54)),
                (PaneKind::Blotter, at(0., 0.54, 0.40, 0.46)),
                (PaneKind::Fills, at(0.40, 0.54, 0.30, 0.46)),
                (PaneKind::Portfolio, at(0.70, 0.54, 0.30, 0.23)),
                (PaneKind::TraderInfo, at(0.70, 0.77, 0.30, 0.23)),
            ],
            Self::MarketMaking => vec![
                (PaneKind::BookTrader, at(0., 0., 0.52, 0.50)),
                (PaneKind::LadderTrader, at(0.52, 0., 0.24, 0.50)),
                (PaneKind::OrderEntry, at(0.76, 0., 0.24, 0.50)),
                (PaneKind::Blotter, at(0., 0.50, 0.50, 0.24)),
                (PaneKind::Fills, at(0.50, 0.50, 0.50, 0.24)),
                (PaneKind::Tape, at(0., 0.74, 0.40, 0.26)),
                (PaneKind::Portfolio, at(0.40, 0.74, 0.30, 0.26)),
                (PaneKind::TraderInfo, at(0.70, 0.74, 0.30, 0.26)),
            ],
            Self::Arbitrage => vec![
                (PaneKind::BookTrader, at(0., 0., 0.40, 0.60)),
                (PaneKind::Blotter, at(0., 0.60, 0.40, 0.40)),
                (PaneKind::Tape, at(0.40, 0., 0.30, 0.38)),
                (PaneKind::Fills, at(0.40, 0.38, 0.30, 0.32)),
                (PaneKind::Portfolio, at(0.40, 0.70, 0.30, 0.30)),
                (PaneKind::OrderEntry, at(0.70, 0., 0.30, 0.44)),
                (PaneKind::News, at(0.70, 0.44, 0.30, 0.30)),
                (PaneKind::Notices, at(0.70, 0.74, 0.30, 0.26)),
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
    /// The canvas size the saved rects were laid out for, so a screen
    /// can be reflowed when shown on a different-sized canvas. Missing
    /// (files saved before this was recorded) means the current canvas.
    #[serde(default)]
    pub canvas_w: Option<f32>,
    #[serde(default)]
    pub canvas_h: Option<f32>,
}

impl Screen {
    #[must_use]
    pub fn new(name: impl Into<String>, preset: Preset) -> Self {
        Self {
            name: name.into(),
            preset,
            windows: None,
            canvas_w: None,
            canvas_h: None,
        }
    }

    /// The recorded layout canvas size, when both halves were saved.
    #[must_use]
    pub fn canvas_size(&self) -> Option<(f32, f32)> {
        match (self.canvas_w, self.canvas_h) {
            (Some(w), Some(h)) if w > 0. && h > 0. => Some((w, h)),
            _ => None,
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
        // The owner's laptop canvas (1280x720 logical minus frame) and a
        // large desktop canvas.
        for canvas in [
            Rect::new(0., 0., 1040., 660.),
            Rect::new(0., 0., 1680., 960.),
        ] {
            for preset in Preset::ALL {
                let placed = preset.template(canvas);
                for (index, (kind, rect)) in placed.iter().enumerate() {
                    assert!(rect.x >= 0. && rect.y >= 0., "{preset:?}");
                    assert!(rect.right() <= canvas.right() + 0.01, "{preset:?}");
                    assert!(rect.bottom() <= canvas.bottom() + 0.01, "{preset:?}");
                    let (min_w, min_h) = kind.min_size();
                    assert!(
                        rect.w >= min_w - 0.01 && rect.h >= min_h - 0.01,
                        "{preset:?} {kind:?} is {rect:?}, below its minimum {min_w}x{min_h} on {canvas:?}"
                    );
                    for (_, other) in &placed[index + 1..] {
                        assert!(!rect.overlaps(*other), "{preset:?} overlaps");
                        // Neighbours keep at least the gutter: separated
                        // horizontally or vertically by it.
                        let gap_x = (other.x - rect.right()).max(rect.x - other.right());
                        let gap_y = (other.y - rect.bottom()).max(rect.y - other.bottom());
                        assert!(
                            gap_x >= crate::canvas::geometry::GUTTER - 0.01
                                || gap_y >= crate::canvas::geometry::GUTTER - 0.01,
                            "{preset:?} neighbours closer than the gutter: {rect:?} {other:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_trading_template_reflows_between_desktop_and_laptop() {
        use crate::canvas::geometry::{GUTTER, reflow};
        let desktop = Rect::new(0., 0., 1600., 940.);
        let laptop = Rect::new(0., 0., 1040., 660.);
        let placed = Preset::Trading.template(desktop);
        let rects: Vec<Rect> = placed.iter().map(|(_, rect)| *rect).collect();
        let mins: Vec<(f32, f32)> = placed.iter().map(|(kind, _)| kind.min_size()).collect();
        let flowed = reflow(&rects, desktop, laptop, &mins);
        let inner = laptop.inset(GUTTER);
        for (index, rect) in flowed.iter().enumerate() {
            assert!(
                rect.x >= inner.x - 0.01
                    && rect.y >= inner.y - 0.01
                    && rect.right() <= inner.right() + 0.01
                    && rect.bottom() <= inner.bottom() + 0.01,
                "inside the gutter-inset laptop canvas: {rect:?}"
            );
            for other in &flowed[index + 1..] {
                assert!(!rect.overlaps(*other), "no overlap after reflow");
            }
        }
        let rightmost = flowed
            .iter()
            .map(|rect| rect.right())
            .fold(f32::MIN, f32::max);
        assert!((rightmost - inner.right()).abs() < 0.01);
        let back = reflow(&flowed, laptop, desktop, &mins);
        for (after, before) in back.iter().zip(&rects) {
            assert!(
                (after.x - before.x).abs() < 1.
                    && (after.y - before.y).abs() < 1.
                    && (after.w - before.w).abs() < 1.
                    && (after.h - before.h).abs() < 1.,
                "round trip restores within a pixel: {after:?} vs {before:?}"
            );
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
