use crate::ids::{ClientOrderId, VenueOrderId};
use crate::order::DesiredOrder;
use crate::positions::AuthoritativePosition;
use bunting_market_types::QuantityLots;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoritativeOpenOrder {
    pub client_order_id: Option<ClientOrderId>,
    pub venue_order_id: VenueOrderId,
    pub order: DesiredOrder,
    pub filled_quantity: QuantityLots,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoritativeVenueSnapshot {
    pub committed_sequence: u64,
    pub open_orders: Vec<AuthoritativeOpenOrder>,
    pub positions: Vec<AuthoritativePosition>,
}
