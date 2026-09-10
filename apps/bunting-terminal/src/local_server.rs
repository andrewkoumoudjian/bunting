use serde_json::Value;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io,
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const FIX_ENDPOINT: &str = "127.0.0.1:9880";
const PROBE_INTERVAL: Duration = Duration::from_millis(400);
const WASMER_VERSION: &str = "7.2.1";

#[derive(Clone, Debug)]
pub struct LocalServerSnapshot {
    pub label: String,
    pub detail: String,
    pub ready: bool,
    pub owned: bool,
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
        let state = if endpoint_is_ready() {
            LocalServerState::External
        } else {
            LocalServerState::Stopped
        };
        Self {
            child: None,
            state,
            last_probe: Instant::now() - PROBE_INTERVAL,
            log_path: None,
        }
    }

    pub fn snapshot(&self) -> LocalServerSnapshot {
        let (label, detail, ready) = match &self.state {
            LocalServerState::Stopped => (
                "STOPPED".to_owned(),
                format!("Local WASM server is not listening on {FIX_ENDPOINT}"),
                false,
            ),
            LocalServerState::Starting => (
                "STARTING".to_owned(),
                self.log_detail("Wasmer is starting the bundled server"),
                false,
            ),
            LocalServerState::Ready => (
                "READY".to_owned(),
                self.log_detail(&format!("App-managed server is listening on {FIX_ENDPOINT}")),
                true,
            ),
            LocalServerState::External => (
                "EXTERNAL".to_owned(),
                format!("A server not owned by this app is listening on {FIX_ENDPOINT}"),
                true,
            ),
            LocalServerState::Failed(error) => ("ERROR".to_owned(), error.clone(), false),
            LocalServerState::Exited(status) => (
                "EXITED".to_owned(),
                self.log_detail(&format!("Server process exited: {status}")),
                false,
            ),
        };
        LocalServerSnapshot {
            label,
            detail,
            ready,
            owned: self.child.is_some(),
        }
    }

    pub fn start(&mut self) -> Result<String, String> {
        self.poll();

        if self.child.is_some() {
            return Ok(self.log_detail("The app-managed server is already starting or running"));
        }
        if endpoint_is_ready() {
            self.state = LocalServerState::External;
            return Ok(format!(
                "A server is already listening on {FIX_ENDPOINT}; Bunting will reconnect without taking ownership"
            ));
        }

        let plan = LaunchPlan::resolve().map_err(|error| {
            let message = error.to_string();
            self.state = LocalServerState::Failed(message.clone());
            message
        })?;
        let stdout = open_log(&plan.log_path).map_err(|error| {
            let message = format!("Cannot open local server log {}: {error}", plan.log_path.display());
            self.state = LocalServerState::Failed(message.clone());
            message
        })?;
        let stderr = stdout.try_clone().map_err(|error| {
            let message = format!("Cannot duplicate local server log handle: {error}");
            self.state = LocalServerState::Failed(message.clone());
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
                "Could not start Wasmer {} at {}: {error}",
                WASMER_VERSION,
                plan.wasmer.display()
            );
            self.state = LocalServerState::Failed(message.clone());
            message
        })?;

        self.log_path = Some(plan.log_path);
        self.child = Some(child);
        self.state = LocalServerState::Starting;
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
        Ok("Stopped the app-managed local WASM server".to_owned())
    }

    pub fn poll(&mut self) -> bool {
        let was_ready = self.is_ready();

        let child_status = self.child.as_mut().map(Child::try_wait);
        match child_status {
            Some(Ok(Some(status))) => {
                self.child = None;
                self.state = if endpoint_is_ready() {
                    LocalServerState::External
                } else {
                    LocalServerState::Exited(status.to_string())
                };
            }
            Some(Ok(None)) | None => {}
            Some(Err(error)) => {
                self.child = None;
                self.state =
                    LocalServerState::Failed(format!("Could not inspect server process: {error}"));
            }
        }

        if self.last_probe.elapsed() >= PROBE_INTERVAL {
            self.last_probe = Instant::now();
            let ready = endpoint_is_ready();
            match (self.child.is_some(), ready) {
                (true, true) => self.state = LocalServerState::Ready,
                (true, false) => {
                    if !matches!(&self.state, LocalServerState::Failed(_)) {
                        self.state = LocalServerState::Starting;
                    }
                }
                (false, true) => self.state = LocalServerState::External,
                (false, false) => {
                    if matches!(&self.state, LocalServerState::External) {
                        self.state = LocalServerState::Stopped;
                    }
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

fn endpoint_is_ready() -> bool {
    let Ok(address) = FIX_ENDPOINT.parse::<SocketAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&address, Duration::from_millis(80)).is_ok()
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
    use std::time::{SystemTime, UNIX_EPOCH};

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
