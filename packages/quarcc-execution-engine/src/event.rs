use crate::ids::{ActionId, LocalOrderId, VenueOrderId};
use crate::order::DesiredOrder;
use bunting_market_events::OrderKind;
use bunting_market_types::QuantityLots;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionAction {
    Submit {
        action_id: ActionId,
        local_order_id: LocalOrderId,
        order: DesiredOrder,
    },
    Cancel {
        action_id: ActionId,
        local_order_id: LocalOrderId,
        venue_order_id: Option<VenueOrderId>,
    },
    Replace {
        action_id: ActionId,
        local_order_id: LocalOrderId,
        venue_order_id: VenueOrderId,
        quantity: QuantityLots,
        kind: OrderKind,
    },
    QueryOrder {
        action_id: ActionId,
        local_order_id: LocalOrderId,
        venue_order_id: Option<VenueOrderId>,
    },
    QueryOpenOrders {
        action_id: ActionId,
    },
}
