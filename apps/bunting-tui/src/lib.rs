#![forbid(unsafe_code)]
//! The Bunting trading terminal (ADR 0047): a Ratatui rendering of
//! `bunting-trading-state` over the Bunting Native Protocol.
//!
//! The terminal holds no market state of its own and never embeds an
//! engine. It connects to a venue with the participant's certificate, and
//! `--local` starts a private `bunting server` on this machine and connects
//! to it the same way.

#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod connect;
#[cfg(not(target_arch = "wasm32"))]
pub mod ui;
#[cfg(not(target_arch = "wasm32"))]
pub mod view;

#[cfg(not(target_arch = "wasm32"))]
pub use bunting_trading_state::local_venue::ServerProgram;
#[cfg(not(target_arch = "wasm32"))]
pub use connect::TuiOptions;

/// Runs the terminal until the trader quits. `server` is how `--local`
/// starts the venue (for the `bunting` executable: itself with `server`);
/// `--server-bin` overrides it.
///
/// # Errors
/// Returns an error when the connection settings are incomplete, the local
/// venue cannot start, or the terminal fails.
#[cfg(not(target_arch = "wasm32"))]
pub fn run(options: &TuiOptions, server: Option<ServerProgram>) -> Result<(), String> {
    let target = connect::resolve(options, server)?;
    app::run(target)
}
