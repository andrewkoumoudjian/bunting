#![forbid(unsafe_code)]
// Rust guideline compliant 2026-02-21

#[cfg(not(target_arch = "wasm32"))]
mod chart;
#[cfg(not(target_arch = "wasm32"))]
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod io_task;
#[cfg(not(target_arch = "wasm32"))]
mod local_market;
#[cfg(not(target_arch = "wasm32"))]
mod protocol;
#[cfg(not(target_arch = "wasm32"))]
mod transport;
#[cfg(not(target_arch = "wasm32"))]
mod tui;

#[cfg(not(target_arch = "wasm32"))]
use clap::Parser;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Parser)]
#[command(about = "Bunting participant/operator terminal over FIX 4.4 TCP/TLS")]
struct Arguments {
    /// Named profile from the terminal configuration.
    #[arg(long)]
    profile: Option<String>,
    /// Terminal profile/workspace configuration path.
    #[arg(long)]
    config: Option<PathBuf>,
    /// Override the selected profile's HOST:PORT endpoint for this process.
    #[arg(long)]
    endpoint: Option<String>,
    /// Start the native embedded market test fixture. Production local mode connects to a server.
    #[arg(long)]
    fixture: bool,
    /// Built-in policies to run in the embedded market (repeat or comma-separate).
    #[arg(
        long = "agent",
        value_delimiter = ',',
        default_value = "static_liquidity_provider,zero_intelligence_noise,long_momentum"
    )]
    agents: Vec<String>,
    /// Wall-clock pacing for one deterministic logical wake cycle.
    #[arg(long, default_value_t = 500)]
    agent_tick_ms: u64,
}

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments = Arguments::parse();
    let (config, config_path) = config::TerminalConfig::load(arguments.config)?;
    let profile_name = arguments
        .profile
        .unwrap_or_else(|| config.selected_profile.clone());
    let mut profile = config.profile(&profile_name)?;
    if let Some(endpoint) = arguments.endpoint {
        profile.endpoint = endpoint;
    }
    let server = if arguments.fixture {
        let scenario = local_market::LocalScenarioConfig::from_names(
            &arguments.agents,
            arguments.agent_tick_ms,
        )?;
        Some(local_market::spawn(&profile.endpoint, scenario).await?)
    } else {
        None
    };
    let credential_override = arguments.fixture.then(|| "fixture-only".to_owned());
    let result = Box::pin(tui::run(
        profile_name,
        profile,
        credential_override,
        config,
        config_path,
    ))
    .await;
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
