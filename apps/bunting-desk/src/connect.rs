//! Where the app connects: a hosted venue named by flags or environment
//! (the same ones as `bunting-trader` and `bunting tui`), or a private
//! local practice venue started on this machine.

use crate::desk::Venue;
use bunting_trading_state::Session;
use bunting_trading_state::bunting_client::ClientConfig;
use bunting_trading_state::local_venue::{self, LocalVenue, ServerProgram};
use clap::Parser;
use std::path::PathBuf;

/// Bunting desktop trading app.
#[derive(Clone, Debug, Default, Parser)]
#[command(name = "bunting-desk", version)]
pub struct Options {
    /// Start a private practice venue on this machine and connect to it.
    #[arg(long, conflicts_with = "server")]
    pub local: bool,
    /// Directory for the local venue's certificates, configuration and log.
    #[arg(long, requires = "local")]
    pub venue_dir: Option<PathBuf>,
    /// Program that runs the local venue (`bunting` or `bunting-server`);
    /// default: `bunting` next to this app, then on `PATH`.
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
    /// Resume the private stream after this sequence (from a previous session).
    #[arg(long)]
    pub resume_after: Option<u64>,
}

/// Connects as `options` say and returns the venue and its label.
///
/// # Errors
/// Returns why the venue cannot be reached or started.
pub fn open(options: &Options) -> Result<(Venue, String), String> {
    let (mut client, label, local) = if options.local {
        let venue = start_local(options)?;
        let client = venue.prepared.client.clone();
        let label = format!("local venue {}", client.address);
        (client, label, Some(venue))
    } else {
        let client = hosted(options)?;
        let label = client.address.clone();
        (client, label, None)
    };
    concat!("bunting-desk/", env!("CARGO_PKG_VERSION")).clone_into(&mut client.client_name);
    let session = Session::start(client).map_err(|error| error.to_string())?;
    Ok((Venue { session, local }, label))
}

fn hosted(options: &Options) -> Result<ClientConfig, String> {
    let missing = |name: &str| {
        format!(
            "--{name} (or BUNTING_{}) is required; or use --local to practise on this machine",
            name.to_uppercase()
        )
    };
    let address = options.server.clone().ok_or_else(|| missing("server"))?;
    let server_name = options
        .server_name
        .clone()
        .unwrap_or_else(|| host_of(&address));
    let ca = options.ca.clone().ok_or_else(|| missing("ca"))?;
    let cert = options.cert.clone().ok_or_else(|| missing("cert"))?;
    let key = options.key.clone().ok_or_else(|| missing("key"))?;
    let mut client = ClientConfig::from_files(address, server_name, &ca, &cert, &key)
        .map_err(|error| error.to_string())?;
    client.resume_after = options.resume_after;
    Ok(client)
}

fn start_local(options: &Options) -> Result<LocalVenue, String> {
    let program = options
        .server_bin
        .clone()
        .or_else(default_server_bin)
        .ok_or("--local needs --server-bin: the path of `bunting` or `bunting-server`")?;
    // `bunting` runs the venue as `bunting server`; `bunting-server` takes
    // the configuration directly.
    let arguments = if program
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("bunting"))
    {
        vec!["server".into()]
    } else {
        Vec::new()
    };
    let directory = options.venue_dir.clone().unwrap_or_else(default_venue_dir);
    let prepared = local_venue::prepare(&directory).map_err(|error| error.to_string())?;
    LocalVenue::start(prepared, &ServerProgram { program, arguments })
        .map_err(|error| error.to_string())
}

/// `bunting` installed next to this app, else the first on `PATH`.
fn default_server_bin() -> Option<PathBuf> {
    let name = format!("bunting{}", std::env::consts::EXE_SUFFIX);
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(&name)))
        .filter(|path| path.is_file());
    beside.or_else(|| {
        std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(&name))
            .find(|path| path.is_file())
    })
}

fn default_venue_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("bunting")
        .join("desk-venue")
}

fn host_of(address: &str) -> String {
    address
        .rsplit_once(':')
        .map_or(address, |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_certificate_name_defaults_to_the_host() {
        assert_eq!(host_of("venue.example:9443"), "venue.example");
        assert_eq!(host_of("[::1]:9443"), "::1");
    }

    #[test]
    fn a_hosted_venue_needs_its_address_and_certificates() {
        let error = hosted(&Options::default()).err();
        assert!(error.is_some_and(|error| error.starts_with("--server")));
    }
}
