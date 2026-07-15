#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Pure exact pre-trade admission policy and replayable run controls.

use bunting_ledger::Ledger;
use bunting_market_events::{OrderKind, RejectCode, Side, SubmitOrder};
use bunting_market_types::{
    InstrumentId, MoneyMinor, ParticipantId, PriceBounds, PriceTicks, QuantityLots,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskLimits {
    pub max_order_quantity: QuantityLots,
    pub max_open_order_quantity: QuantityLots,
    pub max_absolute_position: QuantityLots,
}

/// Enforcement behavior for one portfolio-risk rule set.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskMode {
    HardReject,
    AllowAndPenalize,
    Warning,
}

/// Exact portfolio and grouped-risk limits.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioRiskLimits {
    pub shortable: bool,
    pub buying_power: MoneyMinor,
    pub max_gross_notional: MoneyMinor,
    pub max_net_notional: MoneyMinor,
    pub max_concentration_bps: u32,
    pub margin_requirement_bps: u32,
    pub stress_loss_limit: MoneyMinor,
    pub mode: RiskMode,
    pub gross_groups: BTreeMap<String, Vec<InstrumentId>>,
    pub net_groups: BTreeMap<String, Vec<InstrumentId>>,
}

/// Immutable exact exposure presented to portfolio risk.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PortfolioExposure {
    pub available_cash: MoneyMinor,
    pub position_after: QuantityLots,
    pub order_notional: MoneyMinor,
    pub gross_notional_after: MoneyMinor,
    pub net_notional_after: MoneyMinor,
    pub largest_position_notional: MoneyMinor,
    pub stress_loss: MoneyMinor,
}

/// Stable portfolio-risk outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortfolioRiskDecision {
    pub accepted: bool,
    pub warnings: Vec<RejectCode>,
    pub penalty: MoneyMinor,
}

/// Evaluates exact grouped exposure without mutating market or ledger state.
///
/// # Errors
/// Returns a stable rejection when the configured enforcement mode is hard.
pub fn check_portfolio(
    limits: &PortfolioRiskLimits,
    exposure: PortfolioExposure,
) -> Result<PortfolioRiskDecision, RejectCode> {
    let mut warnings = Vec::new();
    if !limits.shortable && exposure.position_after.get() < 0 {
        warnings.push(RejectCode::InsufficientInventory);
    }
    if exposure.order_notional > exposure.available_cash
        || exposure.order_notional > limits.buying_power
    {
        warnings.push(RejectCode::InsufficientCash);
    }
    if exposure.gross_notional_after > limits.max_gross_notional
        || exposure.net_notional_after.get().unsigned_abs()
            > limits.max_net_notional.get().unsigned_abs()
    {
        warnings.push(RejectCode::PositionLimit);
    }
    let concentration_bps = if exposure.gross_notional_after.get() == 0 {
        0
    } else {
        exposure
            .largest_position_notional
            .get()
            .unsigned_abs()
            .saturating_mul(10_000)
            .checked_div(exposure.gross_notional_after.get().unsigned_abs())
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(u32::MAX)
    };
    if concentration_bps > limits.max_concentration_bps
        || exposure.stress_loss > limits.stress_loss_limit
    {
        warnings.push(RejectCode::PositionLimit);
    }
    if warnings.is_empty() {
        return Ok(PortfolioRiskDecision {
            accepted: true,
            warnings,
            penalty: MoneyMinor::new(0),
        });
    }
    match limits.mode {
        RiskMode::HardReject => Err(warnings[0]),
        RiskMode::Warning => Ok(PortfolioRiskDecision {
            accepted: true,
            warnings,
            penalty: MoneyMinor::new(0),
        }),
        RiskMode::AllowAndPenalize => {
            let penalty = MoneyMinor::new(
                i128::try_from(warnings.len()).map_err(|_| RejectCode::ArithmeticOverflow)?,
            );
            Ok(PortfolioRiskDecision {
                accepted: true,
                warnings,
                penalty,
            })
        }
    }
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
            OrderKind::Limit { price }
            | OrderKind::LimitWithPolicy { price, .. }
            | OrderKind::AdvancedLimit { price, .. } => {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(mode: RiskMode) -> PortfolioRiskLimits {
        PortfolioRiskLimits {
            shortable: false,
            buying_power: MoneyMinor::new(100),
            max_gross_notional: MoneyMinor::new(200),
            max_net_notional: MoneyMinor::new(100),
            max_concentration_bps: 7_500,
            margin_requirement_bps: 2_500,
            stress_loss_limit: MoneyMinor::new(50),
            mode,
            gross_groups: BTreeMap::new(),
            net_groups: BTreeMap::new(),
        }
    }

    #[test]
    fn hard_and_penalty_modes_share_the_same_exact_breach_detection() -> Result<(), RejectCode> {
        let exposure = PortfolioExposure {
            available_cash: MoneyMinor::new(100),
            position_after: QuantityLots::new(-1),
            order_notional: MoneyMinor::new(10),
            gross_notional_after: MoneyMinor::new(10),
            net_notional_after: MoneyMinor::new(-10),
            largest_position_notional: MoneyMinor::new(10),
            stress_loss: MoneyMinor::new(1),
        };
        assert_eq!(
            check_portfolio(&limits(RiskMode::HardReject), exposure),
            Err(RejectCode::InsufficientInventory)
        );
        let allowed = check_portfolio(&limits(RiskMode::AllowAndPenalize), exposure)?;
        assert!(allowed.accepted);
        assert_eq!(allowed.penalty, MoneyMinor::new(2));
        Ok(())
    }
}
