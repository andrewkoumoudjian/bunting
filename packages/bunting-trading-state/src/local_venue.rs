//! A shared practice venue on this machine: the same `bunting server` a
//! host runs, started detached with a throwaway operator CA, and
//! connected to over BNP exactly as a hosted venue is. There is no
//! embedded engine.
//!
//! A market needs more than one participant, so the local venue is a
//! market by default: it provisions **eight** participant identities
//! (participants 1–8, each with its own client certificate), all funded
//! in the scenario, and every `--local` client on the machine joins the
//! one running venue as the next free participant (see
//! [`join_or_start`]). Two desks, or a desk and a terminal, started with
//! `--local` therefore trade with each other. The built-in agents
//! (participants 10, 11, 12) quote as before.
//!
//! The FIX roster lists all eight participants with distinct target comp
//! IDs (`TRADER1`…`TRADER8`) so a participant's own FIX program can sit
//! beside any desk; the apps themselves use BNP.
//!
//! Venue 1 sits with the trader and venue 2 is 2 ms away (ADR 0035
//! virtual distance, both ways), so the trader feels the distance when
//! choosing where to send an order. BNT is listed on venue 1, where the
//! built-in agents quote it; QRC is listed on both venues with no agents.
//! BNT is not listed on venue 2 yet because the built-in agent runtime
//! resolves an instrument to a single listing.
//!
//! Lifetime: the server runs detached from the client that started it.
//! Each client holds an advisory lock claiming its participant number;
//! the last client to leave stops the server. A server left behind by
//! crashed clients is reused by the next client, or restarted if its
//! configuration no longer matches what this build provisions.

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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SCENARIO: &str = include_str!("local/scenario.json");
/// How many participant identities the local venue provisions.
pub const PARTICIPANT_COUNT: usize = 8;
/// The first local participant ID.
pub const FIRST_PARTICIPANT: u128 = 1;
/// The last local participant ID.
pub const LAST_PARTICIPANT: u128 = 8;
/// The first participant's ID, kept for callers that named the single
/// local trader before the venue became a market of eight.
pub const TRADER: u128 = FIRST_PARTICIPANT;
/// Virtual distance from the trader to venue 2, microseconds one way.
pub const FAR_VENUE_US: u64 = 2_000;
/// BNP connections the local venue accepts: one per participant. The
/// server's configuration validation caps `max_connections` at the
/// roster size, so headroom beyond the eight is not expressible today;
/// a reconnect that races the old session's reaping is retried by the
/// client session instead.
const BNP_MAX_CONNECTIONS: usize = PARTICIPANT_COUNT;
/// FIX connections: one per participant.
const FIX_MAX_CONNECTIONS: usize = PARTICIPANT_COUNT;
/// How long a starting server is watched for an immediate exit.
const START_CHECK: Duration = Duration::from_millis(300);
/// How long stopping a server waits for a polite exit before forcing it.
const STOP_WAIT: Duration = Duration::from_secs(3);

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

/// Every participant ID the local venue provisions, in order.
#[must_use]
pub fn participant_ids() -> Vec<u128> {
    (FIRST_PARTICIPANT..=LAST_PARTICIPANT).collect()
}

/// One local participant's identity: its certificate fingerprint (the BNP
/// roster names participants by fingerprint) and how it connects.
#[derive(Clone)]
pub struct ParticipantIdentity {
    pub participant_id: u128,
    pub fingerprint: String,
    pub certificate_path: PathBuf,
    pub private_key_path: PathBuf,
    /// How this participant connects over BNP.
    pub client: ClientConfig,
}

/// A local venue's files, written and ready to start.
#[derive(Clone)]
pub struct PreparedVenue {
    pub directory: PathBuf,
    /// The server configuration to pass to `bunting server`.
    pub config_path: PathBuf,
    /// How the first participant connects; see [`PreparedVenue::client_for`]
    /// for the others.
    pub client: ClientConfig,
    /// Every provisioned participant, in participant order.
    pub participants: Vec<ParticipantIdentity>,
    pub bnp_port: u16,
    /// FIX port, for a participant's own FIX program next to the app.
    pub fix_port: u16,
}

impl PreparedVenue {
    /// How `participant_id` connects, if the venue provisions it.
    #[must_use]
    pub fn client_for(&self, participant_id: u128) -> Option<ClientConfig> {
        self.participants
            .iter()
            .find(|identity| identity.participant_id == participant_id)
            .map(|identity| identity.client.clone())
    }

    /// The identity of `participant_id`, if the venue provisions it.
    #[must_use]
    pub fn identity(&self, participant_id: u128) -> Option<&ParticipantIdentity> {
        self.participants
            .iter()
            .find(|identity| identity.participant_id == participant_id)
    }
}

/// How to start the server: a program and the arguments before the
/// configuration path, for example the running `bunting` executable and
/// `server`.
#[derive(Clone, Debug)]
pub struct ServerProgram {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
}

/// The well-known per-user directory of the shared local venue:
/// `$XDG_STATE_HOME/bunting/local-venue` (or `~/.local/state/...`) on
/// Unix, `%LOCALAPPDATA%\bunting\local-venue` on Windows. Never the
/// repository or the working directory.
#[must_use]
pub fn default_venue_dir() -> PathBuf {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from));
    #[cfg(not(windows))]
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
        });
    base.unwrap_or_else(std::env::temp_dir)
        .join("bunting")
        .join("local-venue")
}

