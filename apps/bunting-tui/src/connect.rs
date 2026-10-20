//! Where the terminal connects: a hosted venue named by flags, environment
//! or a saved profile, or the shared local venue on this machine.

use bunting_trading_state::bunting_client::ClientConfig;
use bunting_trading_state::local_venue::{self, ServerProgram, SharedVenue};
use clap::Args;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Connection options; the same flags and environment as `bunting-trader`.
#[derive(Args, Clone, Debug, Default)]
pub struct TuiOptions {
    /// Join the shared practice venue on this machine (starting it
    /// first if needed) as the next free of its eight participants, so
    /// several terminals and desks trade in one local market.
    #[arg(long, conflicts_with_all = ["server", "profile"])]
    pub local: bool,
    /// Directory for the local venue's certificates, configuration and
    /// log (default: the per-user `bunting/local-venue` directory,
    /// shared by every local client).
    #[arg(long, requires = "local")]
    pub venue_dir: Option<PathBuf>,
    /// Claim exactly this local participant (1-8) instead of the next
    /// free one; fails if another client already holds it.
    #[arg(long, requires = "local")]
    pub participant: Option<u64>,
    /// Program that runs the local venue (`bunting` or `bunting-server`).
    #[arg(long, requires = "local")]
    pub server_bin: Option<PathBuf>,
    /// The venue's BNP listener, `host:port`.
    #[arg(long, env = "BUNTING_SERVER")]
    pub server: Option<String>,
    /// The name on the venue's certificate (default: the host of --server).
    #[arg(long, env = "BUNTING_SERVER_NAME")]
    pub server_name: Option<String>,
    /// PEM file of the CA that issued the venue's certificate.
    #[arg(long, env = "BUNTING_CA")]
    pub ca: Option<PathBuf>,
    /// PEM file of your participant certificate.
    #[arg(long, env = "BUNTING_CERT")]
    pub cert: Option<PathBuf>,
    /// PEM file of your participant private key.
    #[arg(long, env = "BUNTING_KEY")]
    pub key: Option<PathBuf>,
    /// A saved venue from the terminal configuration file.
    #[arg(long)]
    pub profile: Option<String>,
    /// Terminal configuration file (default: `$XDG_CONFIG_HOME/bunting/terminal.json`).
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Resume the private stream after this sequence (from a previous session).
    #[arg(long)]
    pub resume_after: Option<u64>,
}

/// Saved venues (`terminal.json`, version 2). Version 1 files described FIX
/// sessions and are refused.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalConfig {
    version: u16,
    default: Option<String>,
    profiles: BTreeMap<String, Profile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    server: String,
    server_name: Option<String>,
    ca: PathBuf,
    cert: PathBuf,
    key: PathBuf,
}

/// What the terminal connects to, and its claim on the shared local
/// venue (whose drop stops the server only for the last client).
pub struct Target {
    pub client: ClientConfig,
    pub label: String,
    pub local: Option<SharedVenue>,
}

pub fn resolve(options: &TuiOptions, server: Option<ServerProgram>) -> Result<Target, String> {
    if options.local {
        return local(options, server);
    }
    let profile = match (&options.server, &options.profile) {
        (Some(_), _) => None,
        (None, profile) => load_profile(options.config.as_deref(), profile.as_deref())?,
    };
    let missing = |name: &str| {
        format!(
            "--{name} (or BUNTING_{}) is required; or use --profile, or --local to practise on this machine",
            name.to_uppercase()
        )
    };
    let address = options
        .server
        .clone()
        .or_else(|| profile.as_ref().map(|p| p.server.clone()))
        .ok_or_else(|| missing("server"))?;
    let server_name = options
        .server_name
        .clone()
        .or_else(|| profile.as_ref().and_then(|p| p.server_name.clone()))
        .unwrap_or_else(|| host_of(&address));
    let pick = |flag: &Option<PathBuf>, saved: Option<&PathBuf>, name: &str| {
        flag.clone()
            .or_else(|| saved.cloned())
            .ok_or_else(|| missing(name))
    };
    let ca = pick(&options.ca, profile.as_ref().map(|p| &p.ca), "ca")?;
    let cert = pick(&options.cert, profile.as_ref().map(|p| &p.cert), "cert")?;
    let key = pick(&options.key, profile.as_ref().map(|p| &p.key), "key")?;
    let mut client = ClientConfig::from_files(address.clone(), server_name, &ca, &cert, &key)
        .map_err(|error| error.to_string())?;
    client.resume_after = options.resume_after;
    client_name(&mut client);
    Ok(Target {
        client,
        label: address,
        local: None,
    })
}

