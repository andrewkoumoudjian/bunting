#![forbid(unsafe_code)]
//! Implements authorized NBC configuration and deterministic run-kernel behavior.

mod config;
mod engine;

#[doc(inline)]
pub use config::{
    ConfigError, ConfigHash, ExactDecimal, LegacyParameters, MarketConfig, Provenance,
    ProvenanceHash, ScenarioConfig, SourceArtifact, StepCount, StepIntervalMillis,
};
#[doc(inline)]
pub use engine::{Advance, KernelError, RunKernel, RunStatus, ScheduledEvent};

// Rust guideline compliant 2026-02-21
