use serde_json::Value;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Seek as _, SeekFrom, Write as _},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const ADMIN_ENDPOINT: &str = "127.0.0.1:8080";
const PROBE_INTERVAL: Duration = Duration::from_millis(400);
const HEALTH_TIMEOUT: Duration = Duration::from_millis(250);
const HEALTH_RESPONSE_LIMIT: usize = 16 * 1024;
const LOG_TAIL_LIMIT: usize = 8 * 1024;
const WASMER_VERSION: &str = "7.2.1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalServerPhase {
    Discovery,
    Runtime,
    Probe,
    Process,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalServerDiagnostic {
    pub phase: LocalServerPhase,
    pub code: &'static str,
    pub detail: String,
    pub log_path: Option<PathBuf>,
    pub exit_status: Option<String>,
}

#[derive(Clone, Debug)]
pub struct LocalServerSnapshot {
    pub label: String,
    pub detail: String,
    pub ready: bool,
    pub owned: bool,
    pub diagnostic: Option<LocalServerDiagnostic>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthProbe {
    pub status: String,
    pub service: String,
    pub health_contract_version: u64,
    pub fix_competition_profile_version: String,
}

#[derive(Debug)]
enum HealthProbeFailure {
    Unavailable(String),
    Invalid(String),
}

impl HealthProbeFailure {
    fn detail(&self) -> &str {
        match self {
            Self::Unavailable(detail) | Self::Invalid(detail) => detail,
        }
    }
}

#[derive(Debug)]
enum LocalServerState {
    Stopped,
    Starting,
    Ready,
    External,
    Failed(String),
    Exited(String),
}

pub struct LocalServerController {
    child: Option<Child>,
    state: LocalServerState,
    last_probe: Instant,
    log_path: Option<PathBuf>,
    diagnostic: Option<LocalServerDiagnostic>,
}

struct LaunchPlan {
    wasmer: PathBuf,
    artifact: PathBuf,
    config: PathBuf,
    volumes: Vec<PathBuf>,
    log_path: PathBuf,
}

impl LocalServerController {
    pub fn new() -> Self {
        let (state, diagnostic) = match probe_local_health() {
            Ok(_) => (LocalServerState::External, None),
            Err(HealthProbeFailure::Unavailable(_)) => (LocalServerState::Stopped, None),
            Err(error) => {
                let detail = error.detail().to_owned();
                (
                    LocalServerState::Failed(detail.clone()),
                    Some(LocalServerDiagnostic {
                        phase: LocalServerPhase::Probe,
                        code: "HEALTH_IDENTITY_MISMATCH",
                        detail,
                        log_path: None,
                        exit_status: None,
                    }),
                )
            }
        };
        Self {
            child: None,
            state,
            last_probe: Instant::now() - PROBE_INTERVAL,
            log_path: None,
            diagnostic,
        }
    }

    pub fn snapshot(&self) -> LocalServerSnapshot {
        let (label, detail, ready) = match &self.state {
            LocalServerState::Stopped => (
                "STOPPED".to_owned(),
                format!("Local Bunting health endpoint is unavailable on {ADMIN_ENDPOINT}"),
                false,
            ),
            LocalServerState::Starting => (
                "STARTING".to_owned(),
                self.log_detail("Wasmer is starting the bundled server"),
                false,
            ),
            LocalServerState::Ready => (
                "READY".to_owned(),
                self.log_detail(&format!(
                    "App-managed Bunting server is healthy on {ADMIN_ENDPOINT}"
                )),
                true,
            ),
            LocalServerState::External => (
                "EXTERNAL".to_owned(),
                format!(
                    "A compatible Bunting server not owned by this app is healthy on {ADMIN_ENDPOINT}"
                ),
                true,
            ),
            LocalServerState::Failed(error) => ("ERROR".to_owned(), error.clone(), false),
            LocalServerState::Exited(status) => (
                "EXITED".to_owned(),
                self.diagnostic.as_ref().map_or_else(
                    || self.log_detail(&format!("Server process exited: {status}")),
                    |diagnostic| diagnostic.detail.clone(),
                ),
                false,
            ),
        };
        LocalServerSnapshot {
            label,
            detail,
            ready,
            owned: self.child.is_some(),
            diagnostic: self.diagnostic.clone(),
        }
    }

