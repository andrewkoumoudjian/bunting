//! Defines the bounded NBC run lifecycle and logical scheduler.

use crate::ScenarioConfig;
use core::fmt;

const MAX_RUN_ID_BYTES: usize = 128;
const MAX_EVENT_ID_BYTES: usize = 256;
const MAX_TERMINATION_REASON_BYTES: usize = 1024;
const MAX_SCHEDULED_EVENTS: usize = 4096;

/// Identifies one inert event at a logical simulation step.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduledEvent {
    id: String,
    trigger_step: u32,
}

impl ScheduledEvent {
    /// Creates an event for one logical step.
    ///
    /// Event effects belong to later NBC slices. This slice only returns identifiers in the
    /// JAR-observed scheduler order.
    ///
    /// # Errors
    /// Returns an error when the identifier is empty or exceeds 256 bytes.
    pub fn new(id: impl Into<String>, trigger_step: u32) -> Result<Self, KernelError> {
        let id = id.into();
        validate_text(&id, MAX_EVENT_ID_BYTES, TextField::EventId)?;
        Ok(Self { id, trigger_step })
    }

    /// Returns the event identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the exact logical trigger step.
    #[must_use]
    pub const fn trigger_step(&self) -> u32 {
        self.trigger_step
    }
}

/// Describes the terminal or active run lifecycle state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Active,
    Completed,
    Terminated { reason: String },
}

/// Reports the deterministic result of one logical advance.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Advance {
    executed_step: u32,
    current_step: u32,
    triggered_event_ids: Vec<String>,
    status: RunStatus,
}

impl Advance {
    /// Returns the step whose scheduled events were evaluated.
    #[must_use]
    pub const fn executed_step(&self) -> u32 {
        self.executed_step
    }

    /// Returns the logical clock after the advance.
    #[must_use]
    pub const fn current_step(&self) -> u32 {
        self.current_step
    }

    /// Returns triggered identifiers in source-list order.
    #[must_use]
    pub fn triggered_event_ids(&self) -> &[String] {
        &self.triggered_event_ids
    }

    /// Returns the lifecycle state after the advance.
    #[must_use]
    pub const fn status(&self) -> &RunStatus {
        &self.status
    }
}

/// Reports a bounded run-kernel failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KernelError {
    EmptyRunId,
    RunIdTooLong,
    EmptyEventId,
    EventIdTooLong,
    TooManyScheduledEvents,
    EventOutsideRun {
        trigger_step: u32,
        duration_steps: u32,
    },
    RunNotActive,
    EmptyTerminationReason,
    TerminationReasonTooLong,
    LogicalStepOverflow,
}

impl fmt::Display for KernelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyRunId => formatter.write_str("run ID must not be empty"),
            Self::RunIdTooLong => formatter.write_str("run ID exceeds 128 bytes"),
            Self::EmptyEventId => formatter.write_str("event ID must not be empty"),
            Self::EventIdTooLong => formatter.write_str("event ID exceeds 256 bytes"),
            Self::TooManyScheduledEvents => formatter.write_str("scheduler exceeds 4096 events"),
            Self::EventOutsideRun {
                trigger_step,
                duration_steps,
            } => write!(
                formatter,
                "event step {trigger_step} is outside run duration {duration_steps}"
            ),
            Self::RunNotActive => formatter.write_str("run is not active"),
            Self::EmptyTerminationReason => {
                formatter.write_str("termination reason must not be empty")
            }
            Self::TerminationReasonTooLong => {
                formatter.write_str("termination reason exceeds 1024 bytes")
            }
            Self::LogicalStepOverflow => formatter.write_str("logical step overflowed"),
        }
    }
}

impl std::error::Error for KernelError {}

#[derive(Clone, Copy)]
enum TextField {
    RunId,
    EventId,
    TerminationReason,
}

/// Owns one deterministic NBC run lifecycle.
pub struct RunKernel {
    run_id: String,
    config: ScenarioConfig,
    current_step: u32,
    scheduled_events: Vec<ScheduledEvent>,
    status: RunStatus,
}