fn participant_file(directory: &Path, participant_id: u128, extension: &str) -> PathBuf {
    directory.join(format!("participant-{participant_id}.{extension}"))
}

fn client_config(
    bnp_port: u16,
    ca_pem: &[u8],
    certificate_pem: &[u8],
    private_key_pem: &[u8],
) -> ClientConfig {
    ClientConfig {
        address: format!("127.0.0.1:{bnp_port}"),
        server_name: "localhost".to_owned(),
        ca_pem: ca_pem.to_vec(),
        certificate_pem: certificate_pem.to_vec(),
        private_key_pem: private_key_pem.to_vec(),
        resume_after: None,
        client_name: concat!("bunting-trading-state/", env!("CARGO_PKG_VERSION")).to_owned(),
        connect_timeout: Duration::from_secs(10),
    }
}

/// Writes a fresh CA, venue and eight participant certificates, the
/// scenario and the server configuration into `directory`. Keys are
/// generated for this provisioning only and never leave the directory.
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
    let mut issued: Vec<(u128, String, String, String)> = Vec::new();
    for participant_id in participant_ids() {
        let (certificate, key) = authority.issue(
            Vec::new(),
            &format!("local participant {participant_id}"),
            ExtendedKeyUsagePurpose::ClientAuth,
        )?;
        let fingerprint = certificate_fingerprint(certificate.as_bytes())
            .map_err(|e| error("cannot fingerprint a participant certificate")(&e))?;
        issued.push((participant_id, fingerprint, certificate, key));
    }
    let write = |name: &str, contents: &str| -> Result<PathBuf, LocalVenueError> {
        let path = directory.join(name);
        write_private(&path, contents).map_err(|e| error(name)(&e))?;
        Ok(path)
    };
    write("ca.pem", &authority.ca_pem)?;
    write("venue.pem", &server.0)?;
    write("venue.key", &server.1)?;
    for (participant_id, _, certificate, key) in &issued {
        write(&format!("participant-{participant_id}.pem"), certificate)?;
        write(&format!("participant-{participant_id}.key"), key)?;
    }
    write("scenario.json", SCENARIO)?;
    let bnp_port = free_port()?;
    let fix_port = free_port()?;
    let fingerprints: Vec<&str> = issued.iter().map(|(_, f, _, _)| f.as_str()).collect();
    let config = server_config(bnp_port, fix_port, &fingerprints);
    let config_path = write("server.json", &config.to_string())?;
    let participants: Vec<ParticipantIdentity> = issued
        .into_iter()
        .map(
            |(participant_id, fingerprint, certificate, key)| ParticipantIdentity {
                participant_id,
                fingerprint,
                certificate_path: participant_file(directory, participant_id, "pem"),
                private_key_path: participant_file(directory, participant_id, "key"),
                client: client_config(
                    bnp_port,
                    authority.ca_pem.as_bytes(),
                    certificate.as_bytes(),
                    key.as_bytes(),
                ),
            },
        )
        .collect();
    let client = participants
        .first()
        .map(|identity| identity.client.clone())
        .ok_or_else(|| LocalVenueError("the local venue has no participants".to_owned()))?;
    Ok(PreparedVenue {
        directory: directory.to_path_buf(),
        config_path,
        client,
        participants,
        bnp_port,
        fix_port,
    })
}

/// Reads a venue directory provisioned by [`prepare`] back into a
/// [`PreparedVenue`], without regenerating anything. Fails unless the
/// directory still matches what this build provisions: the scenario is
/// the bundled one, all eight participant certificates are on disk, and
/// the server configuration's BNP roster names exactly their
/// fingerprints with room for every participant.
///
/// # Errors
/// Returns an error when a file is missing or the configuration no
/// longer matches.
pub fn load_prepared(directory: &Path) -> Result<PreparedVenue, LocalVenueError> {
    let read = |name: &str| -> Result<String, LocalVenueError> {
        std::fs::read_to_string(directory.join(name)).map_err(|e| error(name)(&e))
    };
    if read("scenario.json")? != SCENARIO {
        return Err(LocalVenueError(
            "the local venue scenario no longer matches this build".to_owned(),
        ));
    }
    let config: Value = serde_json::from_str(&read("server.json")?)
        .map_err(|e| error("cannot parse server.json")(&e))?;
    let port_of = |section: &str| -> Result<u16, LocalVenueError> {
        let bind = config[section]["bind"]
            .as_str()
            .ok_or_else(|| LocalVenueError(format!("server.json has no {section} bind")))?;
        bind.rsplit_once(':')
            .and_then(|(_, port)| port.parse::<u16>().ok())
            .ok_or_else(|| LocalVenueError(format!("server.json has a bad {section} bind")))
    };
    let bnp_port = port_of("bnp")?;
    let fix_port = port_of("fix")?;
    let max_connections = config["bnp"]["max_connections"].as_u64().unwrap_or(0);
    if max_connections < u64::try_from(PARTICIPANT_COUNT).unwrap_or(u64::MAX) {
        return Err(LocalVenueError(
            "the local venue configuration has no room for every participant".to_owned(),
        ));
    }
    let roster: Vec<(String, u128)> = config["bnp"]["roster"]
        .as_array()
        .ok_or_else(|| LocalVenueError("server.json has no BNP roster".to_owned()))?
        .iter()
        .map(|entry| {
            Ok((
                entry["certificate_sha256"]
                    .as_str()
                    .ok_or_else(|| {
                        LocalVenueError("a BNP roster entry has no fingerprint".to_owned())
                    })?
                    .to_owned(),
                entry["participant_id"]
                    .as_u64()
                    .map(u128::from)
                    .ok_or_else(|| {
                        LocalVenueError("a BNP roster entry has no participant".to_owned())
                    })?,
            ))
        })
        .collect::<Result<_, LocalVenueError>>()?;
    let ca_pem = read("ca.pem")?;
    let mut participants = Vec::new();
    for participant_id in participant_ids() {
        let certificate = read(&format!("participant-{participant_id}.pem"))?;
        let key = read(&format!("participant-{participant_id}.key"))?;
        let fingerprint = certificate_fingerprint(certificate.as_bytes())
            .map_err(|e| error("cannot fingerprint a participant certificate")(&e))?;
        if !roster.contains(&(fingerprint.clone(), participant_id)) {
            return Err(LocalVenueError(format!(
                "participant {participant_id} is not in the local venue roster"
            )));
        }
        participants.push(ParticipantIdentity {
            participant_id,
            fingerprint,
            certificate_path: participant_file(directory, participant_id, "pem"),
            private_key_path: participant_file(directory, participant_id, "key"),
            client: client_config(
                bnp_port,
                ca_pem.as_bytes(),
                certificate.as_bytes(),
                key.as_bytes(),
            ),
        });
    }
    if roster.len() != PARTICIPANT_COUNT {
        return Err(LocalVenueError(
            "the local venue roster is not the eight local participants".to_owned(),
        ));
    }
    let client = participants
        .first()
        .map(|identity| identity.client.clone())
        .ok_or_else(|| LocalVenueError("the local venue has no participants".to_owned()))?;
    Ok(PreparedVenue {
        directory: directory.to_path_buf(),
        config_path: directory.join("server.json"),
        client,
        participants,
        bnp_port,
        fix_port,
    })
}

