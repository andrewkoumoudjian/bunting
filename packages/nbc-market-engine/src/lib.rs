#![forbid(unsafe_code)]
//! Strict configuration and provenance for the authorized NBC market-engine port.

mod config;

#[doc(inline)]
pub use config::{
    ConfigError, ConfigHash, ExactDecimal, LegacyParameters, MarketConfig, Provenance,
    ProvenanceHash, ScenarioConfig, SourceArtifact, StepCount, StepIntervalMillis,
};

// Rust guideline compliant 2026-02-21