    pub fn start(&mut self) -> Result<String, String> {
        self.poll();

        if self.child.is_some() {
            return Ok(self.log_detail("The app-managed server is already starting or running"));
        }
        match probe_local_health() {
            Ok(_) => {
                self.state = LocalServerState::External;
                self.diagnostic = None;
                return Ok(format!(
                    "A compatible Bunting server is already healthy on {ADMIN_ENDPOINT}; Bunting will reconnect without taking ownership"
                ));
            }
            Err(HealthProbeFailure::Unavailable(_)) => {}
            Err(error) => {
                let message = error.detail().to_owned();
                self.state = LocalServerState::Failed(message.clone());
                self.diagnostic = Some(LocalServerDiagnostic {
                    phase: LocalServerPhase::Probe,
                    code: "HEALTH_IDENTITY_MISMATCH",
                    detail: message.clone(),
                    log_path: self.log_path.clone(),
                    exit_status: None,
                });
                return Err(message);
            }
        }

        let plan = LaunchPlan::resolve().map_err(|error| {
            let message = error.to_string();
            self.state = LocalServerState::Failed(message.clone());
            self.diagnostic = Some(LocalServerDiagnostic {
                phase: LocalServerPhase::Discovery,
                code: if message.contains("version mismatch") {
                    "WASMER_VERSION_MISMATCH"
                } else {
                    "LAUNCH_PLAN_INVALID"
                },
                detail: message.clone(),
                log_path: self.log_path.clone(),
                exit_status: None,
            });
            message
        })?;
        let stdout = open_log(&plan.log_path).map_err(|error| {
            let message = format!("Cannot open local server log {}: {error}", plan.log_path.display());
            self.state = LocalServerState::Failed(message.clone());
            self.diagnostic = Some(LocalServerDiagnostic {
                phase: LocalServerPhase::Process,
                code: "LOG_OPEN_FAILED",
                detail: message.clone(),
                log_path: Some(plan.log_path.clone()),
                exit_status: None,
            });
            message
        })?;
        let stderr = stdout.try_clone().map_err(|error| {
            let message = format!("Cannot duplicate local server log handle: {error}");
            self.state = LocalServerState::Failed(message.clone());
            self.diagnostic = Some(LocalServerDiagnostic {
                phase: LocalServerPhase::Process,
                code: "LOG_CLONE_FAILED",
                detail: message.clone(),
                log_path: Some(plan.log_path.clone()),
                exit_status: None,
            });
            message
        })?;

        let mut command = Command::new(&plan.wasmer);
        command.arg("run").arg(&plan.artifact).arg("--net");
        for volume in &plan.volumes {
            command
                .arg("--volume")
                .arg(format!("{}:{}", volume.display(), volume.display()));
        }
        let working_directory = plan
            .config
            .parent()
            .ok_or_else(|| "Local server config has no parent directory".to_owned())?;
        command
            .arg("--cwd")
            .arg(working_directory)
            .arg("--")
            .arg(&plan.config)
            .current_dir(working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));

        let child = command.spawn().map_err(|error| {
            let message = format!(
                "Could not start Wasmer {WASMER_VERSION} at {}: {error}",
                plan.wasmer.display()
            );
            self.state = LocalServerState::Failed(message.clone());
            self.diagnostic = Some(LocalServerDiagnostic {
                phase: LocalServerPhase::Process,
                code: "WASMER_SPAWN_FAILED",
                detail: message.clone(),
                log_path: Some(plan.log_path.clone()),
                exit_status: None,
            });
            message
        })?;

        self.log_path = Some(plan.log_path);
        self.child = Some(child);
        self.state = LocalServerState::Starting;
        self.diagnostic = None;
        self.last_probe = Instant::now() - PROBE_INTERVAL;
        Ok(self.log_detail("Starting the bundled Bunting WASM server"))
    }

    pub fn stop(&mut self) -> Result<String, String> {
        let Some(mut child) = self.child.take() else {
            return match &self.state {
                LocalServerState::External => Err(
                    "The listener is external; Bunting will not stop a process it does not own"
                        .to_owned(),
                ),
                _ => Ok("No app-managed local server is running".to_owned()),
            };
        };

        child
            .kill()
            .map_err(|error| format!("Could not stop the app-managed local server: {error}"))?;
        let _ = child.wait();
        self.state = LocalServerState::Stopped;
        self.diagnostic = None;
        Ok("Stopped the app-managed local WASM server".to_owned())
    }