// ----- advisory file locks -------------------------------------------------
//
// The root workspace is pinned to Rust 1.88, where `File::try_lock` is not
// stable, so the lock is `rustix::fs::flock` on Unix and an exclusive
// share mode (`share_mode(0)`, the pattern `bunting-server`'s storage
// lease uses) on Windows. Both are released by the OS when the holder's
// descriptor closes — including on `kill -9` — which is what makes a
// claim safe to hold for a client's whole life.

/// An advisory exclusive lock held by an open file.
struct FileLock {
    _file: std::fs::File,
}

fn open_lock_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        // No sharing: a second open fails while the holder lives.
        options.share_mode(0);
    }
    options.open(path)
}

impl FileLock {
    /// Takes the lock if nobody holds it; `Ok(None)` when it is held.
    fn try_lock(path: &Path) -> Result<Option<Self>, LocalVenueError> {
        #[cfg(unix)]
        {
            let file = open_lock_file(path).map_err(|e| error("cannot open a lock file")(&e))?;
            match rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
                Ok(()) => Ok(Some(Self { _file: file })),
                Err(errno) if errno == rustix::io::Errno::WOULDBLOCK => Ok(None),
                Err(errno) => Err(error("cannot lock a lock file")(&errno)),
            }
        }
        #[cfg(windows)]
        {
            match open_lock_file(path) {
                Ok(file) => Ok(Some(Self { _file: file })),
                // A sharing violation means another client holds it; any
                // other failure to open is reported the same way the
                // server's storage lease reports it: unavailable.
                Err(_) => Ok(None),
            }
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = path;
            Err(LocalVenueError(
                "the shared local venue needs file locking this platform does not offer".to_owned(),
            ))
        }
    }

    /// Takes the lock, waiting for the current holder to finish.
    fn lock_blocking(path: &Path) -> Result<Self, LocalVenueError> {
        #[cfg(unix)]
        {
            let file = open_lock_file(path).map_err(|e| error("cannot open a lock file")(&e))?;
            rustix::fs::flock(&file, rustix::fs::FlockOperation::LockExclusive)
                .map_err(|e| error("cannot lock a lock file")(&e))?;
            Ok(Self { _file: file })
        }
        #[cfg(not(unix))]
        {
            // Windows has no blocking form of the share-mode lock; the
            // holder (check-and-start, or last-client shutdown) is brief
            // and the OS releases it if that process dies.
            let deadline = Instant::now() + Duration::from_secs(120);
            loop {
                if let Some(lock) = Self::try_lock(path)? {
                    return Ok(lock);
                }
                if Instant::now() > deadline {
                    return Err(LocalVenueError(format!(
                        "timed out waiting for the local venue lock {}",
                        path.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(25));
            }
        }
    }
}

// ----- participant claims --------------------------------------------------

fn claim_path(directory: &Path, participant_id: u128) -> PathBuf {
    directory
        .join("claims")
        .join(format!("participant-{participant_id}.lock"))
}

/// A claimed participant identity, held for the client's whole life.
/// Dropping it (or the process dying, even on `kill -9`) releases the
/// number for the next client.
pub struct ParticipantClaim {
    pub participant_id: u128,
    _lock: FileLock,
}

/// Claims `requested`, or the lowest free participant number when
/// `requested` is `None`.
///
/// # Errors
/// Returns an error when the requested number is outside 1–8 or already
/// claimed, or when every participant is claimed.
pub fn claim_participant(
    directory: &Path,
    requested: Option<u128>,
) -> Result<ParticipantClaim, LocalVenueError> {
    std::fs::create_dir_all(directory.join("claims"))
        .map_err(|e| error("cannot create the claims directory")(&e))?;
    if let Some(participant_id) = requested {
        if !(FIRST_PARTICIPANT..=LAST_PARTICIPANT).contains(&participant_id) {
            return Err(LocalVenueError(format!(
                "participant {participant_id} is outside the local venue's \
                 {FIRST_PARTICIPANT}..={LAST_PARTICIPANT}"
            )));
        }
        return match FileLock::try_lock(&claim_path(directory, participant_id))? {
            Some(lock) => Ok(ParticipantClaim {
                participant_id,
                _lock: lock,
            }),
            None => Err(LocalVenueError(format!(
                "participant {participant_id} is already claimed by another local client"
            ))),
        };
    }
    for participant_id in participant_ids() {
        if let Some(lock) = FileLock::try_lock(&claim_path(directory, participant_id))? {
            return Ok(ParticipantClaim {
                participant_id,
                _lock: lock,
            });
        }
    }
    Err(LocalVenueError(format!(
        "all {PARTICIPANT_COUNT} local participants are claimed by running clients; \
         close one or use --venue-dir for a second local market"
    )))
}

/// Whether any client currently holds a participant claim, found by
/// trying to lock every claim file.
fn any_claim_held(directory: &Path) -> bool {
    participant_ids().iter().any(|&participant_id| {
        // An error is treated as "held": never stop a venue on doubt.
        !matches!(
            FileLock::try_lock(&claim_path(directory, participant_id)),
            Ok(Some(_))
        )
    })
}

// ----- the venue record ------------------------------------------------------

/// What `venue.json` records about the running server.
struct VenueRecord {
    pid: u32,
    bnp_port: u16,
    fix_port: u16,
    nonce: String,
    /// The server program as started (from `venue.json`); its file name
    /// is checked against the live process off Linux.
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    program: String,
}

fn read_venue_record(directory: &Path) -> Option<VenueRecord> {
    let text = std::fs::read_to_string(directory.join("venue.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    Some(VenueRecord {
        pid: u32::try_from(value["pid"].as_u64()?).ok()?,
        bnp_port: u16::try_from(value["bnp_port"].as_u64()?).ok()?,
        fix_port: u16::try_from(value["fix_port"].as_u64()?).ok()?,
        nonce: value["nonce"].as_str()?.to_owned(),
        program: value["program"].as_str().unwrap_or_default().to_owned(),
    })
}

fn run_nonce() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash as _, Hasher as _};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let mut hasher = DefaultHasher::new();
    nanos.hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn write_venue_record(
    directory: &Path,
    pid: u32,
    prepared: &PreparedVenue,
    program: &ServerProgram,
) -> Result<VenueRecord, LocalVenueError> {
    let record = VenueRecord {
        pid,
        bnp_port: prepared.bnp_port,
        fix_port: prepared.fix_port,
        nonce: run_nonce(),
        program: program.program.to_string_lossy().into_owned(),
    };
    let value = json!({
        "pid": record.pid,
        "bnp_port": record.bnp_port,
        "fix_port": record.fix_port,
        "nonce": record.nonce,
        "program": program.program.to_string_lossy(),
        "config": prepared.config_path.to_string_lossy(),
    });
    write_private(&directory.join("venue.json"), &value.to_string())
        .map_err(|e| error("venue.json")(&e))?;
    Ok(record)
}

// ----- process helpers -------------------------------------------------------

fn pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(raw) = i32::try_from(pid) else {
            return false;
        };
        let Some(pid) = rustix::process::Pid::from_raw(raw) else {
            return false;
        };
        // `PERM` means the process exists but is not ours to signal.
        match rustix::process::test_kill_process(pid) {
            Ok(()) => true,
            Err(errno) => errno == rustix::io::Errno::PERM,
        }
    }
    #[cfg(windows)]
    {
        // No safe `std` liveness probe exists and this crate forbids
        // `unsafe`, so ask `tasklist`. The CSV row carries the PID in
        // quotes; "No tasks" output does not.
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .is_ok_and(|output| {
                String::from_utf8_lossy(&output.stdout).contains(&format!("\"{pid}\""))
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        false
    }
}

/// Whether the PID in the record is still the server this venue started:
/// alive and — where the command line can be read (Linux `/proc`) — it
/// names this venue's configuration. There is deliberately no connection
/// probe: a bare TCP connect to the BNP port fails the venue's TLS
/// handshake and is logged as an error. Readiness after a start is the
/// client's own BNP connect-and-retry. A process that fails any check is
/// never signalled.
fn process_is_our_server(record: &VenueRecord, directory: &Path) -> bool {
    if !pid_alive(record.pid) {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(cmdline) = std::fs::read(format!("/proc/{}/cmdline", record.pid)) {
            let text = String::from_utf8_lossy(&cmdline);
            let config = directory.join("server.json").to_string_lossy().into_owned();
            return text.contains(&config);
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = directory;
        // PIDs are recycled (Windows aggressively): never signal a process
        // whose image name cannot be read and matched to the server program.
        process_image_name(record.pid)
            .is_some_and(|actual| image_names_match(&record.program, &actual))
    }
    #[cfg(target_os = "linux")]
    {
        let _ = directory;
        false
    }
}

/// Whether the recorded server program and a live process's image name
/// denote the same executable: file names compare, ignoring directories,
/// a Windows `.exe` suffix and case (Windows and macOS file systems are
/// case-insensitive by default).
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn image_names_match(recorded: &str, actual: &str) -> bool {
    fn base(text: &str) -> String {
        let name = text.rsplit(['/', '\\']).next().unwrap_or(text).trim();
        let lower = name.to_lowercase();
        lower.strip_suffix(".exe").unwrap_or(&lower).to_owned()
    }
    let (recorded, actual) = (base(recorded), base(actual));
    !recorded.is_empty() && recorded == actual
}

/// The image name of a live process, where the platform can tell
/// (`ps` on macOS and other Unix, `tasklist` on Windows).
#[cfg(not(target_os = "linux"))]
fn process_image_name(pid: u32) -> Option<String> {
    #[cfg(windows)]
    {
        let output = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        // "image.exe","1234","Console","1","12,345 K"
        let line = text.lines().find(|line| line.starts_with('"'))?;
        Some(line.trim_start_matches('"').split('"').next()?.to_owned())
    }
    #[cfg(not(windows))]
    {
        let output = Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "comm="])
            .output()
            .ok()?;
        let name = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        (!name.is_empty()).then_some(name)
    }
}

