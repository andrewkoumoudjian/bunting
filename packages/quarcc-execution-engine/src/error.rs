use crate::ids::{ClientOrderId, LocalOrderId, ReportId, VenueOrderId};
use core::fmt;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionError {
    BufferFull { limit: usize },
    DuplicateIntent,
    InvalidQuantity,
    InvalidPrice,
    InvalidTransition { from: String, report: String },
    KillSwitchActive,
    OpenOrderLimit,
    PositionLimit,
    UnknownClientOrder(ClientOrderId),
    UnknownLocalOrder(LocalOrderId),
    UnknownVenueOrder(VenueOrderId),
    AmbiguousReport(ReportId),
    ArithmeticOverflow,
    InvalidSnapshotVersion(u16),
    SnapshotEncoding,
}

impl ExecutionError {
    /// A participant-side pre-trade check refused the intent: nothing was
    /// sent and local state is unchanged, so the participant carries on.
    #[must_use]
    pub const fn is_pre_trade_refusal(&self) -> bool {
        matches!(
            self,
            Self::OpenOrderLimit | Self::PositionLimit | Self::KillSwitchActive
        )
    }
}

impl fmt::Display for ExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ExecutionError {}
