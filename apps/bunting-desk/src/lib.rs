#![forbid(unsafe_code)]
//! Bunting desktop trading app (ADR 0050).
//!
//! A participant client over the Bunting Native Protocol, rendered with GPUI
//! and Longbridge's components (`gpui-kit`). It holds no market authority:
//! every book, order and fill it shows comes from the venue's committed
//! reports and feeds. Local play spawns a local `bunting server` and connects
//! to it like any hosted venue.

pub mod workspace;

pub use workspace::{Connection, PANELS, PanelSpec, Workspace};