/// Stops a server known to be ours: polite first, then forced.
fn stop_server(pid: u32) {
    #[cfg(unix)]
    {
        let Some(raw) = i32::try_from(pid)
            .ok()
            .and_then(rustix::process::Pid::from_raw)
        else {
            return;
        };
        let _ = rustix::process::kill_process(raw, rustix::process::Signal::TERM);
        let deadline = Instant::now() + STOP_WAIT;
        while pid_alive(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
        }
        if pid_alive(pid) {
            let _ = rustix::process::kill_process(raw, rustix::process::Signal::KILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
    }
}

/// Under the start lock: stops the venue's server when no client holds a
/// claim. Returns whether a server was stopped.
fn stop_if_last(directory: &Path) -> bool {
    let Ok(_start_lock) = FileLock::lock_blocking(&directory.join("venue.lock")) else {
        return false;
    };
    if any_claim_held(directory) {
        return false;
    }
    let Some(record) = read_venue_record(directory) else {
        return false;
    };
    if !process_is_our_server(&record, directory) {
        return false;
    }
    stop_server(record.pid);
    let _ = std::fs::remove_file(directory.join("venue.json"));
    true
}

/// Spawns the server detached from this client (its own process group on
/// Unix, a new process group with no window on Windows), so closing the
/// client that started it does not kill the market for the others.
fn start_detached(
    prepared: &PreparedVenue,
    program: &ServerProgram,
) -> Result<Child, LocalVenueError> {
    let log_path = prepared.directory.join("server.log");
    let log = std::fs::File::create(&log_path).map_err(|e| error("server.log")(&e))?;
    let log_err = log.try_clone().map_err(|e| error("server.log")(&e))?;
    let mut command = Command::new(&program.program);
    command
        .args(&program.arguments)
        .arg(&prepared.config_path)
        .current_dir(&prepared.directory)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        // CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW.
        command.creation_flags(0x0000_0200 | 0x0800_0000);
    }
    let mut child = command.spawn().map_err(|e| {
        error(&format!(
            "cannot start the venue server {}",
            program.program.display()
        ))(&e)
    })?;
    // A bad configuration stops the server at once.
    let deadline = Instant::now() + START_CHECK;
    while Instant::now() < deadline {
        if let Ok(Some(status)) = child.try_wait() {
            let log = std::fs::read_to_string(&log_path).unwrap_or_default();
            return Err(LocalVenueError(format!(
                "the venue server exited ({status}): {}",
                log.trim()
            )));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    Ok(child)
}

// ----- joining or starting the shared venue ----------------------------------

/// A client joined to the shared local venue. Holds the participant
/// claim for its whole life; dropping it releases the claim and, when it
/// was the last claim, stops the server. Drop the client's session
/// before this so it logs out while the venue is still up.
pub struct SharedVenue {
    pub prepared: PreparedVenue,
    /// The claimed participant identity.
    pub participant_id: u128,
    /// How the claimed participant connects.
    pub client: ClientConfig,
    /// Whether this client started the server (informational only: every
    /// client takes part in the last-leaver shutdown).
    pub started_server: bool,
    claim: Option<ParticipantClaim>,
    /// The starter keeps the child handle only to reap it; it is never
    /// killed through the handle, so the server survives this client
    /// whenever other clients remain.
    child: Option<Child>,
}

impl SharedVenue {
    /// The connection label the apps show: who you are, and where.
    #[must_use]
    pub fn label(&self) -> String {
        format!("Participant {} · local venue", self.participant_id)
    }
}

impl Drop for SharedVenue {
    fn drop(&mut self) {
        // Release our number first, then stop the market if we were its
        // last client.
        self.claim = None;
        let stopped = stop_if_last(&self.prepared.directory);
        if let Some(mut child) = self.child.take()
            && stopped
        {
            // Reap the server we started and have now stopped.
            let deadline = Instant::now() + STOP_WAIT;
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) | Err(_) => break,
                    Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                }
            }
        }
    }
}

