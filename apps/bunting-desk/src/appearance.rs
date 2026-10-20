//! Appearance settings: the chosen theme, the UI font size and the
//! density, saved between runs (ADR 0051 decision 7, slice U2a).
//!
//! The settings live in `appearance.json` beside the screen layouts:
//! the directory comes from [`LayoutStore::default_location`], so the
//! two files can never disagree about where the desk's configuration
//! lives. User themes are `*.json` files in the `themes` folder of the
//! same directory, hot-reloaded through `ThemeRegistry::watch_dir`; a
//! file that fails to parse is logged and skipped, never fatal.
//!
//! Everything visual that needs the density reads [`metrics`], which
//! reads the [`AppearanceSettings`] global installed by [`install`].

use crate::actions;
use crate::layouts::LayoutStore;
use crate::theme;
use gpui_kit::component::command::{CommandGroup, CommandItem};
use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeRegistry, ThemeSet};
use gpui_kit::{App, Div, Global, InteractiveElement as _, Pixels, Stateful, px};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// The appearance file version this build reads and writes.
pub const APPEARANCE_VERSION: usize = 1;
/// The smallest UI font size, in points.
pub const MIN_UI_FONT_SIZE: f32 = 12.;
/// The largest UI font size, in points.
pub const MAX_UI_FONT_SIZE: f32 = 16.;
/// The UI font size a first run uses, in points.
pub const DEFAULT_UI_FONT_SIZE: f32 = 13.;

/// Keeps a UI font size inside the supported range.
#[must_use]
pub fn clamp_ui_font_size(size: f32) -> f32 {
    if size.is_nan() {
        DEFAULT_UI_FONT_SIZE
    } else {
        size.clamp(MIN_UI_FONT_SIZE, MAX_UI_FONT_SIZE)
    }
}

/// How much room controls and rows get (ADR 0051 decision 7).
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Compact,
    #[default]
    Default,
    Comfortable,
}

impl Density {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Default, Self::Comfortable];

    /// The name written to `appearance.json` and shown in the palette.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Default => "default",
            Self::Comfortable => "comfortable",
        }
    }

    /// The next density, cycling back to compact after comfortable.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Compact => Self::Default,
            Self::Default => Self::Comfortable,
            Self::Comfortable => Self::Compact,
        }
    }

    /// The measurements every surface sizes itself from at this density.
    #[must_use]
    pub fn metrics(self, ui_font_size: f32) -> Metrics {
        let (control, row, title_strip, sidebar_row) = match self {
            Self::Compact => (22., 22., 24., 24.),
            Self::Default => (26., 24., 28., 28.),
            Self::Comfortable => (30., 28., 32., 32.),
        };
        Metrics {
            control_height: px(control),
            row_height: px(row),
            title_strip_height: px(title_strip),
            sidebar_row_height: px(sidebar_row),
            font_size: px(clamp_ui_font_size(ui_font_size)),
        }
    }
}

/// The sizes one density and font size produce. Pane code reads these
/// through [`metrics`] instead of repeating pixel literals.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    /// Inputs and buttons.
    pub control_height: Pixels,
    /// Table and list rows.
    pub row_height: Pixels,
    /// A floating window's title strip.
    pub title_strip_height: Pixels,
    /// Sidebar rows and section headers.
    pub sidebar_row_height: Pixels,
    /// The base UI font size.
    pub font_size: Pixels,
}

/// The saved appearance: one theme by name, one font size, one density.
#[derive(Clone, Debug, PartialEq)]
pub struct Appearance {
    pub theme: String,
    pub ui_font_size: f32,
    pub density: Density,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: theme::BUNTING_DARK.to_owned(),
            ui_font_size: DEFAULT_UI_FONT_SIZE,
            density: Density::Default,
        }
    }
}

impl Appearance {
    /// Brings hand-edited values back inside their ranges.
    pub fn sanitize(&mut self) {
        self.ui_font_size = clamp_ui_font_size(self.ui_font_size);
        if self.theme.trim().is_empty() {
            theme::BUNTING_DARK.clone_into(&mut self.theme);
        }
    }

    /// The measurements this appearance produces.
    #[must_use]
    pub fn metrics(&self) -> Metrics {
        self.density.metrics(self.ui_font_size)
    }
}

