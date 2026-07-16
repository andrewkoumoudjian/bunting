#![forbid(unsafe_code)]

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
use clap::Args;
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Args)]
pub struct TuiOptions {
    /// Named profile from the terminal configuration.
    #[arg(long)]
    pub profile: Option<String>,
    /// Terminal profile/workspace configuration path.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Override the selected profile's HOST:PORT endpoint for this process.
    #[arg(long)]
    pub endpoint: Option<String>,
    /// Start the native embedded market test fixture. Production local mode connects to a server.
    #[arg(long)]
    pub fixture: bool,
    /// Built-in policies to run in the embedded market (repeat or comma-separate).
    #[arg(
        long = "agent",
        value_delimiter = ',',
        default_value = "static_liquidity_provider,zero_intelligence_noise,long_momentum"
    )]
    pub agents: Vec<String>,
    /// Wall-clock pacing for one deterministic logical wake cycle.
    #[arg(long, default_value_t = 500)]
    pub agent_tick_ms: u64,
}

#[cfg(not(target_arch = "wasm32"))]
/// Runs the terminal with the selected connection profile and optional fixture.
///
/// # Errors
///
/// Returns an error when configuration, fixture startup, transport, or terminal
/// execution fails.
pub async fn run(options: TuiOptions) -> Result<(), String> {
    let (config, config_path) = config::TerminalConfig::load(options.config)?;
    let profile_name = options
        .profile
        .unwrap_or_else(|| config.selected_profile.clone());
    let mut profile = config.profile(&profile_name)?;
    if let Some(endpoint) = options.endpoint {
        profile.endpoint = endpoint;
    }
    let server = if options.fixture {
        let scenario =
            local_market::LocalScenarioConfig::from_names(&options.agents, options.agent_tick_ms)
                .map_err(|error| error.to_string())?;
        Some(
            local_market::spawn(&profile.endpoint, scenario)
                .await
                .map_err(|error| error.to_string())?,
        )
    } else {
        None
    };
    let credential_override = options.fixture.then(|| "fixture-only".to_owned());
    let result = Box::pin(tui::run(
        profile_name,
        profile,
        credential_override,
        config,
        config_path,
    ))
    .await
    .map_err(|error| error.to_string());
    if let Some(server) = server {
        server.abort();
    }
    result
}

#[cfg(target_arch = "wasm32")]
pub fn wasm_target_marker() {}
