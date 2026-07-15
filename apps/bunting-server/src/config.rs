use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeploymentProfile {
    Local,
    HostedNative,
    Cloudflare,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageKind {
    Memory,
    File,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    pub kind: StorageKind,
    pub path: Option<String>,
    pub max_runs: usize,
    pub max_commands: usize,
    pub max_events_per_run: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "mode")]
pub enum TlsConfig {
    Disabled,
    Terminated {
        trusted_proxy: String,
        require_mutual_tls: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixConfig {
    pub bind: String,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub username: String,
    pub password: String,
    pub participant_id: u128,
    pub run_id: u128,
    pub heartbeat_seconds: u32,
    pub max_connections: usize,
    pub max_message_bytes: usize,
    pub max_journal_messages: usize,
    pub max_pending_inbound: usize,
    pub tls: TlsConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdminConfig {
    pub bind: String,
    pub bearer_token: String,
    pub max_request_bytes: usize,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    pub path: String,
    pub run_id: u128,
    pub iteration_id: u128,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RelayConfig {
    pub participant_bind: String,
    pub worker_bind: String,
    pub participant_sender_comp_id: String,
    pub worker_sender_comp_id: String,
    pub target_comp_id: String,
    pub participant_username: String,
    pub participant_password: String,
    pub journal_path: String,
    pub max_message_bytes: usize,
    pub max_journal_bytes: usize,
    pub max_pending_bytes: usize,
    pub tls: TlsConfig,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    pub version: u16,
    pub profile: DeploymentProfile,
    pub storage: StorageConfig,
    pub fix: Option<FixConfig>,
    pub admin: Option<AdminConfig>,
    pub scenario: Option<ScenarioConfig>,
    pub relay: Option<RelayConfig>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

impl ServerConfig {
    pub fn from_file(path: &Path) -> Result<Self, ConfigError> {
        let bytes = fs::read(path)
            .map_err(|error| ConfigError(format!("cannot read {}: {error}", path.display())))?;
        if bytes.len() > 65_536 {
            return Err(ConfigError("configuration exceeds 65536 bytes".to_owned()));
        }
        let mut config: Self = serde_json::from_slice(&bytes)
            .map_err(|error| ConfigError(format!("invalid configuration JSON: {error}")))?;
        config.resolve_relative_paths(path);
        config.validate()?;
        Ok(config)
    }

    fn resolve_relative_paths(&mut self, config_path: &Path) {
        let base = config_path.parent().unwrap_or_else(|| Path::new("."));
        if let Some(path) = self.storage.path.as_mut() {
            resolve_relative(path, base);
        }
        if let Some(scenario) = self.scenario.as_mut() {
            resolve_relative(&mut scenario.path, base);
        }
        if let Some(relay) = self.relay.as_mut() {
            resolve_relative(&mut relay.journal_path, base);
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError(format!(
                "unsupported configuration version {}; expected 1",
                self.version
            )));
        }
        if self.storage.max_runs == 0
            || self.storage.max_commands == 0
            || self.storage.max_events_per_run == 0
        {
            return Err(ConfigError(
                "storage bounds max_runs, max_commands and max_events_per_run must be positive"
                    .to_owned(),
            ));
        }
        match self.storage.kind {
            StorageKind::Memory if self.storage.path.is_some() => {
                return Err(ConfigError(
                    "memory storage must not configure a path".to_owned(),
                ));
            }
            StorageKind::File if self.storage.path.as_deref().is_none_or(str::is_empty) => {
                return Err(ConfigError(
                    "file storage requires a non-empty path".to_owned(),
                ));
            }
            StorageKind::Memory | StorageKind::File => {}
        }
        if self.profile == DeploymentProfile::Cloudflare {
            if self.fix.is_some() {
                return Err(ConfigError(
                    "Cloudflare profile cannot accept inbound FIX; configure relay only".to_owned(),
                ));
            }
            if self.relay.is_none() {
                return Err(ConfigError(
                    "Cloudflare profile requires an external relay configuration".to_owned(),
                ));
            }
        }
        if let Some(fix) = &self.fix {
            validate_fix(fix, self.profile)?;
        }
        if let Some(admin) = &self.admin {
            parse_socket("admin.bind", &admin.bind)?;
            if admin.bearer_token.len() < 16 || admin.bearer_token.len() > 256 {
                return Err(ConfigError(
                    "admin.bearer_token must contain 16..=256 bytes".to_owned(),
                ));
            }
            if !(1_024..=65_536).contains(&admin.max_request_bytes) {
                return Err(ConfigError(
                    "admin.max_request_bytes must be 1024..=65536".to_owned(),
                ));
            }
        }
        if let Some(relay) = &self.relay {
            validate_relay(relay)?;
        }
        Ok(())
    }
}

fn resolve_relative(value: &mut String, base: &Path) {
    let path = Path::new(value);
    if path.is_relative() {
        *value = base.join(path).to_string_lossy().into_owned();
    }
}

fn parse_socket(field: &str, value: &str) -> Result<SocketAddr, ConfigError> {
    value
        .parse()
        .map_err(|_| ConfigError(format!("{field} must be an IP socket address, got {value}")))
}

fn validate_tls(bind: SocketAddr, tls: &TlsConfig, field: &str) -> Result<(), ConfigError> {
    match tls {
        TlsConfig::Disabled if !bind.ip().is_loopback() => Err(ConfigError(format!(
            "{field} is non-loopback but TLS is disabled; bind loopback or configure mode=terminated"
        ))),
        TlsConfig::Terminated {
            trusted_proxy,
            require_mutual_tls,
        } => {
            let proxy: IpAddr = trusted_proxy.parse().map_err(|_| {
                ConfigError(format!("{field}.tls.trusted_proxy must be one IP address"))
            })?;
            if !*require_mutual_tls {
                return Err(ConfigError(format!(
                    "{field}.tls requires mutual TLS at the trusted terminator"
                )));
            }
            if !bind.ip().is_loopback() && proxy.is_unspecified() {
                return Err(ConfigError(format!(
                    "{field}.tls.trusted_proxy cannot be unspecified"
                )));
            }
            Ok(())
        }
        TlsConfig::Disabled => Ok(()),
    }
}

fn validate_fix(fix: &FixConfig, profile: DeploymentProfile) -> Result<(), ConfigError> {
    let bind = parse_socket("fix.bind", &fix.bind)?;
    if fix.sender_comp_id.is_empty()
        || fix.target_comp_id.is_empty()
        || fix.username.is_empty()
        || fix.password.len() < 12
    {
        return Err(ConfigError(
            "FIX CompIDs/username must be non-empty and password must contain at least 12 bytes"
                .to_owned(),
        ));
    }
    if fix.participant_id == 0 || fix.run_id == 0 {
        return Err(ConfigError(
            "fix participant_id and run_id must be non-zero".to_owned(),
        ));
    }
    if fix.max_connections != 1
        || !(256..=1_048_576).contains(&fix.max_message_bytes)
        || fix.max_journal_messages == 0
        || fix.max_pending_inbound == 0
        || fix.heartbeat_seconds == 0
    {
        return Err(ConfigError(
            "FIX bounds are invalid; the static identity/session binding requires max_connections=1, message bytes must be 256..=1048576, and heartbeat/journal/pending must be positive"
                .to_owned(),
        ));
    }
    if profile == DeploymentProfile::HostedNative {
        validate_tls(bind, &fix.tls, "fix")?;
    }
    Ok(())
}

fn validate_relay(relay: &RelayConfig) -> Result<(), ConfigError> {
    let participant = parse_socket("relay.participant_bind", &relay.participant_bind)?;
    let worker = parse_socket("relay.worker_bind", &relay.worker_bind)?;
    if participant == worker {
        return Err(ConfigError(
            "relay participant_bind and worker_bind must differ".to_owned(),
        ));
    }
    validate_tls(participant, &relay.tls, "relay")?;
    if relay.participant_sender_comp_id.is_empty()
        || relay.worker_sender_comp_id.is_empty()
        || relay.target_comp_id.is_empty()
        || relay.participant_username.is_empty()
        || relay.participant_password.len() < 12
    {
        return Err(ConfigError(
            "relay CompIDs/username must be non-empty and participant_password must contain at least 12 bytes"
                .to_owned(),
        ));
    }
    if !(256..=1_048_576).contains(&relay.max_message_bytes)
        || relay.max_journal_bytes < relay.max_message_bytes
        || relay.max_pending_bytes < relay.max_message_bytes
    {
        return Err(ConfigError(
            "relay message/journal/pending byte bounds are inconsistent".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_plaintext_non_loopback_is_actionable() {
        let fix = FixConfig {
            bind: "0.0.0.0:9876".to_owned(),
            sender_comp_id: "BUNTING".to_owned(),
            target_comp_id: "CLIENT".to_owned(),
            username: "client".to_owned(),
            password: "long-password".to_owned(),
            participant_id: 1,
            run_id: 1,
            heartbeat_seconds: 30,
            max_connections: 1,
            max_message_bytes: 16_384,
            max_journal_messages: 1_024,
            max_pending_inbound: 32,
            tls: TlsConfig::Disabled,
        };
        let Err(error) = validate_fix(&fix, DeploymentProfile::HostedNative) else {
            return;
        };
        assert!(error.0.contains("TLS is disabled"));
    }

    #[test]
    fn checked_in_profiles_parse_and_validate() -> Result<(), ConfigError> {
        for value in [
            include_str!("../config/local.json"),
            include_str!("../config/hosted-native.json"),
            include_str!("../config/cloudflare.json"),
        ] {
            let config: ServerConfig = serde_json::from_str(value)
                .map_err(|error| ConfigError(format!("profile JSON invalid: {error}")))?;
            config.validate()?;
        }
        Ok(())
    }

    #[test]
    fn local_profile_paths_are_relative_to_the_configuration() -> Result<(), ConfigError> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("config/local.json");
        let config = ServerConfig::from_file(&path)?;
        assert!(
            config
                .storage
                .path
                .as_deref()
                .is_some_and(|path| path.ends_with("config/bunting-local-state.json"))
        );
        assert!(
            config
                .scenario
                .as_ref()
                .is_some_and(|scenario| scenario.path.ends_with("config/scenario.json"))
        );
        Ok(())
    }
}