/// The directory the desk's own files live in: the folder holding the
/// layout file, so `appearance.json` and `themes/` sit beside it.
#[must_use]
pub fn desk_config_dir() -> PathBuf {
    LayoutStore::default_location()
        .path()
        .parent()
        .map_or_else(|| PathBuf::from("."), std::borrow::ToOwned::to_owned)
}

/// Where user theme files go: `themes` inside [`desk_config_dir`].
#[must_use]
pub fn themes_dir() -> PathBuf {
    desk_config_dir().join("themes")
}

#[derive(Deserialize, Serialize)]
struct File {
    version: usize,
    theme: String,
    ui_font_size: f32,
    density: Density,
}

/// Just the version line, read before deciding what to do with a file.
#[derive(Deserialize)]
struct VersionProbe {
    version: Option<usize>,
}

/// Where the appearance is saved: one JSON file, replaced whole.
#[derive(Clone, Debug)]
pub struct AppearanceStore {
    path: PathBuf,
}

impl AppearanceStore {
    #[must_use]
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// `appearance.json` beside the layout file.
    #[must_use]
    pub fn default_location() -> Self {
        Self::at(desk_config_dir().join("appearance.json"))
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved appearance. A missing, unreadable or unknown-version
    /// file falls back to the defaults; the file itself is left alone
    /// until the next change is saved over it.
    #[must_use]
    pub fn load(&self) -> Appearance {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Appearance::default();
        };
        let version = serde_json::from_str::<VersionProbe>(&text)
            .ok()
            .and_then(|probe| probe.version);
        if version != Some(APPEARANCE_VERSION) {
            return Appearance::default();
        }
        serde_json::from_str::<File>(&text).map_or_else(
            |_| Appearance::default(),
            |file| {
                let mut appearance = Appearance {
                    theme: file.theme,
                    ui_font_size: file.ui_font_size,
                    density: file.density,
                };
                appearance.sanitize();
                appearance
            },
        )
    }

    /// Writes the appearance.
    ///
    /// # Errors
    /// Returns why the file could not be written.
    pub fn save(&self, appearance: &Appearance) -> std::io::Result<()> {
        let file = File {
            version: APPEARANCE_VERSION,
            theme: appearance.theme.clone(),
            ui_font_size: clamp_ui_font_size(appearance.ui_font_size),
            density: appearance.density,
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

/// The note left in a fresh user themes folder.
const USER_THEMES_README: &str = "\
Bunting desk themes
===================

Drop theme files in this folder and the desk picks them up while it
runs (no restart needed). A theme file is JSON in the gpui-kit theme
format: one file can hold several themes.

    {
      \"name\": \"My themes\",
      \"author\": \"Me\",
      \"themes\": [
        {
          \"name\": \"My Dark\",
          \"mode\": \"dark\",
          \"colors\": {
            \"background\": \"#0e1014\",
            \"foreground\": \"#e6e8eb\",
            \"chart.bullish\": \"#34c27a\",
            \"chart.bearish\": \"#f0575f\"
          }
        }
      ]
    }

Only the colours you name change; the rest fall back to the built-in
light or dark defaults for the theme's mode. The bundled
\"Bunting Dark\" and \"Bunting Light\" themes show every token the
desk reads (sidebar, title_bar, status_bar, tab_bar, table, list,
popover, input, success/danger/warning/info, and the market colours
chart.bullish for bid/buy/up and chart.bearish for ask/sell/down).

Pick a theme from the command palette (Appearance group). A file that
fails to parse is reported on the terminal and skipped; it never
blanks the app. Delete a file and its themes disappear on the next
reload.
";

/// Creates the user themes folder and, on first run, the note that
/// says how to add a theme. An existing note is left as the user
/// edited it.
///
/// # Errors
/// Returns why the folder or the note could not be written.
pub fn ensure_themes_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let readme = dir.join("README.txt");
    if !readme.exists() {
        std::fs::write(readme, USER_THEMES_README)?;
    }
    Ok(())
}

/// Every `*.json` theme file in `dir`, in path order.
fn theme_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("json")
        })
        .collect();
    paths.sort();
    paths
}

