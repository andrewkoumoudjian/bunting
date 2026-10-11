//! A private venue on this machine for practice: the same `bunting server`
//! a host runs, started as a child process with a throwaway operator CA,
//! and connected to over BNP exactly as a hosted venue is. There is no
//! embedded engine.
//!
//! Venue 1 sits with the trader and venue 2 is 2 ms away (ADR 0035
//! virtual distance, both ways), so the trader feels the distance when
//! choosing where to send an order. BNT is listed on venue 1, where the
//! built-in agents quote it; QRC is listed on both venues with no agents.
//! BNT is not listed on venue 2 yet because the built-in agent runtime
//! resolves an instrument to a single listing.

use bunting_client::{ClientConfig, certificate_fingerprint};
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const SCENARIO: &str = include_str!("local/scenario.json");
/// The trader's participant ID in the local scenario.
pub const TRADER: u128 = 1;
/// Virtual distance from the trader to venue 2, microseconds one way.
pub const FAR_VENUE_US: u64 = 2_000;
/// How long a starting server is watched for an immediate exit.
const START_CHECK: Duration = Duration::from_millis(300);

#[derive(Debug)]
pub struct LocalVenueError(pub String);

impl std::fmt::Display for LocalVenueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LocalVenueError {}

fn error(context: &str) -> impl Fn(&dyn std::fmt::Display) -> LocalVenueError + '_ {
    move |cause| LocalVenueError(format!("{context}: {cause}"))
}

/// A local venue's files, written and ready to start.
#[derive(Clone)]
pub struct PreparedVenue {
    pub directory: PathBuf,
    /// The server configuration to pass to `bunting server`.
    pub config_path: PathBuf,
    /// How the trader connects.
    pub client: ClientConfig,
    pub bnp_port: u16,
    /// FIX port, for a participant's own FIX program next to the app.
    pub fix_port: u16,
}

/// How to start the server: a program and the arguments before the
/// configuration path, for example the running `bunting` executable and
/// `server`.
#[derive(Clone, Debug)]
pub struct ServerProgram {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
}

/// Writes a fresh CA, venue and trader certificates, the scenario and the
/// server configuration into `directory`. Keys are generated for this
/// start only and never leave the directory.
///
/// # Errors
/// Returns an error when a file cannot be written or a port reserved.
pub fn prepare(directory: &Path) -> Result<PreparedVenue, LocalVenueError> {
    std::fs::create_dir_all(directory).map_err(|e| error("cannot create venue directory")(&e))?;
    let mut authority = Authority::new()?;
    let server = authority.issue(
        vec!["localhost".to_owned()],
        "Bunting local venue",
        ExtendedKeyUsagePurpose::ServerAuth,
    )?;
    let trader = authority.issue(
        Vec::new(),
        "local trader",
        ExtendedKeyUsagePurpose::ClientAuth,
    )?;
    let fingerprint = certificate_fingerprint(trader.0.as_bytes())
        .map_err(|e| error("cannot fingerprint the trader certificate")(&e))?;
    let write = |name: &str, contents: &str| -> Result<PathBuf, LocalVenueError> {
        let path = directory.join(name);
        write_private(&path, contents).map_err(|e| error(name)(&e))?;
        Ok(path)
    };
    write("ca.pem", &authority.ca_pem)?;
    write("venue.pem", &server.0)?;
    write("venue.key", &server.1)?;
    write("trader.pem", &trader.0)?;
    write("trader.key", &trader.1)?;
    write("scenario.json", SCENARIO)?;
    let bnp_port = free_port()?;
    let fix_port = free_port()?;
    let config = server_config(bnp_port, fix_port, &fingerprint);
    let config_path = write("server.json", &config.to_string())?;
    let client = ClientConfig {
        address: format!("127.0.0.1:{bnp_port}"),
        server_name: "localhost".to_owned(),
        ca_pem: authority.ca_pem.into_bytes(),
        certificate_pem: trader.0.into_bytes(),
        private_key_pem: trader.1.into_bytes(),
        resume_after: None,
        client_name: concat!("bunting-trading-state/", env!("CARGO_PKG_VERSION")).to_owned(),
        connect_timeout: Duration::from_secs(10),
    };
    Ok(PreparedVenue {
        directory: directory.to_path_buf(),
        config_path,
        client,
        bnp_port,
        fix_port,
    })
}

/// A running local venue; dropping it stops the server.
pub struct LocalVenue {
    child: Child,
    pub prepared: PreparedVenue,
}

