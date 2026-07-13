#![forbid(unsafe_code)]
// Rust guideline compliant 2026-02-21

#[cfg(not(target_arch = "wasm32"))]
mod local_market;
#[cfg(not(target_arch = "wasm32"))]
mod protocol;
#[cfg(not(target_arch = "wasm32"))]
mod tui;

#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Parser)]
#[command(about = "Local Bunting market terminal over FIX 4.4/TCP")]
struct Arguments {
    /// FIX acceptor address.
    #[arg(long, default_value = "127.0.0.1:9880")]
    address: String,
    /// Connect to an already-running acceptor instead of spawning the local market.
    #[arg(long)]
    remote: bool,
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    let server = if arguments.remote {
        None
    } else {
        Some(local_market::spawn(&arguments.address).await?)
    };
    let result = Box::pin(tui::run(&arguments.address)).await;
    if let Some(server) = server {
        server.abort();
    }
    result
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Native-only executable. Keeping a stub lets the workspace Wasm gate prove
    // that this app cannot pull terminal or socket dependencies into the Worker.
}
