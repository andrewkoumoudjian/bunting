use bunting_market_types::{InstrumentId, LogicalTimeNs, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MarketObservation {
    pub instrument_id: InstrumentId,
    pub logical_time: LogicalTimeNs,
    pub best_bid: Option<(PriceTicks, QuantityLots)>,
    pub best_ask: Option<(PriceTicks, QuantityLots)>,
    pub last_trade: Option<(PriceTicks, QuantityLots)>,
    pub committed_sequence: u64,
}