    pub fn poll(&mut self) -> bool {
        let was_ready = self.is_ready();

        let child_status = self.child.as_mut().map(Child::try_wait);
        match child_status {
            Some(Ok(Some(status))) => {
                self.child = None;
                match probe_local_health() {
                    Ok(_) => {
                        self.state = LocalServerState::External;
                        self.diagnostic = None;
                    }
                    Err(_) => {
                        let status = status.to_string();
                        let root_error = self
                            .log_path
                            .as_deref()
                            .and_then(|path| tail_log(path, LOG_TAIL_LIMIT).ok())
                            .and_then(|tail| {
                                tail.lines()
                                    .rev()
                                    .find(|line| {
                                        line.trim_start().starts_with("bunting-server:")
                                    })
                                    .map(str::trim)
                                    .map(ToOwned::to_owned)
                            });
                        let detail = root_error
                            .unwrap_or_else(|| format!("Server process exited: {status}"));
                        self.state = LocalServerState::Exited(status.clone());
                        self.diagnostic = Some(LocalServerDiagnostic {
                            phase: LocalServerPhase::Process,
                            code: "SERVER_EXITED",
                            detail,
                            log_path: self.log_path.clone(),
                            exit_status: Some(status),
                        });
                    }
                }
            }
            Some(Ok(None)) | None => {}
            Some(Err(error)) => {
                self.child = None;
                let message = format!("Could not inspect server process: {error}");
                self.state = LocalServerState::Failed(message.clone());
                self.diagnostic = Some(LocalServerDiagnostic {
                    phase: LocalServerPhase::Process,
                    code: "PROCESS_INSPECTION_FAILED",
                    detail: message,
                    log_path: self.log_path.clone(),
                    exit_status: None,
                });
            }
        }

        if self.last_probe.elapsed() >= PROBE_INTERVAL {
            self.last_probe = Instant::now();
            match (self.child.is_some(), probe_local_health()) {
                (true, Ok(_)) => {
                    self.state = LocalServerState::Ready;
                    self.diagnostic = None;
                }
                (true, Err(HealthProbeFailure::Unavailable(_))) => {
                    if !matches!(&self.state, LocalServerState::Failed(_)) {
                        self.state = LocalServerState::Starting;
                    }
                }
                (true, Err(error)) => {
                    let message = error.detail().to_owned();
                    self.state = LocalServerState::Failed(message.clone());
                    self.diagnostic = Some(LocalServerDiagnostic {
                        phase: LocalServerPhase::Probe,
                        code: "HEALTH_IDENTITY_MISMATCH",
                        detail: message,
                        log_path: self.log_path.clone(),
                        exit_status: None,
                    });
                }
                (false, Ok(_)) => {
                    self.state = LocalServerState::External;
                    self.diagnostic = None;
                }
                (false, Err(HealthProbeFailure::Unavailable(_))) => {
                    if matches!(&self.state, LocalServerState::External | LocalServerState::Ready) {
                        self.state = LocalServerState::Stopped;
                        self.diagnostic = None;
                    }
                }
                (false, Err(error)) => {
                    let message = error.detail().to_owned();
                    self.state = LocalServerState::Failed(message.clone());
                    self.diagnostic = Some(LocalServerDiagnostic {
                        phase: LocalServerPhase::Probe,
                        code: "HEALTH_IDENTITY_MISMATCH",
                        detail: message,
                        log_path: self.log_path.clone(),
                        exit_status: None,
                    });
                }
            }
        }

        !was_ready && self.is_ready()
    }

    pub fn is_ready(&self) -> bool {
        matches!(
            &self.state,
            LocalServerState::Ready | LocalServerState::External
        )
    }

    pub fn is_owned(&self) -> bool {
        self.child.is_some()
    }

    fn log_detail(&self, prefix: &str) -> String {
        self.log_path.as_ref().map_or_else(
            || prefix.to_owned(),
            |path| format!("{prefix}; log: {}", path.display()),
        )
    }
}

