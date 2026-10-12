use crate::config::{EXECUTION_SNAPSHOT_VERSION, ExecutionConfig};
use crate::ids::{ClientOrderId, IntentId, LocalOrderId, ReportId, VenueOrderId};
use crate::market_data::MarketObservation;
use crate::normalized_report::NormalizedVenueReport;
use crate::order::ManagedOrder;
use crate::positions::PositionProjection;
use bunting_market_types::InstrumentId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionSnapshot {
    pub version: u16,
    pub config: ExecutionConfig,
    pub next_id: u128,
    pub kill_switch_active: bool,
    pub last_venue_sequence: u64,
    pub processed_intents: BTreeSet<IntentId>,
    /// Every intent at or below this ID counts as processed: the oldest
    /// IDs leave `processed_intents` once it holds `max_seen_reports`.
    /// Intent IDs only grow, so a long session's set stays bounded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intents_processed_through: Option<IntentId>,
    pub seen_reports: BTreeSet<ReportId>,
    /// Every report at or below this ID counts as seen, the same way:
    /// report IDs follow the venue's commit order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reports_seen_through: Option<ReportId>,
    pub orders: BTreeMap<LocalOrderId, ManagedOrder>,
    pub client_to_local: BTreeMap<ClientOrderId, LocalOrderId>,
    pub venue_to_local: BTreeMap<VenueOrderId, LocalOrderId>,
    pub deferred_reports: Vec<NormalizedVenueReport>,
    pub positions: BTreeMap<InstrumentId, PositionProjection>,
    pub market_data: BTreeMap<InstrumentId, MarketObservation>,
}

impl ExecutionSnapshot {
    #[must_use]
    pub fn empty(config: ExecutionConfig) -> Self {
        Self {
            version: EXECUTION_SNAPSHOT_VERSION,
            config,
            next_id: 1,
            kill_switch_active: false,
            last_venue_sequence: 0,
            processed_intents: BTreeSet::new(),
            intents_processed_through: None,
            seen_reports: BTreeSet::new(),
            reports_seen_through: None,
            orders: BTreeMap::new(),
            client_to_local: BTreeMap::new(),
            venue_to_local: BTreeMap::new(),
            deferred_reports: Vec::new(),
            positions: BTreeMap::new(),
            market_data: BTreeMap::new(),
        }
    }
}
