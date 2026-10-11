#![forbid(unsafe_code)]
//! Bunting desktop trading app (ADR 0050).
//!
//! A participant client over the Bunting Native Protocol, rendered with GPUI
//! and Longbridge's components (`gpui-kit`). It renders the shared
//! `bunting-trading-state` model (ADR 0047), as the terminal does, and holds
//! no market authority: every book, order and fill it shows comes from the
//! venue's committed reports and feeds. Local practice starts a local
//! `bunting server` and connects to it like any hosted venue.

pub mod connect;
pub mod desk;
pub mod format;
pub mod layouts;
pub mod panels;
pub mod panes;
pub mod theme;
pub mod workspace;

pub use desk::{Desk, Outbox, Venue};
pub use layouts::{LayoutStore, Preset};
pub use panes::PaneKind;
pub use workspace::{PANELS, Workspace};
