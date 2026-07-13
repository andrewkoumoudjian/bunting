//! Defines strict, deterministic NBC configuration values.

use core::fmt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const CONFIG_SCHEMA: &str = "bunting.nbc.config.v1";
const ENGINE_VERSION: &str = "nbc-v1-config-1";
const SHA256_HEX_LENGTH: usize = 64;
const MAX_CONFIG_BYTES: usize = 256 * 1024;
const MAX_SOURCE_ARTIFACTS: usize = 32;
const MAX_UNRESOLVED_FIELDS: usize = 128;

/// Reports a strict NBC configuration failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    InvalidJson(String),
    UnsupportedSchema(String),
    UnsupportedEngineVersion(String),
    EmptyText(&'static str),
    InvalidPositiveValue(&'static str),
    InvalidDecimal(String),
    InvalidSha256(String),
    ConfigurationTooLarge,
    TooManySourceArtifacts,
    TooManyUnresolvedFields,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidJson(message) => {
                write!(formatter, "invalid NBC configuration JSON: {message}")
            }
            Self::UnsupportedSchema(value) => write!(formatter, "unsupported NBC schema: {value}"),
            Self::UnsupportedEngineVersion(value) => {
                write!(formatter, "unsupported NBC engine version: {value}")
            }
            Self::EmptyText(field) => write!(formatter, "{field} must not be empty"),
            Self::InvalidPositiveValue(field) => write!(formatter, "{field} must be positive"),
            Self::InvalidDecimal(value) => write!(formatter, "invalid exact decimal: {value}"),
            Self::InvalidSha256(value) => write!(formatter, "invalid SHA-256: {value}"),
            Self::ConfigurationTooLarge => formatter.write_str("NBC configuration exceeds 256 KiB"),
            Self::TooManySourceArtifacts => {
                formatter.write_str("NBC provenance exceeds 32 artifacts")
            }
            Self::TooManyUnresolvedFields => {
                formatter.write_str("NBC provenance exceeds 128 unresolved fields")
            }
        }
    }
}

impl std::error::Error for ConfigError {}

/// Stores a positive simulation duration measured in logical steps.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct StepCount(u32);