impl RunKernel {
    /// Starts a run at logical step zero.
    ///
    /// Scheduled events remain inert until [`Self::advance`] evaluates their exact trigger step.
    /// The configuration's legacy parameters are retained only as provenance.
    ///
    /// # Errors
    /// Returns an error for invalid identifiers, excessive events, or unreachable event steps.
    pub fn start(
        run_id: impl Into<String>,
        config: ScenarioConfig,
        scheduled_events: Vec<ScheduledEvent>,
    ) -> Result<Self, KernelError> {
        let run_id = run_id.into();
        validate_text(&run_id, MAX_RUN_ID_BYTES, TextField::RunId)?;
        if scheduled_events.len() > MAX_SCHEDULED_EVENTS {
            return Err(KernelError::TooManyScheduledEvents);
        }
        let duration_steps = config.duration_steps.get();
        if let Some(event) = scheduled_events
            .iter()
            .find(|event| event.trigger_step >= duration_steps)
        {
            return Err(KernelError::EventOutsideRun {
                trigger_step: event.trigger_step,
                duration_steps,
            });
        }
        Ok(Self {
            run_id,
            config,
            current_step: 0,
            scheduled_events,
            status: RunStatus::Active,
        })
    }

    /// Returns the run identifier.
    #[must_use]
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// Returns the strict scenario configuration.
    #[must_use]
    pub const fn config(&self) -> &ScenarioConfig {
        &self.config
    }

    /// Returns the current logical step.
    #[must_use]
    pub const fn current_step(&self) -> u32 {
        self.current_step
    }

    /// Returns the current lifecycle state.
    #[must_use]
    pub const fn status(&self) -> &RunStatus {
        &self.status
    }

    /// Advances one logical step and returns same-step events in source order.
    ///
    /// # Errors
    /// Returns an error after completion or termination, or if the clock overflows.
    pub fn advance(&mut self) -> Result<Advance, KernelError> {
        if self.status != RunStatus::Active {
            return Err(KernelError::RunNotActive);
        }
        let executed_step = self.current_step;
        let triggered_event_ids = self
            .scheduled_events
            .iter()
            .filter(|event| event.trigger_step == executed_step)
            .map(|event| event.id.clone())
            .collect();
        self.current_step = self
            .current_step
            .checked_add(1)
            .ok_or(KernelError::LogicalStepOverflow)?;
        if self.current_step == self.config.duration_steps.get() {
            self.status = RunStatus::Completed;
        }
        Ok(Advance {
            executed_step,
            current_step: self.current_step,
            triggered_event_ids,
            status: self.status.clone(),
        })
    }

    /// Terminates an active run with a bounded reason.
    ///
    /// # Errors
    /// Returns an error for inactive runs or invalid reason text.
    pub fn terminate(&mut self, reason: impl Into<String>) -> Result<(), KernelError> {
        if self.status != RunStatus::Active {
            return Err(KernelError::RunNotActive);
        }
        let reason = reason.into();
        validate_text(
            &reason,
            MAX_TERMINATION_REASON_BYTES,
            TextField::TerminationReason,
        )?;
        self.status = RunStatus::Terminated { reason };
        Ok(())
    }
}

fn validate_text(value: &str, maximum_bytes: usize, field: TextField) -> Result<(), KernelError> {
    if value.is_empty() {
        return Err(match field {
            TextField::RunId => KernelError::EmptyRunId,
            TextField::EventId => KernelError::EmptyEventId,
            TextField::TerminationReason => KernelError::EmptyTerminationReason,
        });
    }
    if value.len() > maximum_bytes {
        return Err(match field {
            TextField::RunId => KernelError::RunIdTooLong,
            TextField::EventId => KernelError::EventIdTooLong,
            TextField::TerminationReason => KernelError::TerminationReasonTooLong,
        });
    }
    Ok(())
}

// Rust guideline compliant 2026-02-21
