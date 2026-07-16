//! Native terminal profiles and workspace persistence.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, env, fs, io, path::PathBuf};

pub const FIX_PROFILE_VERSION: &str = "bunting.fixlatest.competition.v1";

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorRole {
    #[default]
    Participant,
    Team,
    Instructor,
    Administrator,
}

impl ActorRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Participant => "participant",
            Self::Team => "team",
            Self::Instructor => "instructor",
            Self::Administrator => "administrator",
        }
    }

    pub const fn privileged(self) -> bool {
        matches!(self, Self::Instructor | Self::Administrator)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TransportConfig {
    Tcp,
    Tls {
        server_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ca_file: Option<PathBuf>,
    },
}

impl TransportConfig {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Tcp => "TCP",
            Self::Tls { .. } => "TLS",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConnectionProfile {
    pub endpoint: String,
    pub transport: TransportConfig,
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub username: String,
    /// Passwords and tokens are loaded at connection time and are never persisted.
    pub password_env: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default)]
    pub role: ActorRole,
    #[serde(default = "default_heartbeat")]
    pub heartbeat_seconds: u32,
    #[serde(default)]
    pub allow_sequence_reset: bool,
}

impl ConnectionProfile {
    pub fn validate(&self) -> Result<(), String> {
        if self.endpoint.is_empty() || !self.endpoint.contains(':') {
            return Err("profile endpoint must be HOST:PORT".to_owned());
        }
        if self.sender_comp_id.is_empty() || self.target_comp_id.is_empty() {
            return Err("profile SenderCompID and TargetCompID are required".to_owned());
        }
        if self.username.is_empty() || self.password_env.is_empty() {
            return Err("profile username and password_env are required".to_owned());
        }
        if self.heartbeat_seconds == 0 {
            return Err("profile heartbeat_seconds must be greater than zero".to_owned());
        }
        if let TransportConfig::Tls { server_name, .. } = &self.transport
            && server_name.is_empty()
        {
            return Err("TLS server_name is required".to_owned());
        }
        Ok(())
    }

    pub fn password(&self) -> Result<String, String> {
        env::var(&self.password_env).map_err(|_| {
            format!(
                "credential environment variable {} is not set",
                self.password_env
            )
        })
    }
}

const fn default_heartbeat() -> u32 {
    30
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkspaceLayout {
    pub initial_view: String,
    pub theme: String,
    pub sound_enabled: bool,
    pub indicators: Vec<String>,
}

impl Default for WorkspaceLayout {
    fn default() -> Self {
        Self {
            initial_view: "market".to_owned(),
            theme: "dark".to_owned(),
            sound_enabled: false,
            indicators: vec!["ohlc".to_owned(), "volume".to_owned()],
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalConfig {
    pub selected_profile: String,
    pub profiles: BTreeMap<String, ConnectionProfile>,
    #[serde(default)]
    pub workspaces: BTreeMap<String, WorkspaceLayout>,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        let profile = |endpoint: &str, transport, password_env: &str| ConnectionProfile {
            endpoint: endpoint.to_owned(),
            transport,
            sender_comp_id: "HUMAN".to_owned(),
            target_comp_id: "BUNTING".to_owned(),
            username: "participant".to_owned(),
            password_env: password_env.to_owned(),
            team_id: None,
            run_id: None,
            role: ActorRole::Participant,
            heartbeat_seconds: 30,
            allow_sequence_reset: false,
        };
        Self {
            selected_profile: "local".to_owned(),
            profiles: BTreeMap::from([
                (
                    "cloudflare-gateway".to_owned(),
                    profile(
                        "fix-gateway.example.invalid:443",
                        TransportConfig::Tls {
                            server_name: "fix-gateway.example.invalid".to_owned(),
                            ca_file: None,
                        },
                        "BUNTING_CLOUDFLARE_PASSWORD",
                    ),
                ),
                (
                    "local".to_owned(),
                    profile(
                        "127.0.0.1:9880",
                        TransportConfig::Tcp,
                        "BUNTING_LOCAL_PASSWORD",
                    ),
                ),
                (
                    "remote".to_owned(),
                    profile(
                        "fix.example.invalid:9880",
                        TransportConfig::Tls {
                            server_name: "fix.example.invalid".to_owned(),
                            ca_file: None,
                        },
                        "BUNTING_REMOTE_PASSWORD",
                    ),
                ),
            ]),
            workspaces: BTreeMap::from([("default".to_owned(), WorkspaceLayout::default())]),
        }
    }
}

impl TerminalConfig {
    pub fn load(path: Option<PathBuf>) -> Result<(Self, PathBuf), String> {
        let path = path.unwrap_or_else(default_path);
        if !path.exists() {
            return Ok((Self::default(), path));
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        let config: Self = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid terminal config {}: {error}", path.display()))?;
        for (name, profile) in &config.profiles {
            profile
                .validate()
                .map_err(|error| format!("invalid profile {name}: {error}"))?;
        }
        Ok((config, path))
    }

    pub fn save(&self, path: &PathBuf) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        fs::write(path, bytes)
    }

    pub fn profile(&self, name: &str) -> Result<ConnectionProfile, String> {
        self.profiles
            .get(name)
            .cloned()
            .ok_or_else(|| format!("unknown connection profile: {name}"))
    }
}

fn default_path() -> PathBuf {
    if let Some(root) = env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(root).join("bunting/terminal.json");
    }
    env::var_os("HOME").map_or_else(
        || PathBuf::from("bunting-terminal.json"),
        |home| PathBuf::from(home).join(".config/bunting/terminal.json"),
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn defaults_include_all_deployment_profiles_without_secrets() {
        let config = TerminalConfig::default();
        assert!(config.profiles.contains_key("local"));
        assert!(config.profiles.contains_key("remote"));
        assert!(config.profiles.contains_key("cloudflare-gateway"));
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("password\":"));
    }

    #[test]
    fn workspace_round_trip_is_deterministic() {
        let config = TerminalConfig::default();
        let encoded = serde_json::to_string(&config).unwrap();
        let decoded: TerminalConfig = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, config);
    }
}