/// Parses every theme file in `dir`. A file that fails to read or
/// parse is logged and skipped; the good themes still load.
#[must_use]
pub fn load_user_themes(dir: &Path) -> Vec<ThemeConfig> {
    let mut themes = Vec::new();
    for path in theme_files(dir) {
        match std::fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| {
                serde_json::from_str::<ThemeSet>(&text).map_err(|error| error.to_string())
            }) {
            Ok(set) => themes.extend(set.themes),
            Err(error) => {
                eprintln!(
                    "bunting-desk: theme file skipped: {}: {error}",
                    path.display()
                );
            }
        }
    }
    themes
}

/// The live appearance and where it is saved. A GPUI global, so any
/// module can read the current settings through [`metrics`].
pub struct AppearanceSettings {
    appearance: Appearance,
    store: AppearanceStore,
}

impl Global for AppearanceSettings {}

impl AppearanceSettings {
    #[must_use]
    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    #[must_use]
    pub fn appearance(&self) -> &Appearance {
        &self.appearance
    }
}

/// The measurements for the appearance in force. Before [`install`]
/// runs (or in a context that never installed it) this returns the
/// defaults, so rendering never panics on a missing global.
#[must_use]
pub fn metrics(cx: &App) -> Metrics {
    if cx.has_global::<AppearanceSettings>() {
        AppearanceSettings::global(cx).appearance().metrics()
    } else {
        Appearance::default().metrics()
    }
}

fn appearance_or_default(cx: &App) -> Appearance {
    if cx.has_global::<AppearanceSettings>() {
        AppearanceSettings::global(cx).appearance().clone()
    } else {
        Appearance::default()
    }
}

/// Every theme the user can pick: the bundled pair first, then the
/// registry's themes (user files and the gpui-kit defaults) in its
/// sorted order, without repeating a bundled name.
#[must_use]
pub fn installed_themes(cx: &App) -> Vec<Rc<ThemeConfig>> {
    let mut themes: Vec<Rc<ThemeConfig>> =
        theme::bundled_themes().into_iter().map(Rc::new).collect();
    if cx.has_global::<ThemeRegistry>() {
        for config in ThemeRegistry::global(cx).sorted_themes() {
            if !themes.iter().any(|bundled| bundled.name == config.name) {
                themes.push(Rc::clone(config));
            }
        }
    }
    themes
}

fn find_theme(name: &str, cx: &App) -> Option<Rc<ThemeConfig>> {
    installed_themes(cx)
        .into_iter()
        .find(|config| config.name.as_ref() == name)
}

/// The name of the theme in force.
#[must_use]
pub fn current_theme_name(cx: &App) -> String {
    if cx.has_global::<AppearanceSettings>() {
        AppearanceSettings::global(cx).appearance().theme.clone()
    } else if cx.has_global::<Theme>() {
        Theme::global(cx).theme_name().to_string()
    } else {
        theme::BUNTING_DARK.to_owned()
    }
}

fn apply_config(config: &Rc<ThemeConfig>, cx: &mut App) {
    let ui_font_size = appearance_or_default(cx).ui_font_size;
    let config = Rc::clone(config);
    Theme::update(cx, |theme| {
        theme.apply_config(&config);
        // The desk's fonts are the desk's, whatever a theme file says;
        // the user's font size likewise outranks a theme's `font.size`.
        theme.font_family = theme::SANS.into();
        theme.font_size = px(ui_font_size);
    });
}

/// Makes the named theme current. Returns false, leaving the current
/// theme alone, when no installed theme has that name.
#[must_use = "check whether the theme was installed before recording it"]
pub fn apply_theme(name: &str, cx: &mut App) -> bool {
    let Some(config) = find_theme(name, cx) else {
        eprintln!("bunting-desk: theme not installed: {name}");
        return false;
    };
    apply_config(&config, cx);
    true
}

/// Changes the saved appearance and writes it out. A failed write is
/// logged; the change still applies for this run.
fn update_appearance(cx: &mut App, edit: impl FnOnce(&mut Appearance)) {
    if !cx.has_global::<AppearanceSettings>() {
        cx.set_global(AppearanceSettings {
            appearance: Appearance::default(),
            store: AppearanceStore::default_location(),
        });
    }
    let settings = cx.global_mut::<AppearanceSettings>();
    edit(&mut settings.appearance);
    settings.appearance.sanitize();
    let (appearance, store) = (settings.appearance.clone(), settings.store.clone());
    if let Err(error) = store.save(&appearance) {
        eprintln!(
            "bunting-desk: appearance not saved to {}: {error}",
            store.path().display()
        );
    }
}

