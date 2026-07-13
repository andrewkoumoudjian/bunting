use crate::ids::{ClientOrderId, LocalOrderId, VenueOrderId};
use crate::lifecycle::OrderLifecycle;
use bunting_market_events::{OrderKind, Side};
use bunting_market_types::{InstrumentId, ParticipantId, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DesiredOrder {
    pub client_order_id: ClientOrderId,
    pub instrument_id: InstrumentId,
    pub participant_id: ParticipantId,
    pub side: Side,
    pub quantity: QuantityLots,
    pub kind: OrderKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ManagedOrder {
    pub local_order_id: LocalOrderId,
    pub desired: DesiredOrder,
    pub venue_order_id: Option<VenueOrderId>,
    pub lifecycle: OrderLifecycle,
    pub filled_quantity: QuantityLots,
    pub average_fill_price: Option<PriceTicks>,
    pub replacement_for: Option<LocalOrderId>,
    pub pending_replace: Option<(QuantityLots, OrderKind)>,
    pub quarantine_reason: Option<String>,
}

impl ManagedOrder {
    #[must_use]
    pub fn remaining_quantity(&self) -> QuantityLots {
        QuantityLots::new(
            self.desired
                .quantity
                .get()
                .saturating_sub(self.filled_quantity.get()),
        )
    }
}