/// Joins the shared local venue in `directory`, starting it first when
/// needed, and claims `requested` (or the lowest free participant).
///
/// Joining and starting are race-safe: an exclusive lock on `venue.lock`
/// covers the check-and-start and the claim, so two clients started at
/// the same moment start one server and claim different participants. A
/// running server whose configuration still matches is joined as it is
/// (including one left behind by crashed clients holding no claims); a
/// server whose configuration no longer matches is stopped — after
/// verifying it is still this venue's server — and replaced. A stopped
/// server is restarted on its existing files when they still match, so
/// clients holding claims can reconnect after a server crash.
///
/// # Errors
/// Returns an error when the venue cannot be provisioned or started, or
/// the claim cannot be taken (requested number taken or out of range,
/// or all eight claimed).
pub fn join_or_start(
    directory: &Path,
    program: &ServerProgram,
    requested: Option<u128>,
) -> Result<SharedVenue, LocalVenueError> {
    if let Some(participant_id) = requested
        && !(FIRST_PARTICIPANT..=LAST_PARTICIPANT).contains(&participant_id)
    {
        return Err(LocalVenueError(format!(
            "participant {participant_id} is outside the local venue's \
             {FIRST_PARTICIPANT}..={LAST_PARTICIPANT}"
        )));
    }
    std::fs::create_dir_all(directory).map_err(|e| error("cannot create venue directory")(&e))?;
    std::fs::create_dir_all(directory.join("claims"))
        .map_err(|e| error("cannot create the claims directory")(&e))?;
    let start_lock = FileLock::lock_blocking(&directory.join("venue.lock"))?;
    let record = read_venue_record(directory);
    let current = load_prepared(directory).ok();
    let running = record.as_ref().is_some_and(|record| {
        current.as_ref().is_some_and(|prepared| {
            prepared.bnp_port == record.bnp_port && prepared.fix_port == record.fix_port
        }) && process_is_our_server(record, directory)
    });
    let (prepared, child, started_server) = if running {
        let prepared = current.ok_or_else(|| {
            LocalVenueError("the running local venue has no readable configuration".to_owned())
        })?;
        (prepared, None, false)
    } else {
        // Stop a leftover server only when it is verifiably this venue's.
        if let Some(record) = &record
            && process_is_our_server(record, directory)
        {
            stop_server(record.pid);
        }
        let prepared = match current {
            Some(prepared) => prepared,
            None => prepare(directory)?,
        };
        let child = start_detached(&prepared, program)?;
        write_venue_record(directory, child.id(), &prepared, program)?;
        (prepared, Some(child), true)
    };
    // Claim while still holding the start lock, so a leaving client's
    // last-claim check (which takes the same lock) cannot stop the
    // server between our start and our claim.
    let claim = claim_participant(directory, requested)?;
    drop(start_lock);
    let client = prepared
        .client_for(claim.participant_id)
        .ok_or_else(|| LocalVenueError("the claimed participant has no certificate".to_owned()))?;
    Ok(SharedVenue {
        participant_id: claim.participant_id,
        prepared,
        client,
        started_server,
        claim: Some(claim),
        child,
    })
}