fn select_theme(name: &str, cx: &mut App) {
    if apply_theme(name, cx) {
        update_appearance(cx, |appearance| {
            name.clone_into(&mut appearance.theme);
        });
    }
}

/// The theme after `current` in `names`, wrapping to the first. An
/// unknown current name restarts at the first.
fn next_theme_name(current: &str, names: &[String]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let index = names
        .iter()
        .position(|name| name == current)
        .map_or(0, |index| (index + 1) % names.len());
    names.get(index).cloned()
}

fn select_next_theme(cx: &mut App) {
    let names: Vec<String> = installed_themes(cx)
        .iter()
        .map(|config| config.name.to_string())
        .collect();
    if let Some(next) = next_theme_name(&current_theme_name(cx), &names) {
        select_theme(&next, cx);
    }
}

/// The light/dark counterpart of `current`: the same family with the
/// other suffix when that theme is installed, otherwise the other
/// bundled theme. `themes` pairs each name with whether it is dark.
fn counterpart_name(current: &str, themes: &[(&str, bool)]) -> String {
    for (suffix, other) in [(" Dark", " Light"), (" Light", " Dark")] {
        if let Some(prefix) = current.strip_suffix(suffix) {
            let candidate = format!("{prefix}{other}");
            if themes.iter().any(|(name, _)| *name == candidate) {
                return candidate;
            }
        }
    }
    let current_is_dark = themes
        .iter()
        .find(|(name, _)| *name == current)
        .is_none_or(|(_, dark)| *dark);
    if current_is_dark {
        theme::BUNTING_LIGHT.to_owned()
    } else {
        theme::BUNTING_DARK.to_owned()
    }
}

fn toggle_light_dark(cx: &mut App) {
    let themes = installed_themes(cx);
    let pairs: Vec<(&str, bool)> = themes
        .iter()
        .map(|config| (config.name.as_ref(), config.mode.is_dark()))
        .collect();
    let target = counterpart_name(&current_theme_name(cx), &pairs);
    select_theme(&target, cx);
}

fn set_ui_font_size(size: f32, cx: &mut App) {
    let size = clamp_ui_font_size(size);
    update_appearance(cx, |appearance| appearance.ui_font_size = size);
    Theme::update(cx, |theme| theme.font_size = px(size));
}

#[allow(clippy::float_arithmetic, reason = "one-point font size steps")]
fn step_ui_font_size(delta: f32, cx: &mut App) {
    set_ui_font_size(appearance_or_default(cx).ui_font_size + delta, cx);
}

fn cycle_density(cx: &mut App) {
    update_appearance(cx, |appearance| {
        appearance.density = appearance.density.next();
    });
    cx.refresh_windows();
}

/// Registers every user theme file with the theme registry now, so a
/// saved user theme can apply at start-up. The folder watcher set up
/// in [`install`] handles later edits; its reloads rebuild the
/// registry from the folder, which is also how a deleted file's
/// themes go away.
fn register_user_themes(dir: &Path, cx: &mut App) {
    if !cx.has_global::<ThemeRegistry>() {
        return;
    }
    for path in theme_files(dir) {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(&text) {
                    eprintln!(
                        "bunting-desk: theme file skipped: {}: {error}",
                        path.display()
                    );
                }
            }
            Err(error) => {
                eprintln!(
                    "bunting-desk: theme file skipped: {}: {error}",
                    path.display()
                );
            }
        }
    }
}

/// Re-applies the saved theme and font size. A registry reload (a user
/// editing a theme file) rebuilds the active theme from its file,
/// which would otherwise put the file's font settings back.
fn reapply(cx: &mut App) {
    let appearance = appearance_or_default(cx);
    if !apply_theme(&appearance.theme, cx) {
        // The saved theme is gone (its file was deleted): fall back to
        // the bundled dark theme and record that, rather than leaving
        // a name nothing can apply.
        select_theme(theme::BUNTING_DARK, cx);
    }
}

