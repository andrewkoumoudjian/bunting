use bunting_market_types::ParticipantId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const MAX_DONE_PARTICIPANTS: usize = 1_024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DoneBarrier {
    participants: BTreeSet<ParticipantId>,
    acknowledged: BTreeSet<ParticipantId>,
    step: u32,
}

impl DoneBarrier {
    pub fn new(
        participants: impl IntoIterator<Item = ParticipantId>,
    ) -> Result<Self, SynchronizationError> {
        let participants: BTreeSet<_> = participants.into_iter().collect();
        if participants.is_empty() {
            return Err(SynchronizationError::NoParticipants);
        }
        if participants.len() > MAX_DONE_PARTICIPANTS {
            return Err(SynchronizationError::TooManyParticipants);
        }
        Ok(Self {
            participants,
            acknowledged: BTreeSet::new(),
            step: 0,
        })
    }

    pub fn acknowledge(
        &mut self,
        participant: ParticipantId,
        step: u32,
    ) -> Result<(), SynchronizationError> {
        if step != self.step {
            return Err(SynchronizationError::WrongStep {
                expected: self.step,
                actual: step,
            });
        }
        if !self.participants.contains(&participant) {
            return Err(SynchronizationError::UnknownParticipant);
        }
        if !self.acknowledged.insert(participant) {
            return Err(SynchronizationError::DuplicateDone);
        }
        Ok(())
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.acknowledged == self.participants
    }

    pub fn begin_step(&mut self, step: u32) -> Result<(), SynchronizationError> {
        if !self.is_ready() {
            return Err(SynchronizationError::BarrierIncomplete);
        }
        self.step = step;
        self.acknowledged.clear();
        Ok(())
    }

    #[must_use]
    pub const fn step(&self) -> u32 {
        self.step
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SynchronizationError {
    NoParticipants,
    TooManyParticipants,
    UnknownParticipant,
    DuplicateDone,
    WrongStep { expected: u32, actual: u32 },
    BarrierIncomplete,
}
