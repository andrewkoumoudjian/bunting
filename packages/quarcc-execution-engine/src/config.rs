use bunting_market_types::QuantityLots;
use serde::{Deserialize, Serialize};

pub const EXECUTION_SNAPSHOT_VERSION: u16 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExecutionConfig {
    pub max_orders: usize,
    pub max_actions_per_call: usize,
    pub max_seen_reports: usize,
    pub max_deferred_reports: usize,
    pub max_order_quantity: QuantityLots,
    pub max_absolute_position: QuantityLots,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_orders: 1_024,
            max_actions_per_call: 256,
            max_seen_reports: 16_384,
            max_deferred_reports: 256,
            max_order_quantity: QuantityLots::new(1_000_000),
            max_absolute_position: QuantityLots::new(10_000_000),
        }
    }
}