/// Loads the saved appearance and the user themes, applies them, and
/// starts watching the user themes folder. Call once, after
/// `gpui_kit::init` and [`theme::install`]. Nothing here is fatal: a
/// bad file or an unwritable folder is logged and the defaults stand.
pub fn install(cx: &mut App) {
    let store = AppearanceStore::default_location();
    let appearance = store.load();
    cx.set_global(AppearanceSettings { appearance, store });

    let dir = themes_dir();
    if let Err(error) = ensure_themes_dir(&dir) {
        eprintln!(
            "bunting-desk: user themes folder not ready: {}: {error}",
            dir.display()
        );
    }
    register_user_themes(&dir, cx);
    reapply(cx);

    if cx.has_global::<ThemeRegistry>() {
        // A registry reload clears non-default themes (the bundled
        // pair included), so put them back and re-apply on each load.
        if let Err(error) = ThemeRegistry::watch_dir(dir, cx, |cx| {
            theme::register_bundled(cx);
            reapply(cx);
        }) {
            eprintln!("bunting-desk: theme folder not watched: {error}");
        }
        // Later reloads go through the registry's own observer, which
        // re-applies the active theme file (font included). Registered
        // after it, this observer puts the desk's font back.
        cx.observe_global::<ThemeRegistry>(|cx| {
            if cx.has_global::<AppearanceSettings>() && cx.has_global::<Theme>() {
                let ui_font_size = AppearanceSettings::global(cx).appearance().ui_font_size;
                Theme::update(cx, |theme| {
                    theme.font_family = theme::SANS.into();
                    theme.font_size = px(ui_font_size);
                });
            }
        })
        .detach();
    }
}

/// Handles the appearance actions on the workspace root. The
/// coordinator chains this after `Workspace::with_actions`:
///
/// ```ignore
/// .map(|root| Self::with_actions(root, cx))
/// .map(crate::appearance::with_appearance_actions)
/// ```
#[must_use]
pub fn with_appearance_actions(root: Stateful<Div>) -> Stateful<Div> {
    root.on_action(|_: &actions::SelectNextTheme, _, cx| select_next_theme(cx))
        .on_action(|_: &actions::ToggleLightDark, _, cx| toggle_light_dark(cx))
        .on_action(|action: &actions::SelectTheme, _, cx| {
            select_theme(&action.name, cx);
        })
        .on_action(|_: &actions::IncreaseUiFontSize, _, cx| {
            step_ui_font_size(1., cx);
        })
        .on_action(|_: &actions::DecreaseUiFontSize, _, cx| {
            step_ui_font_size(-1., cx);
        })
        .on_action(|_: &actions::ResetUiFontSize, _, cx| {
            set_ui_font_size(DEFAULT_UI_FONT_SIZE, cx);
        })
        .on_action(|_: &actions::CycleDensity, _, cx| cycle_density(cx))
}

