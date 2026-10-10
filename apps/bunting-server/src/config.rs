use bunting_agents::PolicyKind;
use bunting_api_contract::ActorRole;
use bunting_market_types::{InstrumentId, ParticipantId, PriceTicks, QuantityLots, RunId};
use bunting_runtime::{RuntimeAgentConfig, RuntimeConfig};
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
    /// Committed commands per run; each costs one in-memory index entry.
    pub max_commands_per_run: usize,
    pub max_events_per_run: usize,
    /// Committed commands between state checkpoints. Bounds both restart
    /// re-execution and in-memory rollback work.
    pub checkpoint_interval: usize,
    /// Removed: the store-wide command bound became `max_commands_per_run`.
    /// Present only so a stale configuration fails with an explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_commands: Option<usize>,
}

impl StorageConfig {
    #[must_use]
    pub fn limits(&self) -> bunting_origin_store::RunLimits {
        bunting_origin_store::RunLimits {
            max_commands: self.max_commands_per_run,
            max_events: u64::try_from(self.max_events_per_run).unwrap_or(u64::MAX),
            checkpoint_interval: self.checkpoint_interval,
        }
    }
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
    pub run_id: u128,
    pub roster: Vec<RosterEntry>,
    pub heartbeat_seconds: u32,
    pub max_connections: usize,
    /// Window for `max_messages_per_interval`, a per-participant rate limit.
    /// It has no effect on ordering (ADR 0034).
    pub rate_limit_window_ms: u64,
    pub max_messages_per_interval: usize,
    /// Removed: the per-participant live-order cap is the scenario's
    /// `max_live_orders` risk limit, enforced by the engine. Present only so a
    /// stale configuration fails with an explanation instead of being ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_open_orders: Option<usize>,
    pub max_message_bytes: usize,
    pub max_journal_messages: usize,
    pub max_pending_inbound: usize,
    pub tls: TlsConfig,
    /// Latency-modeled admission (ADR 0030 as amended by ADR 0034).
    pub admission: AdmissionConfig,
    /// Removed with the ADR 0024 interval writer; present only so a stale
    /// configuration fails with an explanation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matching_interval_ms: Option<u64>,
    /// Removed with the ADR 0024 interval writer (see `admission`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_interval_queue: Option<usize>,
}

/// Latency (ADR 0035): each team's real network delay
/// counts as it is, and the scenario's virtual distance from each team to
/// each venue is added in both directions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionConfig {
    /// Virtual team-to-venue latency table; published before the round and
    /// never changed during it.
    pub policy: bunting_admission_sequencer::LatencyPolicy,
    /// Interval between access-latency probes after the logon burst.
    pub probe_interval_ms: u64,
    /// Commands waiting for release, across all sessions.
    pub max_admission_queue: usize,
    /// Outbound messages one session may have in flight on its virtual
    /// path; overflow disconnects.
    pub max_outbound_hold: usize,
}

impl AdmissionConfig {
    /// Every team colocated with every venue (zero virtual distance), so
    /// only real network delay separates teams.
    #[must_use]
    pub fn colocated() -> Self {
        Self::with_policy(bunting_admission_sequencer::LatencyPolicy::default())
    }

