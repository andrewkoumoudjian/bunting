#![forbid(unsafe_code)]
//! Implements authorized NBC configuration and deterministic run-kernel behavior.

mod config;
mod engine;
mod matching;

#[doc(inline)]
pub use config::{
    ConfigError, ConfigHash, ExactDecimal, LegacyParameters, MarketConfig, Provenance,
    ProvenanceHash, ScenarioConfig, SourceArtifact, StepCount, StepIntervalMillis,
};
#[doc(inline)]
pub use engine::{Advance, KernelError, RunKernel, RunStatus, ScheduledEvent};
#[doc(inline)]
pub use matching::{
    CancelOutcome, Fill, MAX_OPEN_ORDERS, MatchError, MatchOutcome, NBC_EXTERNAL_LOT_SIZE,
    NbcOrderBook, OpenOrder,
};

// Rust guideline compliant 2026-02-21
