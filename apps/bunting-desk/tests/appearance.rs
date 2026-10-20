//! Appearance (ADR 0051 decision 7, slice U2a): the bundled themes,
//! user theme files and the saved `appearance.json`, exercised
//! through the crate's public API without a window.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::float_cmp
)]

use bunting_desk::appearance::{self, Appearance, AppearanceStore, Density, clamp_ui_font_size};
use bunting_desk::theme;
use gpui_kit::component::theme::ThemeSet;
use std::path::PathBuf;

fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "bunting-desk-appearance-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos())
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A theme entry's colour keys, sorted for set comparison.
fn token_keys(entry: &serde_json::Value) -> Vec<&str> {
    let mut keys: Vec<&str> = entry["colors"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

#[test]
fn both_bundled_themes_parse_with_every_token_present() {
    let set: ThemeSet = serde_json::from_str(theme::BUNDLED_THEMES).expect("bundled themes parse");
    let names: Vec<&str> = set
        .themes
        .iter()
        .map(|config| config.name.as_ref())
        .collect();
    assert_eq!(names, [theme::BUNTING_DARK, theme::BUNTING_LIGHT]);
    assert!(set.themes[0].mode.is_dark());
    assert!(!set.themes[1].mode.is_dark());

    // Parse the file again as plain JSON: the typed parse fills any
    // missing token from the defaults, so only the raw keys prove
    // each bundled theme actually carries every token.
    let raw: serde_json::Value =
        serde_json::from_str(theme::BUNDLED_THEMES).expect("bundled themes are JSON");
    let themes = raw["themes"].as_array().expect("themes array");
    assert_eq!(themes.len(), 2);
    for entry in themes {
        let colors = entry["colors"].as_object().expect("colors object");
        for token in theme::REQUIRED_TOKENS {
            assert!(
                colors.contains_key(*token),
                "{} is missing {token}",
                entry["name"]
            );
        }
    }
    // The two themes carry the same token set, nothing one-sided.
    assert_eq!(token_keys(&themes[0]), token_keys(&themes[1]));
}

#[test]
fn a_bad_user_theme_is_skipped_and_the_good_one_loads() {
    let dir = temp_dir("themes");
    std::fs::write(dir.join("broken.json"), "{ not json").unwrap();
    std::fs::write(
        dir.join("wrong-shape.json"),
        r#"{"themes": [{"name": 42}]}"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("good.json"),
        r##"{
          "name": "Mine",
          "themes": [
            {"name": "Mine Dark", "mode": "dark",
             "colors": {"background": "#101418", "chart.bullish": "#34c27a"}},
            {"name": "Mine Light", "mode": "light",
             "colors": {"background": "#f4f5f7"}}
          ]
        }"##,
    )
    .unwrap();
    std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();

    let themes = appearance::load_user_themes(&dir);
    let names: Vec<&str> = themes.iter().map(|config| config.name.as_ref()).collect();
    assert_eq!(names, ["Mine Dark", "Mine Light"]);
    assert!(themes[0].mode.is_dark());
    assert!(!themes[1].mode.is_dark());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_themes_folder_is_created_with_how_to_add_a_theme() {
    let dir = temp_dir("ensure").join("themes");
    appearance::ensure_themes_dir(&dir).unwrap();
    let readme = std::fs::read_to_string(dir.join("README.txt")).unwrap();
    assert!(readme.contains("chart.bullish"), "readme:\n{readme}");
    assert!(readme.contains("JSON"), "readme:\n{readme}");
    // A second run leaves the user's edited note alone.
    std::fs::write(dir.join("README.txt"), "my own notes").unwrap();
    appearance::ensure_themes_dir(&dir).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("README.txt")).unwrap(),
        "my own notes"
    );
    std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

#[test]
fn appearance_round_trips_through_the_file() {
    let dir = temp_dir("round-trip");
    let store = AppearanceStore::at(dir.join("appearance.json"));
    let appearance = Appearance {
        theme: "Mine Light".to_owned(),
        ui_font_size: 15.,
        density: Density::Comfortable,
    };
    store.save(&appearance).unwrap();
    assert_eq!(store.load(), appearance);
    // The file is the versioned, human-editable shape the task names.
    let text = std::fs::read_to_string(store.path()).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        raw["version"].as_u64(),
        Some(appearance::APPEARANCE_VERSION as u64)
    );
    assert_eq!(raw["theme"], "Mine Light");
    assert_eq!(raw["ui_font_size"], 15.0);
    assert_eq!(raw["density"], "comfortable");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_unknown_version_falls_back_to_the_defaults() {
    let dir = temp_dir("unknown-version");
    let store = AppearanceStore::at(dir.join("appearance.json"));
    std::fs::write(
        store.path(),
        r#"{"version":99,"theme":"Future","ui_font_size":16.0,"density":"compact"}"#,
    )
    .unwrap();
    assert_eq!(store.load(), Appearance::default());
    // The unknown file is left in place, not deleted.
    assert!(store.path().exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_missing_or_broken_file_falls_back_to_the_defaults() {
    let dir = temp_dir("missing");
    let store = AppearanceStore::at(dir.join("appearance.json"));
    assert_eq!(store.load(), Appearance::default());
    std::fs::write(store.path(), "not json at all").unwrap();
    assert_eq!(store.load(), Appearance::default());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_saved_font_size_outside_the_range_is_clamped_on_load() {
    let dir = temp_dir("clamp");
    let store = AppearanceStore::at(dir.join("appearance.json"));
    std::fs::write(
        store.path(),
        r#"{"version":1,"theme":"Bunting Light","ui_font_size":30.0,"density":"compact"}"#,
    )
    .unwrap();
    let loaded = store.load();
    assert_eq!(loaded.ui_font_size, appearance::MAX_UI_FONT_SIZE);
    assert_eq!(loaded.density, Density::Compact);
    assert_eq!(loaded.theme, "Bunting Light");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn font_size_clamps_to_twelve_to_sixteen() {
    assert_eq!(clamp_ui_font_size(11.), 12.);
    assert_eq!(clamp_ui_font_size(12.), 12.);
    assert_eq!(clamp_ui_font_size(13.), 13.);
    assert_eq!(clamp_ui_font_size(16.), 16.);
    assert_eq!(clamp_ui_font_size(17.), 16.);
}

#[test]
fn config_dir_is_the_layout_files_folder() {
    // appearance.json and themes/ must sit beside workspaces.json:
    // the path comes from LayoutStore, not a second copy of its logic.
    let layouts = bunting_desk::LayoutStore::default_location();
    assert_eq!(
        appearance::desk_config_dir(),
        layouts.path().parent().unwrap()
    );
    assert_eq!(
        appearance::themes_dir(),
        layouts.path().parent().unwrap().join("themes")
    );
    assert_eq!(
        AppearanceStore::default_location().path(),
        layouts.path().parent().unwrap().join("appearance.json")
    );
}
