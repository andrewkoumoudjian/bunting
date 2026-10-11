//! The desk's look: a dark theme, bundled fonts and the type helpers every
//! pane uses (ADR 0050 decision 6).
//!
//! Body text is SF Pro on macOS, through the system font, since Apple's
//! licence covers its own platforms only, and Inter elsewhere. Titles are
//! Source Serif 4 Display. Inter and Source Serif 4 are bundled under the
//! SIL Open Font License 1.1 (`assets/fonts/*-LICENSE*`, provenance in
//! `assets/fonts/README.md`). Numbers use tabular figures so columns line up.

use gpui_kit::component::theme::{Theme, ThemeMode, ThemeSet};
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

const THEME: &str = include_str!("../assets/theme.json");

const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4Display-Regular.ttf"),
    include_bytes!("../assets/fonts/SourceSerif4Display-Semibold.ttf"),
];

/// Loads the bundled fonts and makes the Bunting dark theme current. Call
/// once, after `gpui_kit::init`.
pub fn install(cx: &mut App) {
    let fonts = FONTS.iter().map(|font| Cow::Borrowed(*font)).collect();
    if let Err(error) = cx.text_system().add_fonts(fonts) {
        eprintln!("bunting-desk: bundled fonts not loaded: {error}");
    }
    match serde_json::from_str::<ThemeSet>(THEME) {
        Ok(set) => {
            if let Some(dark) = set.themes.into_iter().find(|theme| theme.mode.is_dark()) {
                Theme::global_mut(cx).dark_theme = Rc::new(dark);
            }
        }
        Err(error) => eprintln!("bunting-desk: theme not loaded: {error}"),
    }
    Theme::change(ThemeMode::Dark, None, cx);
    Theme::update(cx, |theme| {
        theme.font_family = SANS.into();
        theme.font_size = px(13.);
    });
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
    serif(text).text_size(px(15.))
}
