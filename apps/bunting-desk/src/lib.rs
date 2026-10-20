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
pub mod canvas;
pub mod connect;
pub mod deals;
pub mod desk;
pub mod flash;
pub mod format;
pub mod ladder;
pub mod layouts;
pub mod news;
pub mod one_click;
pub mod panels;
pub mod panes;
pub mod theme;
pub mod ui;
pub mod workspace;

pub use appearance::{Appearance, AppearanceStore, Density, Metrics};
pub use canvas::Canvas;
pub use desk::{Desk, Outbox, Venue};
pub use layouts::{LayoutStore, Preset, Screen};
pub use panes::PaneKind;
pub use workspace::{PANELS, Workspace};

/// Sets the desk's look and keys. Call once, after `gpui_kit::init`.
pub fn init(cx: &mut gpui_kit::App) {
    theme::install(cx);
    appearance::install(cx);
    actions::bind_keys(cx);
}
