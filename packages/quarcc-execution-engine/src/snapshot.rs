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
    pub seen_reports: BTreeSet<ReportId>,
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
            seen_reports: BTreeSet::new(),
            orders: BTreeMap::new(),
            client_to_local: BTreeMap::new(),
            venue_to_local: BTreeMap::new(),
            deferred_reports: Vec::new(),
            positions: BTreeMap::new(),
            market_data: BTreeMap::new(),
        }
    }
}