impl LocalVenue {
    /// Starts `program` on the prepared configuration. The server runs in
    /// the terminal's process group, so closing the terminal stops it too. Server output goes to
    /// `server.log` in the venue directory.
    ///
    /// # Errors
    /// Returns an error when the server cannot be started or exits early.
    pub fn start(
        prepared: PreparedVenue,
        program: &ServerProgram,
    ) -> Result<Self, LocalVenueError> {
        let log_path = prepared.directory.join("server.log");
        let log = std::fs::File::create(&log_path).map_err(|e| error("server.log")(&e))?;
        let log_err = log.try_clone().map_err(|e| error("server.log")(&e))?;
        let child = Command::new(&program.program)
            .args(&program.arguments)
            .arg(&prepared.config_path)
            .current_dir(&prepared.directory)
            .stdin(Stdio::null())
            .stdout(log)
            .stderr(log_err)
            .spawn()
            .map_err(|e| {
                error(&format!(
                    "cannot start the venue server {}",
                    program.program.display()
                ))(&e)
            })?;
        let mut venue = Self { child, prepared };
        // A bad configuration stops the server at once. Nothing probes its
        // ports (the venue would log a probe as a failed session); the
        // session retries until the listener is up.
        let deadline = Instant::now() + START_CHECK;
        while Instant::now() < deadline {
            if let Ok(Some(status)) = venue.child.try_wait() {
                let log = std::fs::read_to_string(&log_path).unwrap_or_default();
                return Err(LocalVenueError(format!(
                    "the venue server exited ({status}): {}",
                    log.trim()
                )));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(venue)
    }
}

impl Drop for LocalVenue {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn server_config(bnp_port: u16, fix_port: u16, fingerprint: &str) -> Value {
    let link = |a: &str, b: &str, latency_us: u64| json!({"between": [a, b], "latency_us": latency_us, "jitter_us": latency_us / 20});
    let agent = |kind: &str, participant_id: u128, seed: u64| {
        json!({
            "kind": kind, "participant_id": participant_id, "base_quantity": 5,
            "spread_ticks": 2, "inventory_target": 0, "wake_interval_ns": 500_000_000_u64,
            "seed": seed, "max_intents_per_wake": 4
        })
    };
    json!({
        "version": 1,
        "profile": "local",
        "storage": {
            "kind": "memory", "path": null, "max_runs": 4,
            "max_commands_per_run": 1_000_000, "max_events_per_run": 8_000_000,
            "checkpoint_interval": 8192
        },
        "fix": {
            "bind": format!("127.0.0.1:{fix_port}"),
            "sender_comp_id": "BUNTING",
            "run_id": 1,
            "roster": [{
                "target_comp_id": "TRADER", "username": "trader",
                // FIX for a participant's own program; the app uses BNP.
                "password": &fingerprint[..24], "role": "participant", "participant_id": TRADER
            }],
            "heartbeat_seconds": 30, "max_connections": 1, "rate_limit_window_ms": 100,
            "max_messages_per_interval": 64, "max_message_bytes": 16384,
            "max_journal_messages": 4096, "max_pending_inbound": 64,
            "tls": {"mode": "disabled"},
            "admission": {
                "map": {
                    "default_location": "home",
                    "venues": [
                        {"venue_id": 1, "location": "home"},
                        {"venue_id": 2, "location": "away"}
                    ],
                    "local": {"latency_us": 50, "jitter_us": 5},
                    "links": [link("home", "away", FAR_VENUE_US)]
                },
                "probe_interval_ms": 1000, "max_admission_queue": 4096,
                "max_outbound_hold": 4096, "consolidated_processing_us": 500
            }
        },
        "bnp": {
            "bind": format!("127.0.0.1:{bnp_port}"),
            "run_id": 1,
            "certificate_chain": "venue.pem",
            "private_key": "venue.key",
            "client_ca": "ca.pem",
            "roster": [{"certificate_sha256": fingerprint, "participant_id": TRADER, "role": "participant"}],
            "heartbeat_ms": 1000, "max_connections": 1, "max_frame_bytes": 65536,
            "rate_limit_window_ms": 100, "max_messages_per_interval": 64,
            "handshake_timeout_ms": 5000
        },
        "admin": null,
        "scenario": {"path": "scenario.json", "run_id": 1, "iteration_id": 1},
        "runtime": {
            "wall_tick_ms": 100,
            "scheduler": {
                "run_id": 1, "instrument_id": 1, "fundamental_price": 100,
                "remaining_parent_quantity": 1000, "max_actions_per_tick": 256,
                "agents": [
                    agent("static_liquidity_provider", 10, 42),
                    agent("zero_intelligence_noise", 11, 43),
                    agent("long_momentum", 12, 44)
                ]
            }
        }
    })
}

fn free_port() -> Result<u16, LocalVenueError> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|e| error("cannot reserve a local port")(&e))
}

fn write_private(path: &Path, contents: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    std::io::Write::write_all(&mut options.open(path)?, contents.as_bytes())
}

struct Authority {
    issuer: Issuer<'static, KeyPair>,
    ca_pem: String,
}

impl Authority {
    fn new() -> Result<Self, LocalVenueError> {
        let failed = error("cannot create the local CA");
        let key = KeyPair::generate().map_err(|e| failed(&e))?;
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(|e| failed(&e))?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params
            .distinguished_name
            .push(DnType::CommonName, "Bunting local practice CA");
        let certificate = params.self_signed(&key).map_err(|e| failed(&e))?;
        Ok(Self {
            issuer: Issuer::new(params, key),
            ca_pem: certificate.pem(),
        })
    }

    /// A certificate and key, as PEM.
    fn issue(
        &mut self,
        names: Vec<String>,
        common_name: &str,
        usage: ExtendedKeyUsagePurpose,
    ) -> Result<(String, String), LocalVenueError> {
        let failed = error("cannot issue a local certificate");
        let key = KeyPair::generate().map_err(|e| failed(&e))?;
        let mut params = CertificateParams::new(names).map_err(|e| failed(&e))?;
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.extended_key_usages = vec![usage];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let certificate = params
            .signed_by(&key, &self.issuer)
            .map_err(|e| failed(&e))?;
        Ok((certificate.pem(), key.serialize_pem()))
    }
}
