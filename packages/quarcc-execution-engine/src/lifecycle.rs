use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderLifecycle {
    IntentReceived,
    PendingSubmit,
    Live,
    PartiallyFilled,
    PendingCancel,
    PendingReplace,
    Cancelled,
    Filled,
    Rejected,
    Expired,
    ExternallyDiscovered,
    Quarantined,
}

impl OrderLifecycle {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Cancelled | Self::Filled | Self::Rejected | Self::Expired
        )
    }

    #[must_use]
    pub const fn is_open(self) -> bool {
        !self.is_terminal() && !matches!(self, Self::Quarantined)
    }
}
