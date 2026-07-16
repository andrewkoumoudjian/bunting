#![forbid(unsafe_code)]
#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Parser)]
#[command(about = "Bunting participant/operator terminal over FIX 4.4 TCP/TLS")]
struct Arguments {
    #[command(flatten)]
    options: bunting_tui::TuiOptions,
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    bunting_tui::run(arguments.options)
        .await
        .map_err(Into::into)
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // Native-only executable. Keeping a stub lets the workspace Wasm gate prove
    // that this app cannot pull terminal or socket dependencies into the Worker.
}
