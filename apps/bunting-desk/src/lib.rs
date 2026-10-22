#![forbid(unsafe_code)]
//! Bunting desktop trading app (ADR 0050).
//!
//! A participant client over the Bunting Native Protocol, rendered with GPUI
//! and Longbridge's components (`gpui-kit`). It renders the shared
//! `bunting-trading-state` model (ADR 0047), as the terminal does, and holds
//! no market authority: every book, order and fill it shows comes from the
//! venue's committed reports and feeds. Local practice starts a local
//! `bunting server` and connects to it like any hosted venue.

pub mod actions;
pub mod appearance;
pub mod blotter;
pub mod canvas;
pub mod chart;
pub mod connect;
pub mod deals;
pub mod desk;
pub mod flash;
pub mod format;
pub mod ladder;
pub mod layouts;
pub mod ledger;
pub mod news;
pub mod onboarding;
pub mod one_click;
pub mod panels;
pub mod panes;
pub mod portfolio;
pub mod sounds;
pub mod startup;
pub mod theme;
pub mod transactions;
pub mod ui;
pub mod workspace;

pub use appearance::{Appearance, AppearanceStore, Density, Metrics};
pub use canvas::Canvas;
pub use chart::ChartPanel;
pub use desk::{Desk, Outbox, Venue};
pub use layouts::{LayoutStore, Preset, Screen};
pub use panes::PaneKind;
pub use workspace::{PANELS, Workspace};

/// Sets the desk's look and keys. Call once, after `gpui_kit::init`.
pub fn init(cx: &mut gpui_kit::App) {
    theme::install(cx);
    appearance::install(cx);
    sounds::install(cx);
    actions::bind_keys(cx);
}

/// [`init`] with the appearance settings and user themes kept in
/// `config_dir`, for tests: they must not read or write the real ones.
pub fn init_in(config_dir: &std::path::Path, cx: &mut gpui_kit::App) {
    theme::install(cx);
    appearance::install_in(config_dir, cx);
    actions::bind_keys(cx);
}

/// Opens the trading workspace on a connected `venue` in its own window:
/// the desk model, its pump and the saved screens.
///
/// # Errors
/// Returns why the window could not be opened.
pub fn open_desk_window(venue: Venue, label: String, cx: &mut gpui_kit::App) -> Result<(), String> {
    use gpui_kit::{
        AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
    };
    let desk = cx.new(|cx| {
        Desk::start_pump(cx);
        Desk::connected(venue, label)
    });
    let bounds = Bounds::centered(None, size(px(1440.), px(900.)), cx);
    let window = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        // The Wayland app_id (window class) and title, so the
        // compositor can name and rule the window.
        app_id: Some("bunting-desk".to_owned()),
        titlebar: Some(TitlebarOptions {
            title: Some("Bunting".into()),
            ..TitlebarOptions::default()
        }),
        ..WindowOptions::default()
    };
    let store = LayoutStore::default_location();
    gpui_kit::open_window(window, cx, |window, cx| {
        cx.new(|cx| Workspace::new(desk.clone(), Some(store), window, cx))
    })
    .map(|_| ())
    .map_err(|error| format!("cannot open a window: {error}"))
}
