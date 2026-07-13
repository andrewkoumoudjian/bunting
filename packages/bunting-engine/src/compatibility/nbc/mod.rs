//! Provenance-linked NBC configuration, scheduling, and synchronization.

mod config;
mod profile;
mod scheduler;
mod synchronization;
mod translation;

pub use config::{
    ConfigError, ConfigHash, ExactDecimal, LegacyParameters, MarketConfig, Provenance,
    ProvenanceHash, ScenarioConfig, SourceArtifact, StepCount, StepIntervalMillis,
};
pub use profile::NbcProfileCapabilities;
pub use scheduler::{Advance, KernelError, RunKernel, RunStatus, ScheduledEvent};
pub use synchronization::{DoneBarrier, SynchronizationError};
pub use translation::{NBC_JAR_SHA256, NBC_TRANSLATION_VERSION};

use bunting_market_types::ParticipantId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NbcCompatibilityState {
    pub profile_version: u16,
    pub scheduler: RunKernel,
    pub done: DoneBarrier,
}

impl NbcCompatibilityState {
    pub fn new(
        run_id: impl Into<String>,
        config: ScenarioConfig,
        events: Vec<ScheduledEvent>,
        participants: impl IntoIterator<Item = ParticipantId>,
    ) -> Result<Self, NbcCompatibilityError> {
        Ok(Self {
            profile_version: NBC_TRANSLATION_VERSION,
            scheduler: RunKernel::start(run_id, config, events)?,
            done: DoneBarrier::new(participants)?,
        })
    }

    pub fn acknowledge_and_advance(
        &mut self,
        participant_id: ParticipantId,
        step: u32,
    ) -> Result<Option<Advance>, NbcCompatibilityError> {
        self.done.acknowledge(participant_id, step)?;
        if !self.done.is_ready() {
            return Ok(None);
        }
        let advance = self.scheduler.advance()?;
        self.done.begin_step(advance.current_step())?;
        Ok(Some(advance))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NbcCompatibilityError {
    Scheduler(KernelError),
    Synchronization(SynchronizationError),
}

impl From<KernelError> for NbcCompatibilityError {
    fn from(value: KernelError) -> Self {
        Self::Scheduler(value)
    }
}

impl From<SynchronizationError> for NbcCompatibilityError {
    fn from(value: SynchronizationError) -> Self {
        Self::Synchronization(value)
    }
}