impl StepCount {
    /// Creates a positive logical-step count.
    ///
    /// # Errors
    /// Returns an error when `value` is zero.
    pub fn new(value: u32) -> Result<Self, ConfigError> {
        if value == 0 {
            Err(ConfigError::InvalidPositiveValue("duration_steps"))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the logical-step count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for StepCount {
    type Error = ConfigError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<StepCount> for u32 {
    fn from(value: StepCount) -> Self {
        value.get()
    }
}

/// Stores a positive wall-clock interval measured exactly in milliseconds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct StepIntervalMillis(u32);

impl StepIntervalMillis {
    /// Creates a positive millisecond interval.
    ///
    /// # Errors
    /// Returns an error when `value` is zero.
    pub fn new(value: u32) -> Result<Self, ConfigError> {
        if value == 0 {
            Err(ConfigError::InvalidPositiveValue("step_interval_ms"))
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the exact millisecond count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl TryFrom<u32> for StepIntervalMillis {
    type Error = ConfigError;

    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<StepIntervalMillis> for u32 {
    fn from(value: StepIntervalMillis) -> Self {
        value.get()
    }
}

/// Stores a base-ten decimal without binary floating-point conversion.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ExactDecimal(String);

impl ExactDecimal {
    /// Parses a canonical base-ten decimal string.
    ///
    /// # Errors
    /// Returns an error for exponent notation, redundant signs, or non-canonical zeros.
    pub fn parse(value: &str) -> Result<Self, ConfigError> {
        if is_canonical_decimal(value) {
            Ok(Self(value.to_owned()))
        } else {
            Err(ConfigError::InvalidDecimal(value.to_owned()))
        }
    }

    /// Returns the exact canonical decimal text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for ExactDecimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

fn is_canonical_decimal(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    if unsigned.is_empty() || value.starts_with('+') || value == "-0" {
        return false;
    }
    let mut parts = unsigned.split('.');
    let integer = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || integer.is_empty()
        || !integer.bytes().all(|byte| byte.is_ascii_digit())
        || (integer.len() > 1 && integer.starts_with('0'))
    {
        return false;
    }
    fraction.is_none_or(|digits| {
        !digits.is_empty()
            && digits.bytes().all(|byte| byte.is_ascii_digit())
            && !digits.ends_with('0')
    })
}

/// Defines market values whose units are explicit and non-floating.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MarketConfig {
    pub price_unit: String,
    pub initial_fundamental_value: ExactDecimal,
    pub tick_size: ExactDecimal,
    pub initial_spread: Option<ExactDecimal>,
}

/// Preserves unresolved reference parameters without making them executable.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(transparent)]
pub struct LegacyParameters(BTreeMap<String, Value>);

impl LegacyParameters {
    /// Returns the inert unresolved parameter map.
    #[must_use]
    pub const fn as_map(&self) -> &BTreeMap<String, Value> {
        &self.0
    }
}

/// Identifies one source artifact and its exact content digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceArtifact {
    pub path: String,
    pub sha256: String,
    pub classification: String,
}

/// Records the source and translation identity of a configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    pub jar_sha256: String,
    pub jar_gitlink: String,
    pub source_scenario_id: String,
    pub artifacts: Vec<SourceArtifact>,
    pub unit_mappings: BTreeMap<String, String>,
    pub unresolved_fields: Vec<String>,
    pub translation_disposition: String,
}

/// Defines the executable Sprint 7.1 NBC configuration boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    pub schema_version: String,
    pub engine_version: String,
    pub scenario_id: String,
    pub scenario_name: String,
    pub seed: u64,
    pub duration_steps: StepCount,
    pub step_interval_ms: StepIntervalMillis,
    pub market: MarketConfig,
    pub legacy_parameters: LegacyParameters,
    pub provenance: Provenance,
}

/// Contains a lowercase SHA-256 configuration digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigHash(String);

impl ConfigHash {
    /// Returns the lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Contains a lowercase SHA-256 provenance digest.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceHash(String);

impl ProvenanceHash {
    /// Returns the lowercase hexadecimal digest.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProvenanceHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl fmt::Display for ConfigHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl ScenarioConfig {
    /// Parses and validates strict canonical configuration JSON.
    ///
    /// # Errors
    /// Returns an error for malformed JSON, unknown fields, invalid units, or provenance.
    pub fn from_json(bytes: &[u8]) -> Result<Self, ConfigError> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::ConfigurationTooLarge);
        }
        let config: Self = serde_json::from_slice(bytes)
            .map_err(|error| ConfigError::InvalidJson(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Produces a deterministic SHA-256 over canonical serialized configuration bytes.
    ///
    /// # Errors
    /// Returns an error if serialization unexpectedly fails.
    pub fn deterministic_hash(&self) -> Result<ConfigHash, ConfigError> {
        hash_serializable(self).map(ConfigHash)
    }

    /// Produces a deterministic SHA-256 over canonical provenance bytes.
    ///
    /// # Errors
    /// Returns an error if serialization unexpectedly fails.
    pub fn provenance_hash(&self) -> Result<ProvenanceHash, ConfigError> {
        hash_serializable(&self.provenance).map(ProvenanceHash)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != CONFIG_SCHEMA {
            return Err(ConfigError::UnsupportedSchema(self.schema_version.clone()));
        }
        if self.engine_version != ENGINE_VERSION {
            return Err(ConfigError::UnsupportedEngineVersion(
                self.engine_version.clone(),
            ));
        }
        for (field, value) in [
            ("scenario_id", self.scenario_id.as_str()),
            ("scenario_name", self.scenario_name.as_str()),
            ("market.price_unit", self.market.price_unit.as_str()),
            (
                "provenance.jar_gitlink",
                self.provenance.jar_gitlink.as_str(),
            ),
            (
                "provenance.source_scenario_id",
                self.provenance.source_scenario_id.as_str(),
            ),
            (
                "provenance.translation_disposition",
                self.provenance.translation_disposition.as_str(),
            ),
        ] {
            if value.is_empty() {
                return Err(ConfigError::EmptyText(field));
            }
        }
        validate_sha256(&self.provenance.jar_sha256)?;
        if self.provenance.artifacts.len() > MAX_SOURCE_ARTIFACTS {
            return Err(ConfigError::TooManySourceArtifacts);
        }
        if self.provenance.unresolved_fields.len() > MAX_UNRESOLVED_FIELDS {
            return Err(ConfigError::TooManyUnresolvedFields);
        }
        for artifact in &self.provenance.artifacts {
            if artifact.path.is_empty() {
                return Err(ConfigError::EmptyText("provenance.artifacts.path"));
            }
            validate_sha256(&artifact.sha256)?;
        }
        if self.provenance.source_scenario_id != self.scenario_id {
            return Err(ConfigError::InvalidJson(
                "source_scenario_id must match scenario_id".to_owned(),
            ));
        }
        Ok(())
    }
}

fn hash_serializable(value: &impl Serialize) -> Result<String, ConfigError> {
    let bytes =
        serde_json::to_vec(value).map_err(|error| ConfigError::InvalidJson(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(SHA256_HEX_LENGTH);
    for byte in digest {
        use core::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}")
            .map_err(|error| ConfigError::InvalidJson(error.to_string()))?;
    }
    Ok(encoded)
}

fn validate_sha256(value: &str) -> Result<(), ConfigError> {
    if value.len() == SHA256_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ConfigError::InvalidSha256(value.to_owned()))
    }
}

// Rust guideline compliant 2026-02-21