impl Drop for LocalServerController {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl LaunchPlan {
    fn resolve() -> io::Result<Self> {
        let wasmer = resolve_wasmer()?;
        verify_wasmer(&wasmer).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let artifact = resolve_artifact()?;
        let config = resolve_config()?;
        let volumes = resolve_volume_dirs(&config)?;
        let log_path = application_server_dir()?.join("bunting-server.log");
        Ok(Self {
            wasmer,
            artifact,
            config,
            volumes,
            log_path,
        })
    }
}

fn probe_local_health() -> Result<HealthProbe, HealthProbeFailure> {
    let address = ADMIN_ENDPOINT
        .parse::<SocketAddr>()
        .map_err(|error| HealthProbeFailure::Invalid(format!("invalid admin endpoint: {error}")))?;
    probe_bunting_health_detailed(address, HEALTH_TIMEOUT)
}

pub fn probe_bunting_health(address: SocketAddr, timeout: Duration) -> Result<HealthProbe, String> {
    probe_bunting_health_detailed(address, timeout).map_err(|error| error.detail().to_owned())
}

fn probe_bunting_health_detailed(
    address: SocketAddr,
    timeout: Duration,
) -> Result<HealthProbe, HealthProbeFailure> {
    let mut stream = TcpStream::connect_timeout(&address, timeout).map_err(|error| {
        HealthProbeFailure::Unavailable(format!("Bunting health endpoint unavailable: {error}"))
    })?;
    stream
        .set_read_timeout(Some(timeout))
        .and_then(|()| stream.set_write_timeout(Some(timeout)))
        .map_err(|error| HealthProbeFailure::Invalid(format!("health timeout setup failed: {error}")))?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .map_err(|error| HealthProbeFailure::Invalid(format!("health request failed: {error}")))?;

    let mut response = Vec::with_capacity(1024);
    let mut bounded = stream.take((HEALTH_RESPONSE_LIMIT + 1) as u64);
    bounded
        .read_to_end(&mut response)
        .map_err(|error| HealthProbeFailure::Invalid(format!("health response failed: {error}")))?;
    if response.len() > HEALTH_RESPONSE_LIMIT {
        return Err(HealthProbeFailure::Invalid(
            "health response exceeds 16384 bytes".to_owned(),
        ));
    }
    let response = std::str::from_utf8(&response)
        .map_err(|_| HealthProbeFailure::Invalid("health response is not UTF-8".to_owned()))?;
    let (headers, body) = response.split_once("\r\n\r\n").ok_or_else(|| {
        HealthProbeFailure::Invalid("health response is missing HTTP headers".to_owned())
    })?;
    let status_line = headers.lines().next().unwrap_or_default();
    if !status_line.starts_with("HTTP/1.1 200 ") && !status_line.starts_with("HTTP/1.0 200 ") {
        return Err(HealthProbeFailure::Invalid(format!(
            "health endpoint returned {status_line}"
        )));
    }
    let document: Value = serde_json::from_str(body)
        .map_err(|error| HealthProbeFailure::Invalid(format!("invalid health JSON: {error}")))?;
    let probe = HealthProbe {
        status: required_health_string(&document, "status")?,
        service: required_health_string(&document, "service")?,
        health_contract_version: document
            .get("healthContractVersion")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                HealthProbeFailure::Invalid("healthContractVersion is missing".to_owned())
            })?,
        fix_competition_profile_version: required_health_string(
            &document,
            "fixCompetitionProfileVersion",
        )?,
    };
    if probe.status != "ok" {
        return Err(HealthProbeFailure::Invalid(format!(
            "health status mismatch: {}",
            probe.status
        )));
    }
    if probe.service != "bunting-server" {
        return Err(HealthProbeFailure::Invalid(format!(
            "health service mismatch: {}",
            probe.service
        )));
    }
    if probe.health_contract_version != 1 {
        return Err(HealthProbeFailure::Invalid(format!(
            "health contract version mismatch: {}",
            probe.health_contract_version
        )));
    }
    if probe.fix_competition_profile_version != bunting_tui::client::FIX_PROFILE_VERSION {
        return Err(HealthProbeFailure::Invalid(format!(
            "FIX competition profile mismatch: {}",
            probe.fix_competition_profile_version
        )));
    }
    Ok(probe)
}

fn required_health_string(
    document: &Value,
    field: &'static str,
) -> Result<String, HealthProbeFailure> {
    document
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| HealthProbeFailure::Invalid(format!("health {field} is missing")))
}

