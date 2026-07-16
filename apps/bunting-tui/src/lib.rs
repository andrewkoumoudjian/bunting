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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HeadlessValidation {
    pub verified_role: String,
    pub committed_sequence: String,
    pub observed_projections: Vec<String>,
}

#[cfg(not(target_arch = "wasm32"))]
/// Validates a native server through the same TCP, FIX session and projection
/// reducer used by the interactive terminal.
///
/// # Errors
/// Returns an error when the connection, session, or required competition
/// projection cannot be established within the bounded validation window.
pub async fn validate_server(endpoint: &str, password: &str) -> Result<HeadlessValidation, String> {
    let mut profile = config::TerminalConfig::default().profile("local")?;
    endpoint.clone_into(&mut profile.endpoint);
    let mut client =
        protocol::FixClient::new("validation".to_owned(), profile, Some(password.to_owned()))
            .map_err(|error| error.to_string())?;
    client
        .reconnect()
        .await
        .map_err(|error| error.to_string())?;
    let mut requested = false;
    for _ in 0..200 {
        Box::pin(client.poll_once())
            .await
            .map_err(|error| error.to_string())?;
        if client.connection_state() == simfix_session::ConnectionState::Established && !requested {
            client
                .send(protocol::book_request(1))
                .await
                .map_err(|error| error.to_string())?;
            for request in protocol::competition_requests(2) {
                client
                    .send(request)
                    .await
                    .map_err(|error| error.to_string())?;
            }
            requested = true;
        }
        if client.verified_role.is_some()
            && client.observed_message_types.contains("W")
            && client.discovery.is_some()
            && client.authoritative_account.is_some()
            && client.risk.is_some()
        {
            return Ok(HeadlessValidation {
                verified_role: client.verified_role.unwrap_or_default(),
                committed_sequence: client.committed_sequence,
                observed_projections: vec![
                    "market_snapshot".to_owned(),
                    "discovery".to_owned(),
                    "account".to_owned(),
                    "risk".to_owned(),
                ],
            });
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    Err(format!("server validation timed out: {}", client.status))
}

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
    let credential_override = if options.fixture {
        Some("fixture-only".to_owned())
    } else if profile_name == "local" && std::env::var_os(&profile.password_env).is_none() {
        Some("bunting-local-dev".to_owned())
    } else {
        None
    };
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