/// The palette's Appearance group: the appearance actions, then one
/// entry per installed theme with the current one checked. The
/// coordinator adds it after the existing groups in
/// `Workspace::palette` (`.group(crate::appearance::palette_group(cx))`);
/// it must stay after the Screen group, whose row indices the palette
/// counts on.
#[must_use]
pub fn palette_group(cx: &App) -> CommandGroup {
    let current = current_theme_name(cx);
    let mut group = CommandGroup::new()
        .label("Appearance")
        .item(
            CommandItem::new()
                .label("Next theme")
                .keywords(["theme", "appearance"])
                .action(Box::new(actions::SelectNextTheme)),
        )
        .item(
            CommandItem::new()
                .label("Toggle light / dark")
                .keywords(["theme", "appearance", "dark", "light"])
                .action(Box::new(actions::ToggleLightDark)),
        )
        .item(
            CommandItem::new()
                .label("Increase UI font size")
                .keywords(["font", "zoom", "appearance"])
                .action(Box::new(actions::IncreaseUiFontSize)),
        )
        .item(
            CommandItem::new()
                .label("Decrease UI font size")
                .keywords(["font", "zoom", "appearance"])
                .action(Box::new(actions::DecreaseUiFontSize)),
        )
        .item(
            CommandItem::new()
                .label("Reset UI font size")
                .keywords(["font", "zoom", "appearance"])
                .action(Box::new(actions::ResetUiFontSize)),
        )
        .item(
            CommandItem::new()
                .label("Cycle density")
                .keywords(["density", "compact", "comfortable", "appearance"])
                .action(Box::new(actions::CycleDensity)),
        );
    for config in installed_themes(cx) {
        let name = config.name.to_string();
        group = group.item(
            CommandItem::new()
                .label(format!("Theme: {name}"))
                .checked(name == current)
                .keywords(["theme", "appearance"])
                .action(Box::new(actions::SelectTheme { name })),
        );
    }
    group
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "clamping returns the exact bound constants"
    )]
    fn font_size_clamps_to_the_supported_range() {
        assert_eq!(clamp_ui_font_size(8.), MIN_UI_FONT_SIZE);
        assert_eq!(clamp_ui_font_size(20.), MAX_UI_FONT_SIZE);
        assert_eq!(clamp_ui_font_size(14.), 14.);
        assert_eq!(clamp_ui_font_size(f32::NAN), DEFAULT_UI_FONT_SIZE);
    }

    #[test]
    fn metrics_follow_the_density_table() {
        let compact = Density::Compact.metrics(13.);
        assert_eq!(
            (
                compact.control_height,
                compact.row_height,
                compact.title_strip_height,
                compact.sidebar_row_height,
                compact.font_size
            ),
            (px(22.), px(22.), px(24.), px(24.), px(13.))
        );
        let default = Density::Default.metrics(13.);
        assert_eq!(
            (
                default.control_height,
                default.row_height,
                default.title_strip_height,
                default.sidebar_row_height
            ),
            (px(26.), px(24.), px(28.), px(28.))
        );
        let comfortable = Density::Comfortable.metrics(99.);
        assert_eq!(
            (
                comfortable.control_height,
                comfortable.row_height,
                comfortable.title_strip_height,
                comfortable.sidebar_row_height,
                comfortable.font_size
            ),
            (px(30.), px(28.), px(32.), px(32.), px(MAX_UI_FONT_SIZE))
        );
    }

    #[test]
    fn density_cycles_through_all_three() {
        assert_eq!(Density::Compact.next(), Density::Default);
        assert_eq!(Density::Default.next(), Density::Comfortable);
        assert_eq!(Density::Comfortable.next(), Density::Compact);
        assert_eq!(
            Density::ALL.map(Density::name),
            ["compact", "default", "comfortable"]
        );
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "clamping returns the exact bound constants"
    )]
    fn sanitize_repairs_an_empty_theme_and_a_wild_font_size() {
        let mut appearance = Appearance {
            theme: "  ".to_owned(),
            ui_font_size: 42.,
            density: Density::Compact,
        };
        appearance.sanitize();
        assert_eq!(appearance.theme, theme::BUNTING_DARK);
        assert_eq!(appearance.ui_font_size, MAX_UI_FONT_SIZE);
    }

    #[test]
    fn next_theme_wraps_and_restarts_from_an_unknown_name() {
        let names = vec!["A".to_owned(), "B".to_owned(), "C".to_owned()];
        assert_eq!(next_theme_name("A", &names).as_deref(), Some("B"));
        assert_eq!(next_theme_name("C", &names).as_deref(), Some("A"));
        assert_eq!(next_theme_name("gone", &names).as_deref(), Some("A"));
        assert_eq!(next_theme_name("A", &[]), None);
    }

    #[test]
    fn counterpart_prefers_the_same_family_then_the_bundled_pair() {
        let themes = [
            ("Bunting Dark", true),
            ("Bunting Light", false),
            ("Nord Dark", true),
            ("Nord Light", false),
            ("Solarized", false),
        ];
        assert_eq!(counterpart_name("Nord Dark", &themes), "Nord Light");
        assert_eq!(counterpart_name("Nord Light", &themes), "Nord Dark");
        assert_eq!(counterpart_name("Solarized", &themes), "Bunting Dark");
        assert_eq!(counterpart_name("Unknown", &themes), "Bunting Light");
        // A family with only one side installed falls back too.
        let one_sided = [("Bunting Dark", true), ("Bunting Light", false)];
        assert_eq!(counterpart_name("Only Dark", &one_sided), "Bunting Light");
    }
}