fn local(options: &TuiOptions, server: Option<ServerProgram>) -> Result<Target, String> {
    let program = options
        .server_bin
        .clone()
        .map(|program| {
            // `bunting` runs the venue as `bunting server`; `bunting-server`
            // takes the configuration directly.
            let arguments = if program
                .file_stem()
                .is_some_and(|stem| stem.eq_ignore_ascii_case("bunting"))
            {
                vec!["server".into()]
            } else {
                Vec::new()
            };
            ServerProgram { program, arguments }
        })
        .or(server)
        .ok_or("--local needs --server-bin: the path of `bunting` or `bunting-server`")?;
    let directory = options
        .venue_dir
        .clone()
        .unwrap_or_else(local_venue::default_venue_dir);
    let venue =
        local_venue::join_or_start(&directory, &program, options.participant.map(u128::from))
            .map_err(|error| error.to_string())?;
    eprintln!(
        "{} the local venue in {} as participant {}",
        if venue.started_server {
            "started"
        } else {
            "joined"
        },
        directory.display(),
        venue.participant_id
    );
    let mut client = venue.client.clone();
    client_name(&mut client);
    Ok(Target {
        label: venue.label(),
        client,
        local: Some(venue),
    })
}

fn client_name(client: &mut ClientConfig) {
    concat!("bunting-tui/", env!("CARGO_PKG_VERSION")).clone_into(&mut client.client_name);
}

fn host_of(address: &str) -> String {
    address
        .rsplit_once(':')
        .map_or(address, |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned()
}

fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bunting")
}

fn load_profile(path: Option<&Path>, name: Option<&str>) -> Result<Option<Profile>, String> {
    let path = path.map_or_else(|| config_dir().join("terminal.json"), Path::to_path_buf);
    if !path.exists() {
        return match name {
            Some(name) => Err(format!(
                "profile {name}: no terminal configuration at {}",
                path.display()
            )),
            None => Ok(None),
        };
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let config = parse_config(&text).map_err(|error| format!("{}: {error}", path.display()))?;
    let Some(name) = name.map(str::to_owned).or(config.default.clone()) else {
        return Ok(None);
    };
    let base = path.parent().unwrap_or_else(|| Path::new("."));
    let mut profile = config
        .profiles
        .get(&name)
        .cloned()
        .ok_or_else(|| format!("{}: no profile named {name}", path.display()))?;
    for file in [&mut profile.ca, &mut profile.cert, &mut profile.key] {
        if file.is_relative() {
            *file = base.join(&*file);
        }
    }
    Ok(Some(profile))
}

fn parse_config(text: &str) -> Result<TerminalConfig, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|error| format!("invalid JSON: {error}"))?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(2) {
        return Err(
            "unsupported terminal configuration: version 2 (BNP venues with certificates) is required; FIX profiles from version 1 are no longer read"
                .to_owned(),
        );
    }
    let config: TerminalConfig =
        serde_json::from_value(value).map_err(|error| format!("invalid configuration: {error}"))?;
    debug_assert_eq!(config.version, 2);
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_one_fix_profiles_are_refused_and_version_two_resolves() -> Result<(), String> {
        assert!(parse_config(r#"{"selected_profile":"local","profiles":{}}"#).is_err());
        let directory =
            std::env::temp_dir().join(format!("bunting-tui-config-{}", std::process::id()));
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let path = directory.join("terminal.json");
        std::fs::write(
            &path,
            r#"{"version":2,"default":"finals","profiles":{"finals":{"server":"venue.example.com:9443","ca":"ca.pem","cert":"team.pem","key":"team.key"}}}"#,
        )
        .map_err(|e| e.to_string())?;
        let profile = load_profile(Some(&path), None)?.ok_or("default profile")?;
        assert_eq!(profile.server, "venue.example.com:9443");
        assert_eq!(profile.ca, directory.join("ca.pem"));
        assert!(load_profile(Some(&path), Some("missing")).is_err());
        assert_eq!(host_of("[::1]:9443"), "::1");
        Ok(())
    }
}