fn resolve_wasmer() -> io::Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("WASMER_BIN") {
        candidates.push(PathBuf::from(path));
    }
    if let Some(resources) = bundled_server_dir() {
        candidates.push(resources.join("bin/wasmer"));
    }
    if let Some(home) = env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".wasmer/bin/wasmer"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/wasmer"));
    candidates.push(PathBuf::from("/usr/local/bin/wasmer"));
    if let Some(path) = env::var_os("PATH") {
        candidates.extend(env::split_paths(&path).map(|directory| directory.join("wasmer")));
    }

    candidates.into_iter().find(|path| path.is_file()).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!(
                "Wasmer {WASMER_VERSION} was not found. Install Wasmer or set WASMER_BIN, then press Start Server again"
            ),
        )
    })
}

fn parse_wasmer_version(stdout: &str) -> Option<(u64, u64, u64)> {
    stdout.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches('v');
        let mut parts = token.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        (parts.next().is_none()).then_some((major, minor, patch))
    })
}

fn verify_wasmer(path: &Path) -> Result<(), String> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| format!("Cannot execute Wasmer at {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!(
            "Wasmer at {} returned {} for --version",
            path.display(),
            output.status
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let discovered = parse_wasmer_version(&stdout)
        .or_else(|| parse_wasmer_version(&stderr))
        .ok_or_else(|| format!("Could not parse Wasmer version from {}", path.display()))?;
    if discovered != (7, 2, 1) {
        return Err(format!(
            "Wasmer version mismatch at {}: found {}.{}.{}, required {WASMER_VERSION}",
            path.display(), discovered.0, discovered.1, discovered.2
        ));
    }
    Ok(())
}

fn resolve_artifact() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("BUNTING_SERVER_ARTIFACT") {
        return require_file(PathBuf::from(path), "BUNTING_SERVER_ARTIFACT");
    }
    if let Some(resources) = bundled_server_dir() {
        let path = resources.join("bunting-server.wasm");
        if path.is_file() {
            return Ok(path);
        }
    }
    require_file(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/wasm32-wasmer-wasi-dl/release/bunting-server.wasm"),
        "local WASM build",
    )
}

fn resolve_config() -> io::Result<PathBuf> {
    if let Some(path) = env::var_os("BUNTING_SERVER_CONFIG") {
        return require_file(PathBuf::from(path), "BUNTING_SERVER_CONFIG");
    }

    let template_dir = bundled_server_dir().unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../bunting-server/config")
    });
    let config_template = require_file(template_dir.join("local.json"), "local server config")?;
    let scenario_template =
        require_file(template_dir.join("scenario.json"), "local server scenario")?;
    let data_dir = application_server_dir()?;
    fs::create_dir_all(&data_dir)?;
    copy_if_missing(&config_template, &data_dir.join("local.json"))?;
    copy_if_missing(&scenario_template, &data_dir.join("scenario.json"))?;
    Ok(data_dir.join("local.json"))
}

fn application_server_dir() -> io::Result<PathBuf> {
    let home = env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "HOME is unavailable; cannot create local server state directory",
        )
    })?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support/Bunting Market Terminal/server"))
}

fn bundled_server_dir() -> Option<PathBuf> {
    let executable = env::current_exe().ok()?;
    let contents = executable.parent()?.parent()?;
    let resources = contents.join("Resources/server");
    resources.is_dir().then_some(resources)
}

fn resolve_volume_dirs(config: &Path) -> io::Result<Vec<PathBuf>> {
    let config = config.canonicalize()?;
    let base = config.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "Local server config has no parent directory",
        )
    })?;
    let document: Value = serde_json::from_slice(&fs::read(&config)?).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Invalid local server config JSON: {error}"),
        )
    })?;
    let mut volumes = vec![base.to_path_buf()];
    for pointer in ["/storage/path", "/scenario/path"] {
        let Some(raw_path) = document.pointer(pointer).and_then(Value::as_str) else {
            continue;
        };
        let resolved = if Path::new(raw_path).is_absolute() {
            PathBuf::from(raw_path)
        } else {
            base.join(raw_path)
        };
        let directory = if resolved.is_dir() {
            resolved
        } else {
            resolved
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| base.to_path_buf())
        };
        if !volumes.contains(&directory) {
            volumes.push(directory);
        }
    }
    Ok(volumes)
}