    #[must_use]
    pub const fn with_policy(policy: bunting_admission_sequencer::LatencyPolicy) -> Self {
        Self {
            policy,
            probe_interval_ms: 1_000,
            max_admission_queue: 4_096,
            max_outbound_hold: 4_096,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RosterEntry {
    pub target_comp_id: String,
    pub username: String,
    pub password: String,
    #[serde(default = "participant_role")]
    pub role: ActorRole,
    pub participant_id: u128,
}

const fn participant_role() -> ActorRole {
    ActorRole::Participant
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
pub struct ScenarioRuntimeConfig {
    pub wall_tick_ms: u64,
    pub scheduler: RuntimeConfig,
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
    #[serde(default)]
    pub runtime: Option<ScenarioRuntimeConfig>,
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
    /// Returns a bounded, ephemeral loopback profile suitable for local use.
    #[must_use]
    pub fn local_default() -> Self {
        Self {
            version: 1,
            profile: DeploymentProfile::Local,
            storage: StorageConfig {
                kind: StorageKind::Memory,
                path: None,
                max_runs: 4,
                max_commands_per_run: 1_048_576,
                max_events_per_run: 8_388_608,
                checkpoint_interval: 8_192,
                max_commands: None,
            },
            fix: Some(FixConfig {
                bind: "127.0.0.1:9880".to_owned(),
                sender_comp_id: "BUNTING".to_owned(),
                run_id: 1,
                roster: vec![
                    RosterEntry {
                        target_comp_id: "HUMAN".to_owned(),
                        username: "participant".to_owned(),
                        password: "bunting-local-dev".to_owned(),
                        role: ActorRole::Participant,
                        participant_id: 1,
                    },
                    RosterEntry {
                        target_comp_id: "TEAM2".to_owned(),
                        username: "team2".to_owned(),
                        password: "bunting-team2-dev".to_owned(),
                        role: ActorRole::Participant,
                        participant_id: 2,
                    },
                ],
                heartbeat_seconds: 30,
                max_connections: 2,
                rate_limit_window_ms: 100,
                max_messages_per_interval: 64,
                max_open_orders: None,
                max_message_bytes: 16_384,
                max_journal_messages: 4_096,
                max_pending_inbound: 64,
                tls: TlsConfig::Disabled,
                admission: AdmissionConfig::colocated(),
                matching_interval_ms: None,
                max_interval_queue: None,
            }),
            admin: Some(AdminConfig {
                bind: "127.0.0.1:8080".to_owned(),
                bearer_token: "bunting-local-admin-token".to_owned(),
                max_request_bytes: 4_096,
            }),
            scenario: None,
            runtime: Some(ScenarioRuntimeConfig {
                wall_tick_ms: 250,
                scheduler: RuntimeConfig {
                    run_id: RunId::new(1),
                    instrument_id: InstrumentId::new(1),
                    fundamental_price: PriceTicks::new(100),
                    remaining_parent_quantity: QuantityLots::new(1_000),
                    max_actions_per_tick: 256,
                    agents: vec![RuntimeAgentConfig {
                        kind: PolicyKind::StaticLiquidityProvider,
                        participant_id: ParticipantId::new(10),
                        base_quantity: QuantityLots::new(5),
                        spread_ticks: 2,
                        inventory_target: QuantityLots::new(0),
                        wake_interval_ns: 1_000_000_000,
                        seed: 42,
                        max_intents_per_wake: 4,
                    }],
                },
            }),
        }
    }

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
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.version != 1 {
            return Err(ConfigError(format!(
                "unsupported configuration version {}; expected 1",
                self.version
            )));
        }
        if self.storage.max_commands.is_some() {
            return Err(ConfigError(
                "storage.max_commands was removed: command bounds are per run; set storage.max_commands_per_run and storage.checkpoint_interval instead"
                    .to_owned(),
            ));
        }
        if self.storage.max_runs == 0
            || self.storage.max_commands_per_run == 0
            || self.storage.max_events_per_run == 0
            || self.storage.checkpoint_interval == 0
        {
            return Err(ConfigError(
                "storage bounds max_runs, max_commands_per_run, max_events_per_run and checkpoint_interval must be positive"
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
        if self.profile == DeploymentProfile::HostedNative
            && (self.storage.kind != StorageKind::File || self.scenario.is_none())
        {
            return Err(ConfigError(
                "hosted-native requires bounded file storage and an immutable scenario".to_owned(),
            ));
        }
        if let Some(fix) = &self.fix {
            validate_fix(fix, self.profile)?;
        }
        if let Some(admin) = &self.admin {
            let bind = parse_socket("admin.bind", &admin.bind)?;
            if self.profile == DeploymentProfile::HostedNative && !bind.ip().is_loopback() {
                return Err(ConfigError(
                    "hosted-native admin.bind must remain loopback behind the authenticated terminator"
                        .to_owned(),
                ));
            }
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
        if let Some(runtime) = &self.runtime {
            validate_runtime(runtime, self.scenario.as_ref(), self.fix.as_ref())?;
        }
        Ok(())
    }
}

fn validate_runtime(
    runtime: &ScenarioRuntimeConfig,
    scenario: Option<&ScenarioConfig>,
    fix: Option<&FixConfig>,
) -> Result<(), ConfigError> {
    if !(1..=60_000).contains(&runtime.wall_tick_ms) {
        return Err(ConfigError(
            "runtime.wall_tick_ms must be 1..=60000".to_owned(),
        ));
    }
    runtime
        .scheduler
        .validate()
        .map_err(|error| ConfigError(format!("invalid runtime scheduler: {error}")))?;
    if scenario.is_some_and(|value| value.run_id != runtime.scheduler.run_id.get())
        || fix.is_some_and(|value| value.run_id != runtime.scheduler.run_id.get())
    {
        return Err(ConfigError(
            "runtime, scenario and FIX run IDs must match".to_owned(),
        ));
    }
    Ok(())
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
    if fix.sender_comp_id.is_empty() || fix.roster.is_empty() {
        return Err(ConfigError(
            "fix.sender_comp_id and fix.roster must be non-empty".to_owned(),
        ));
    }
    let mut participants = std::collections::BTreeSet::new();
    let mut usernames = std::collections::BTreeSet::new();
    let mut comp_ids = std::collections::BTreeSet::new();
    for entry in &fix.roster {
        if entry.target_comp_id.is_empty()
            || entry.username.is_empty()
            || entry.password.len() < 12
            || entry.participant_id == 0
            || !participants.insert(entry.participant_id)
            || !usernames.insert(entry.username.as_str())
            || !comp_ids.insert(entry.target_comp_id.as_str())
        {
            return Err(ConfigError(
                "fix.roster requires unique non-zero participant IDs, unique non-empty usernames/CompIDs and passwords of at least 12 bytes"
                    .to_owned(),
            ));
        }
    }
    if fix.run_id == 0 {
        return Err(ConfigError("fix.run_id must be non-zero".to_owned()));
    }
    if fix.max_open_orders.is_some() {
        return Err(ConfigError(
            "fix.max_open_orders was removed: set the per-participant `max_live_orders` risk limit in the scenario instead; the engine enforces it for every connection"
                .to_owned(),
        ));
    }
    if fix.matching_interval_ms.is_some() || fix.max_interval_queue.is_some() {
        return Err(ConfigError(
            "fix.matching_interval_ms and fix.max_interval_queue were removed with the ADR 0024 interval writer: set fix.rate_limit_window_ms for the message-rate window and fix.admission for latency-modeled admission (ADR 0034)"
                .to_owned(),
        ));
    }
    validate_admission(&fix.admission)?;
    if fix.max_connections == 0
        || fix.max_connections > fix.roster.len()
        || !(1..=60_000).contains(&fix.rate_limit_window_ms)
        || fix.max_messages_per_interval == 0
        || !(256..=1_048_576).contains(&fix.max_message_bytes)
        || fix.max_journal_messages == 0
        || fix.max_pending_inbound == 0
        || fix.heartbeat_seconds == 0
    {
        return Err(ConfigError(
            "FIX bounds are invalid; max_connections must fit the roster, rate_limit_window_ms must be 1..=60000, and message, wire, heartbeat, journal and pending limits must be positive"
                .to_owned(),
        ));
    }
    if profile == DeploymentProfile::HostedNative {
        validate_tls(bind, &fix.tls, "fix")?;
    }
    Ok(())
}

fn validate_admission(admission: &AdmissionConfig) -> Result<(), ConfigError> {
    admission
        .policy
        .validate()
        .map_err(|error| ConfigError(format!("fix.admission.policy: {error}")))?;
    if !(50..=60_000).contains(&admission.probe_interval_ms)
        || admission.max_admission_queue == 0
        || admission.max_outbound_hold == 0
    {
        return Err(ConfigError(
            "fix.admission bounds are invalid; probe_interval_ms must be 50..=60000 and queue limits positive"
                .to_owned(),
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
            run_id: 1,
            roster: vec![RosterEntry {
                target_comp_id: "CLIENT".to_owned(),
                username: "client".to_owned(),
                password: "long-password".to_owned(),
                role: ActorRole::Participant,
                participant_id: 1,
            }],
            heartbeat_seconds: 30,
            max_connections: 1,
            rate_limit_window_ms: 100,
            max_messages_per_interval: 64,
            max_open_orders: None,
            max_message_bytes: 16_384,
            max_journal_messages: 1_024,
            max_pending_inbound: 32,
            tls: TlsConfig::Disabled,
            admission: AdmissionConfig::colocated(),
            matching_interval_ms: None,
            max_interval_queue: None,
        };
        let Err(error) = validate_fix(&fix, DeploymentProfile::HostedNative) else {
            return;
        };
        assert!(error.0.contains("TLS is disabled"));
    }

    #[test]
    fn removed_interval_keys_fail_with_an_explanation() -> Result<(), ConfigError> {
        let mut config = ServerConfig::local_default();
        let Some(fix) = config.fix.as_mut() else {
            return Err(ConfigError("local profile has FIX".to_owned()));
        };
        fix.matching_interval_ms = Some(100);
        let error = config.validate().err().map(|error| error.0);
        assert!(error.is_some_and(|text| text.contains("rate_limit_window_ms")));

        Ok(())
    }

    #[test]
    fn checked_in_profiles_parse_and_validate() -> Result<(), ConfigError> {
        for value in [
            include_str!("../config/local.json"),
            include_str!("../config/hosted-native.json"),
        ] {
            let config: ServerConfig = serde_json::from_str(value)
                .map_err(|error| ConfigError(format!("profile JSON invalid: {error}")))?;
            config.validate()?;
        }
        Ok(())
    }

    #[test]
    fn removed_max_open_orders_fails_with_its_replacement() -> Result<(), ConfigError> {
        let mut config = ServerConfig::local_default();
        if let Some(fix) = config.fix.as_mut() {
            fix.max_open_orders = Some(256);
        }
        let Err(error) = config.validate() else {
            return Err(ConfigError("stale max_open_orders was accepted".to_owned()));
        };
        assert!(error.0.contains("max_live_orders"));
        Ok(())
    }

    #[test]
    fn zero_configuration_local_profile_is_bounded_and_valid() -> Result<(), ConfigError> {
        let config = ServerConfig::local_default();
        config.validate()?;
        assert_eq!(config.profile, DeploymentProfile::Local);
        assert_eq!(config.fix.as_ref().map(|fix| fix.max_connections), Some(2));
        assert_eq!(config.storage.kind, StorageKind::Memory);
        Ok(())
    }

    #[test]
    fn hosted_sessions_require_durable_isolated_state() -> Result<(), ConfigError> {
        let mut config: ServerConfig =
            serde_json::from_str(include_str!("../config/hosted-native.json"))
                .map_err(|error| ConfigError(error.to_string()))?;
        config.storage.kind = StorageKind::Memory;
        config.storage.path = None;
        let Err(error) = config.validate() else {
            return Err(ConfigError("memory-hosted profile was accepted".to_owned()));
        };
        assert!(error.0.contains("bounded file storage"));
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