// ----- the single-client venue (kept for existing callers) -------------------

/// A running local venue owned by one client; dropping it stops the
/// server. New callers should use [`join_or_start`], which shares one
/// venue between every local client.
pub struct LocalVenue {
    child: Child,
    pub prepared: PreparedVenue,
}

impl LocalVenue {
    /// Starts `program` on the prepared configuration as a child of this
    /// process. Server output goes to `server.log` in the venue
    /// directory.
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

/// BNT's fundamental is a seeded random walk on the run clock (one run
/// second per second): 40% annualised volatility over a 60 s "trading day"
/// (a 15120 s year), about 8% (one sigma) over the first five minutes,
/// doubling to 80% from the fifth minute so the regime change shows on a
/// chart.
fn bnt_fundamental_path() -> Value {
    json!({
        "kind": "random_walk", "seed": 20_261_020_u64, "step_ns": 500_000_000_u64,
        "annual_volatility_bps": 4000, "trading_days_per_year": 252,
        "run_ns_per_day": 60_000_000_000_u64,
        "regimes": [{"from_ns": 300_000_000_000_u64, "annual_volatility_bps": 8000}],
        "min_price_ticks": 1, "max_price_ticks": 1_000_000
    })
}

fn server_config(bnp_port: u16, fix_port: u16, fingerprints: &[&str]) -> Value {
    let link = |a: &str, b: &str, latency_us: u64| json!({"between": [a, b], "latency_us": latency_us, "jitter_us": latency_us / 20});
    let agent = |kind: &str,
                 participant_id: u128,
                 seed: u64,
                 base_quantity: u64,
                 spread_ticks: u64,
                 wake_ms: u64| {
        json!({
            "kind": kind, "participant_id": participant_id, "base_quantity": base_quantity,
            "spread_ticks": spread_ticks, "inventory_target": 0, "wake_interval_ns": wake_ms * 1_000_000,
            "seed": seed, "max_intents_per_wake": 4
        })
    };
    // FIX for a participant's own program beside any desk; the apps use
    // BNP. Every participant is listed with its own target comp ID, and
    // its password is derived from its certificate fingerprint, so it is
    // stable for the life of one provisioning and known only on this
    // machine.
    let fix_roster: Vec<Value> = fingerprints
        .iter()
        .enumerate()
        .map(|(index, fingerprint)| {
            let participant_id = FIRST_PARTICIPANT + u128::try_from(index).unwrap_or(0);
            json!({
                "target_comp_id": format!("TRADER{participant_id}"),
                "username": format!("trader{participant_id}"),
                "password": &fingerprint[..24],
                "role": "participant",
                "participant_id": participant_id
            })
        })
        .collect();
    let bnp_roster: Vec<Value> = fingerprints
        .iter()
        .enumerate()
        .map(|(index, fingerprint)| {
            let participant_id = FIRST_PARTICIPANT + u128::try_from(index).unwrap_or(0);
            json!({"certificate_sha256": fingerprint, "participant_id": participant_id, "role": "participant"})
        })
        .collect();
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
            "roster": fix_roster,
            "heartbeat_seconds": 30, "max_connections": FIX_MAX_CONNECTIONS, "rate_limit_window_ms": 100,
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
            "roster": bnp_roster,
            "heartbeat_ms": 1000, "max_connections": BNP_MAX_CONNECTIONS, "max_frame_bytes": 65536,
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
                "fundamental_path": bnt_fundamental_path(),
                "agents": [
                    agent("fundamental_liquidity_provider", 10, 42, 40, 1, 500),
                    agent("fundamental_liquidity_provider", 13, 45, 40, 2, 500),
                    agent("fundamental_liquidity_provider", 14, 46, 40, 3, 500),
                    agent("fundamental_liquidity_provider", 15, 47, 40, 4, 500),
                    agent("fundamental_liquidity_provider", 16, 48, 40, 5, 500),
                    agent("zero_intelligence_noise", 11, 43, 10, 4, 500),
                    agent("mean_reversion", 12, 44, 5, 2, 500),
                    agent("fully_informed", 18, 50, 40, 2, 500)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    fn temp_venue_dir(test: &str) -> PathBuf {
        let unique = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "bunting-local-venue-{test}-{}-{unique}",
            std::process::id()
        ))
    }