fn tail_log(path: &Path, max_bytes: usize) -> io::Result<String> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let start = length.saturating_sub(max_bytes as u64);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes)
        .chars()
        .map(|character| {
            if character == '\n' || character == '\t' || !character.is_control() {
                character
            } else {
                ' '
            }
        })
        .collect())
}

fn require_file(path: PathBuf, description: &str) -> io::Result<PathBuf> {
    if path.is_file() {
        Ok(path)
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{description} was not found at {}", path.display()),
        ))
    }
}

fn copy_if_missing(source: &Path, destination: &Path) -> io::Result<()> {
    if !destination.exists() {
        fs::copy(source, destination)?;
    }
    Ok(())
}

fn open_log(path: &Path) -> io::Result<File> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read as _, Write as _},
        net::TcpListener,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn spawn_http_once(body: String) -> io::Result<SocketAddr> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 512];
                let _ = stream.read(&mut request);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Ok(address)
    }

    #[test]
    fn health_probe_rejects_non_bunting_http_listener() -> Result<(), Box<dyn std::error::Error>> {
        let address = spawn_http_once(
            r#"{"status":"ok","service":"other","healthContractVersion":1,"fixCompetitionProfileVersion":"x"}"#.to_owned(),
        )?;
        let error = match probe_bunting_health(address, Duration::from_millis(250)) {
            Ok(_) => return Err("non-Bunting service was accepted".into()),
            Err(error) => error,
        };
        assert!(error.contains("service"));
        Ok(())
    }

    #[test]
    fn health_probe_accepts_exact_bunting_contract() -> Result<(), Box<dyn std::error::Error>> {
        let address = spawn_http_once(format!(
            r#"{{"status":"ok","service":"bunting-server","healthContractVersion":1,"fixCompetitionProfileVersion":"{}"}}"#,
            bunting_tui::client::FIX_PROFILE_VERSION
        ))?;
        let probe = probe_bunting_health(address, Duration::from_millis(250))?;
        assert_eq!(probe.service, "bunting-server");
        assert_eq!(probe.health_contract_version, 1);
        Ok(())
    }

    #[test]
    fn wasmer_version_requires_7_2_1() {
        assert_eq!(parse_wasmer_version("wasmer 7.2.1"), Some((7, 2, 1)));
        assert_ne!(parse_wasmer_version("wasmer 6.1.0"), Some((7, 2, 1)));
    }

    #[test]
    fn tail_log_reads_only_bounded_suffix() -> Result<(), Box<dyn std::error::Error>> {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = env::temp_dir().join(format!("bunting-log-tail-{unique}.log"));
        fs::write(&path, format!("{}bunting-server: root cause\n", "x".repeat(128)))?;
        let tail = tail_log(&path, 64)?;
        assert!(tail.len() <= 64);
        assert!(tail.contains("bunting-server: root cause"));
        fs::remove_file(path)?;
        Ok(())
    }

    #[test]
    fn volume_resolution_is_bounded_to_config_references() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = env::temp_dir().join(format!("bunting-server-helper-{unique}"));
        fs::create_dir_all(root.join("origin")).expect("origin");
        fs::create_dir_all(root.join("sessions")).expect("sessions");
        fs::write(
            root.join("local.json"),
            r#"{
                "storage": {"path": "origin/state.json"},
                "scenario": {"path": "scenario.json"},
                "sessions": {"directory": "sessions"}
            }"#,
        )
        .expect("config");

        let volumes = resolve_volume_dirs(&root.join("local.json")).expect("volumes");
        let canonical_root = root.canonicalize().expect("canonical root");
        assert_eq!(volumes[0], canonical_root);
        assert!(volumes.contains(&canonical_root.join("origin")));
        assert_eq!(volumes.len(), 2);

        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn copy_if_missing_preserves_operator_changes() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = env::temp_dir().join(format!("bunting-server-copy-{unique}"));
        fs::create_dir_all(&root).expect("root");
        let source = root.join("source.json");
        let destination = root.join("destination.json");
        fs::write(&source, "template").expect("source");
        fs::write(&destination, "operator").expect("destination");

        copy_if_missing(&source, &destination).expect("copy");
        assert_eq!(
            fs::read_to_string(&destination).expect("read"),
            "operator"
        );

        fs::remove_dir_all(root).expect("cleanup");
    }
}
