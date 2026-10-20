//! The desk's look: the bundled Bunting Dark and Bunting Light themes,
//! bundled fonts and the type helpers every pane uses (ADR 0050
//! decision 6, ADR 0051 decision 7).
//!
//! Body text is SF Pro on macOS, through the system font, since Apple's
//! licence covers its own platforms only, and Inter elsewhere. Titles are
//! Source Serif 4 Display. Inter and Source Serif 4 are bundled under the
//! SIL Open Font License 1.1 (`assets/fonts/*-LICENSE*`, provenance in
//! `assets/fonts/README.md`). Numbers use tabular figures so columns line up.
//!
//! Which theme is current, at what font size and density, is the saved
//! appearance (`crate::appearance`); this module only makes the bundled
//! themes available and puts Bunting Dark up first.

use gpui_kit::component::theme::{Theme, ThemeConfig, ThemeMode, ThemeRegistry, ThemeSet};
use gpui_kit::{App, Div, FontFeatures, ParentElement as _, SharedString, Styled as _, div, px};
use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

/// The family titles are set in.
pub const SERIF: &str = "Source Serif 4 Display";

/// The family body text is set in.
#[cfg(target_os = "macos")]
pub const SANS: &str = ".SystemUIFont";
#[cfg(not(target_os = "macos"))]
pub const SANS: &str = "Inter";

/// The bundled dark theme's name, as written in the theme file and in
/// `appearance.json`.
pub const BUNTING_DARK: &str = "Bunting Dark";
/// The bundled light theme's name.
pub const BUNTING_LIGHT: &str = "Bunting Light";

/// The bundled themes (`assets/themes/bunting.json`): Bunting Dark and
/// Bunting Light in gpui-kit `ThemeSet` form. They name no font family
/// or size: the desk's fonts and the user's font size outrank a theme
/// file (see `crate::appearance`).
pub const BUNDLED_THEMES: &str = include_str!("../assets/themes/bunting.json");

/// Every colour token the desk reads, by its theme-file key. Both
/// bundled themes must carry all of them (including the market colours
/// `chart.bullish` for bid/buy/up and `chart.bearish` for
/// ask/sell/down); `tests/appearance.rs` holds them to it.
pub const REQUIRED_TOKENS: &[&str] = &[
    "background",
    "foreground",
    "border",
    "window.border",
    "muted.background",
    "muted.foreground",
    "accent.background",
    "accent.foreground",
    "accordion.background",
    "group_box.background",
    "group_box.foreground",
    "description_list_label.foreground",
    "caret",
    "ring",
    "selection.background",
    "drag_border",
    "drop_target.background",
    "primary.background",
    "primary.hover.background",
    "primary.active.background",
    "primary.foreground",
    "secondary.background",
    "secondary.hover.background",
    "secondary.active.background",
    "secondary.foreground",
    "input.border",
    "link.foreground",
    "link.hover.foreground",
    "link.active.foreground",
    "popover.background",
    "popover.foreground",
    "list.background",
    "list.even.background",
    "list.head.background",
    "list.hover.background",
    "list.active.background",
    "list.active.border",
    "table.background",
    "table.even.background",
    "table.head.background",
    "table.head.foreground",
    "table.hover.background",
    "table.active.background",
    "table.active.border",
    "table.row.border",
    "sidebar.background",
    "sidebar.foreground",
    "sidebar.border",
    "sidebar.accent.background",
    "sidebar.accent.foreground",
    "sidebar.primary.background",
    "sidebar.primary.foreground",
    "tab_bar.background",
    "tab_bar.segmented.background",
    "tab.background",
    "tab.foreground",
    "tab.active.background",
    "tab.active.foreground",
    "title_bar.background",
    "title_bar.border",
    "status_bar.background",
    "status_bar.border",
    "scrollbar.background",
    "scrollbar.thumb.background",
    "scrollbar.thumb.hover.background",
    "progress_bar.background",
    "slider.bar.background",
    "slider.thumb.background",
    "chart.1",
    "chart.2",
    "chart.3",
    "chart.4",
    "chart.5",
    "chart.bullish",
    "chart.bearish",
    "chart.grid",
    "base.blue",
    "base.blue.light",
    "base.cyan",
    "base.cyan.light",
    "base.green",
    "base.green.light",
    "base.magenta",
    "base.magenta.light",
    "base.red",
    "base.red.light",
    "base.yellow",
    "base.yellow.light",
    "success.background",
    "success.foreground",
    "danger.background",
    "danger.foreground",
    "warning.background",
    "warning.foreground",
    "info.background",
    "info.foreground",
    "switch.background",
    "skeleton.background",
    "overlay",
];

const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4Display-Regular.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4Display-Semibold.ttf"),
];

/// The bundled themes, parsed. Empty (and logged) if the bundled file
/// ever fails to parse, so a bad build degrades to the gpui-kit
/// defaults instead of panicking.
#[must_use]
pub fn bundled_themes() -> Vec<ThemeConfig> {
    match serde_json::from_str::<ThemeSet>(BUNDLED_THEMES) {
        Ok(set) => set.themes,
        Err(error) => {
            eprintln!("bunting-desk: bundled themes not loaded: {error}");
            Vec::new()
        }
    }
}

/// Puts the bundled themes in the theme registry. Registry reloads
/// (the user themes folder changing) rebuild the registry from the
/// defaults and the folder, dropping the bundled pair, so
/// `crate::appearance` calls this again after each load.
pub fn register_bundled(cx: &mut App) {
    if !cx.has_global::<ThemeRegistry>() {
        return;
    }
    if let Err(error) = ThemeRegistry::global_mut(cx).load_themes_from_str(BUNDLED_THEMES) {
        eprintln!("bunting-desk: bundled themes not registered: {error}");
    }
}

/// Loads the bundled fonts, registers the bundled themes and makes
/// Bunting Dark current at the default font size. Call once, after
/// `gpui_kit::init`; `crate::appearance::install` then applies the
/// saved theme, font size and density over this.
pub fn install(cx: &mut App) {
    let fonts = FONTS.iter().map(|font| Cow::Borrowed(*font)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("bunting-desk: bundled fonts not loaded: {error}");
    }
    register_bundled(cx);
    if cx.has_global::<Theme>() {
        let theme = Theme::global_mut(cx);
        for config in bundled_themes() {
            if config.mode.is_dark() {
                theme.dark_theme = Rc::new(config);
            } else {
                theme.light_theme = Rc::new(config);
            }
        }
    }
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = SANS.into();
        theme.font_size = px(13.);
    });
}

/// The palette's key hint on this platform.
#[must_use]
pub const fn command_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘K"
    } else {
        "Ctrl K"
    }
}

/// Tabular figures, so digits line up in columns.
#[must_use]
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".to_owned(), 1)]))
}

/// A title set in the serif.
#[must_use]
pub fn serif(text: impl Into<SharedString>) -> Div {
    div().font_family(SERIF).child(text.into())
}

/// A pane's title: its kind, and the symbol it shows when it follows one.
#[must_use]
pub fn pane_title(kind: &str, symbol: Option<&str>) -> Div {
    let text = symbol.map_or_else(|| kind.to_owned(), |symbol| format!("{kind} · {symbol}"));
    serif(text).text_size(crate::ui::rem(15.))
}
