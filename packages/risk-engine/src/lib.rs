#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Pure exact pre-trade admission policy and replayable run controls.

use bunting_ledger::Ledger;
use bunting_market_events::{OrderKind, RejectCode, Side, SubmitOrder};
use bunting_market_types::{InstrumentId, ParticipantId, PriceBounds, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskLimits {
    pub max_order_quantity: QuantityLots,
    pub max_open_order_quantity: QuantityLots,
    pub max_absolute_position: QuantityLots,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RiskState {
    pub kill_switch_active: bool,
    instruments: BTreeMap<InstrumentId, PriceBounds>,
    enabled: BTreeSet<ParticipantId>,
    limits: BTreeMap<ParticipantId, RiskLimits>,
}
impl RiskState {
    #[must_use]
    pub fn new() -> Self {
        Self {
            kill_switch_active: false,
            instruments: BTreeMap::new(),
            enabled: BTreeSet::new(),
            limits: BTreeMap::new(),
        }
    }
    pub fn configure_instrument(&mut self, id: InstrumentId, bounds: PriceBounds) {
        self.instruments.insert(id, bounds);
    }
    pub fn configure_participant(&mut self, id: ParticipantId, limits: RiskLimits) {
        self.enabled.insert(id);
        self.limits.insert(id, limits);
    }
    pub fn set_enabled(&mut self, id: ParticipantId, enabled: bool) {
        if enabled {
            self.enabled.insert(id);
        } else {
            self.enabled.remove(&id);
        }
    }
    /// Returns whether a participant is enabled.
    #[must_use]
    pub fn is_enabled(&self, id: ParticipantId) -> bool {
        self.enabled.contains(&id)
    }
    pub fn check(
        &self,
        order: &SubmitOrder,
        open_quantity: QuantityLots,
        ledger: &Ledger,
        market_reservation_price: Option<PriceTicks>,
    ) -> Result<PriceTicks, RejectCode> {
        if self.kill_switch_active {
            return Err(RejectCode::KillSwitchActive);
        }
        if !self.enabled.contains(&order.participant_id) {
            return Err(RejectCode::ParticipantDisabled);
        }
        if order.quantity.get() <= 0 {
            return Err(RejectCode::InvalidQuantity);
        }
        let bounds = self
            .instruments
            .get(&order.instrument_id)
            .ok_or(RejectCode::InvalidInstrument)?;
        let price = match order.kind {
            OrderKind::Limit { price } => {
                bounds
                    .validate(price)
                    .map_err(|_| RejectCode::PriceOutOfBounds)?;
                price
            }
            OrderKind::Market => market_reservation_price.unwrap_or(PriceTicks(0)),
        };
        let limits = self
            .limits
            .get(&order.participant_id)
            .ok_or(RejectCode::ParticipantDisabled)?;
        if order.quantity > limits.max_order_quantity {
            return Err(RejectCode::MaxOrderQuantity);
        }
        let total = open_quantity
            .checked_add(order.quantity)
            .ok_or(RejectCode::ArithmeticOverflow)?;
        if total > limits.max_open_order_quantity {
            return Err(RejectCode::MaxOpenOrderQuantity);
        }
        let holding = ledger.holding(order.participant_id, order.instrument_id);
        let projected = match order.side {
            Side::Buy => holding.position.checked_add(total),
            Side::Sell => holding.position.checked_sub(total),
        }
        .ok_or(RejectCode::ArithmeticOverflow)?;
        if projected.get().unsigned_abs() > limits.max_absolute_position.get().unsigned_abs() {
            return Err(RejectCode::PositionLimit);
        }
        match order.side {
            Side::Buy => {
                let cost = bunting_market_types::MoneyMinor::checked_mul_price_quantity(
                    price,
                    order.quantity,
                )
                .map_err(|_| RejectCode::ArithmeticOverflow)?;
                if ledger
                    .available_cash(order.participant_id)
                    .ok_or(RejectCode::ArithmeticOverflow)?
                    < cost
                {
                    return Err(RejectCode::InsufficientCash);
                }
            }
            Side::Sell => {
                if ledger
                    .available_inventory(order.participant_id, order.instrument_id)
                    .ok_or(RejectCode::ArithmeticOverflow)?
                    < order.quantity
                {
                    return Err(RejectCode::InsufficientInventory);
                }
            }
        }
        Ok(price)
    }
}
impl Default for RiskState {
    fn default() -> Self {
        Self::new()
    }
}