    fn claim(
        directory: &Path,
        requested: Option<u128>,
    ) -> Result<ParticipantClaim, LocalVenueError> {
        claim_participant(directory, requested)
    }

    #[test]
    fn two_claims_get_different_participants() -> Result<(), LocalVenueError> {
        let directory = temp_venue_dir("two-claims");
        let first = claim(&directory, None)?;
        let second = claim(&directory, None)?;
        assert_eq!(first.participant_id, 1);
        assert_eq!(second.participant_id, 2);
        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn a_released_claim_is_reused() -> Result<(), LocalVenueError> {
        let directory = temp_venue_dir("released-claim");
        let first = claim(&directory, None)?;
        let second = claim(&directory, None)?;
        drop(first);
        let third = claim(&directory, None)?;
        assert_eq!(third.participant_id, 1, "the freed lowest number is reused");
        assert_eq!(second.participant_id, 2);
        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn a_requested_participant_is_claimed_exactly_and_a_taken_one_fails()
    -> Result<(), LocalVenueError> {
        let directory = temp_venue_dir("requested-claim");
        let third = claim(&directory, Some(3))?;
        assert_eq!(third.participant_id, 3);
        let taken = claim_participant(&directory, Some(3))
            .err()
            .map(|e| e.to_string())
            .ok_or_else(|| LocalVenueError("a taken claim succeeded".to_owned()))?;
        assert!(
            taken.contains("already claimed"),
            "clear taken message: {taken}"
        );
        let first = claim(&directory, None)?;
        assert_eq!(first.participant_id, 1, "auto-claim skips held numbers");
        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn claiming_beyond_the_roster_and_beyond_eight_clients_fails() -> Result<(), LocalVenueError> {
        let directory = temp_venue_dir("full-claims");
        assert!(claim_participant(&directory, Some(9)).is_err());
        assert!(claim_participant(&directory, Some(0)).is_err());
        let mut held = Vec::new();
        for _ in participant_ids() {
            held.push(claim(&directory, None)?);
        }
        let full = claim_participant(&directory, None)
            .err()
            .map(|e| e.to_string())
            .ok_or_else(|| LocalVenueError("a ninth claim succeeded".to_owned()))?;
        assert!(
            full.contains("all 8 local participants are claimed"),
            "clear full message: {full}"
        );
        assert!(any_claim_held(&directory));
        drop(held);
        assert!(!any_claim_held(&directory));
        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn the_roster_funds_eight_participants_and_names_their_certificates() -> Result<(), String> {
        let directory = temp_venue_dir("roster");
        let prepared = prepare(&directory).map_err(|e| e.to_string())?;
        assert_eq!(prepared.participants.len(), PARTICIPANT_COUNT);

        // Scenario: participants 1-8 enabled and funded alike; agents kept.
        let scenario: Value = serde_json::from_str(SCENARIO).map_err(|e| e.to_string())?;
        let participants = scenario["participants"]
            .as_object()
            .ok_or("scenario participants")?;
        for participant_id in participant_ids() {
            let entry = participants
                .get(&participant_id.to_string())
                .ok_or_else(|| format!("participant {participant_id} in the scenario"))?;
            assert_eq!(entry["enabled"], json!(true));
            assert_eq!(
                entry["initial_cash"]["1"], participants["1"]["initial_cash"]["1"],
                "participant {participant_id} is funded like participant 1"
            );
            assert_eq!(entry["limits"], participants["1"]["limits"]);
        }
        for agent in ["10", "11", "12"] {
            assert!(participants.contains_key(agent), "agent {agent} kept");
        }

        // Server config: the BNP roster is exactly the eight issued
        // certificates, each a different key pair, with connection room.
        let config: Value = serde_json::from_str(
            &std::fs::read_to_string(&prepared.config_path).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let roster = config["bnp"]["roster"].as_array().ok_or("BNP roster")?;
        assert_eq!(roster.len(), PARTICIPANT_COUNT);
        assert_eq!(
            config["bnp"]["max_connections"]
                .as_u64()
                .ok_or("max_connections")?,
            u64::try_from(PARTICIPANT_COUNT).map_err(|e| e.to_string())?,
            "BNP allows all eight participants at once (the server caps \
             max_connections at the roster size)"
        );
        let mut certificates = Vec::new();
        for identity in &prepared.participants {
            assert!(
                roster.iter().any(|entry| {
                    entry["certificate_sha256"] == json!(identity.fingerprint)
                        && entry["participant_id"] == json!(identity.participant_id)
                }),
                "participant {} is in the BNP roster by fingerprint",
                identity.participant_id
            );
            let on_disk = std::fs::read(&identity.certificate_path).map_err(|e| e.to_string())?;
            assert_eq!(
                certificate_fingerprint(&on_disk).map_err(|e| e.to_string())?,
                identity.fingerprint
            );
            certificates.push(identity.client.certificate_pem.clone());
        }
        certificates.sort();
        certificates.dedup();
        assert_eq!(
            certificates.len(),
            PARTICIPANT_COUNT,
            "eight distinct certificates"
        );

        // FIX lists all eight with distinct target comp IDs.
        let fix = config["fix"]["roster"].as_array().ok_or("FIX roster")?;
        assert_eq!(fix.len(), PARTICIPANT_COUNT);
        let mut comp_ids: Vec<&str> = fix
            .iter()
            .filter_map(|entry| entry["target_comp_id"].as_str())
            .collect();
        comp_ids.sort_unstable();
        comp_ids.dedup();
        assert_eq!(comp_ids.len(), PARTICIPANT_COUNT);

        // A provisioned directory loads back unchanged (the join path).
        let loaded = load_prepared(&directory).map_err(|e| e.to_string())?;
        assert_eq!(loaded.bnp_port, prepared.bnp_port);
        assert_eq!(loaded.fix_port, prepared.fix_port);
        for identity in &prepared.participants {
            let loaded_identity = loaded
                .identity(identity.participant_id)
                .ok_or("loaded identity")?;
            assert_eq!(loaded_identity.fingerprint, identity.fingerprint);
            assert_eq!(
                loaded_identity.client.certificate_pem,
                identity.client.certificate_pem
            );
        }
        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn a_venue_record_round_trips_and_a_dead_pid_is_not_our_server() -> Result<(), String> {
        let directory = temp_venue_dir("record");
        let prepared = prepare(&directory).map_err(|e| e.to_string())?;
        let program = ServerProgram {
            program: PathBuf::from("bunting"),
            arguments: vec!["server".into()],
        };
        // A PID that cannot be ours: this test process is alive but its
        // command line does not name the venue's server.json, so it must never be
        // treated as the server (and never signalled).
        let written = write_venue_record(&directory, std::process::id(), &prepared, &program)
            .map_err(|e| e.to_string())?;
        let read = read_venue_record(&directory).ok_or("venue record")?;
        assert_eq!(read.pid, std::process::id());
        assert_eq!(read.bnp_port, prepared.bnp_port);
        assert_eq!(read.nonce, written.nonce);
        // Alive, but not running this venue's configuration. Only where the
        // command line can be read (Linux) is that distinction available.
        #[cfg(target_os = "linux")]
        assert!(!process_is_our_server(&read, &directory));
        let _ = std::fs::remove_dir_all(&directory);
        Ok(())
    }

    #[test]
    fn every_built_in_agent_trades_as_a_funded_scenario_participant() -> Result<(), String> {
        let scenario: Value = serde_json::from_str(SCENARIO).map_err(|e| e.to_string())?;
        let participants = scenario["participants"]
            .as_object()
            .ok_or("scenario participants")?;
        let config = server_config(1, 2, &["0123456789abcdef0123456789abcdef"]);
        let agents = config["runtime"]["scheduler"]["agents"]
            .as_array()
            .ok_or("agents")?;
        // Liquidity at several price levels, so a 100-lot market order fills.
        let providers = agents
            .iter()
            .filter(|agent| agent["kind"] == json!("static_liquidity_provider"))
            .count();
        assert!(providers >= 5, "{providers} liquidity providers");
        let mut ids = Vec::new();
        for agent in agents {
            let id = agent["participant_id"].as_u64().ok_or("participant_id")?;
            assert!(
                participants.contains_key(&id.to_string()),
                "agent participant {id} is in the scenario"
            );
            assert!(
                u128::from(id) > LAST_PARTICIPANT,
                "agent {id} is not a desk seat"
            );
            ids.push(id);
        }
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), agents.len(), "one participant per agent");
        Ok(())
    }

    #[test]
    fn a_process_image_name_must_match_the_recorded_server_program() {
        assert!(image_names_match("/usr/local/bin/bunting", "bunting"));
        assert!(image_names_match("bunting", "/Applications/x/bunting"));
        assert!(image_names_match("C:\\bunting\\Bunting.exe", "bunting.exe"));
        assert!(image_names_match("bunting", "BUNTING.EXE"));
        assert!(!image_names_match("bunting", "chrome.exe"));
        assert!(!image_names_match("bunting", "bunting-desk"));
        assert!(!image_names_match("bunting", ""));
        assert!(!image_names_match("", ""));
    }

    #[test]
    fn the_default_venue_dir_is_per_user_never_the_working_directory() {
        let directory = default_venue_dir();
        assert!(directory.ends_with("local-venue"));
        assert!(
            directory.is_absolute(),
            "an absolute per-user path: {}",
            directory.display()
        );
    }
}
