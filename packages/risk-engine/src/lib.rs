#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Pure, exact pre-trade admission over the authoritative ledger.
//!
//! Admission reads immutable participant limits, listing terms and the
//! committed ledger; it never mutates state. Open-order quantities come from
//! the ledger's per-instrument counters, so a check is O(log n) regardless of
//! how many orders a run has accepted.

use bunting_ledger::{Ledger, Reservation};
use bunting_market_events::{OrderKind, RejectCode, Side, SubmitOrder};
use bunting_market_types::{MoneyMinor, PriceBounds, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};

const fn cash_constrained_default() -> bool {
    true
}

/// Per-participant admission limits pinned by the scenario.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RiskLimits {
    /// Largest single order.
    pub max_order_quantity: QuantityLots,
    /// Largest open quantity per instrument across both sides and all venues.
    pub max_open_order_quantity: QuantityLots,
    /// Largest absolute worst-case position per instrument.
    pub max_absolute_position: QuantityLots,
    /// Whether buy orders must be fully funded by available cash.
    #[serde(default = "cash_constrained_default")]
    pub cash_constrained: bool,
}

impl RiskLimits {
    /// Cash-constrained limits.
    #[must_use]
    pub const fn new(
        max_order_quantity: QuantityLots,
        max_open_order_quantity: QuantityLots,
        max_absolute_position: QuantityLots,
    ) -> Self {
        Self {
            max_order_quantity,
            max_open_order_quantity,
            max_absolute_position,
            cash_constrained: true,
        }
    }
}

/// Listing facts needed to admit one order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListingTerms {
    pub price_bounds: PriceBounds,
    /// Largest per-lot fee the order can incur; reserved with buy notional.
    pub fee_bound: MoneyMinor,
}

/// Accepted admission: the price that bounds the order's cash use and the
/// reservation the ledger must hold while the order is open.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Admission {
    pub reservation_price: PriceTicks,
    pub reservation: Reservation,
}

/// Admits one order or returns a stable rejection.
///
/// `market_reservation_price` bounds a market order's cash use (the listing's
/// worst price on the order side).
pub fn admit(
    limits: &RiskLimits,
    enabled: bool,
    order: &SubmitOrder,
    listing: ListingTerms,
    ledger: &Ledger,
    market_reservation_price: Option<PriceTicks>,
) -> Result<Admission, RejectCode> {
    if !enabled {
        return Err(RejectCode::ParticipantDisabled);
    }
    if order.quantity.get() <= 0 {
        return Err(RejectCode::InvalidQuantity);
    }
    let terms = ledger
        .terms(order.instrument_id)
        .map_err(|_| RejectCode::InvalidInstrument)?;
    let price = match order.kind {
        OrderKind::Limit { price } | OrderKind::LimitWithPolicy { price, .. } => {
            listing
                .price_bounds
                .validate(price)
                .map_err(|_| RejectCode::PriceOutOfBounds)?;
            price
        }
        OrderKind::Market => market_reservation_price.ok_or(RejectCode::InsufficientLiquidity)?,
    };
    if order.quantity > limits.max_order_quantity {
        return Err(RejectCode::MaxOrderQuantity);
    }
    let position = ledger.position(order.participant_id, order.instrument_id);
    let open = position
        .open_buy
        .checked_add(position.open_sell)
        .and_then(|open| open.checked_add(order.quantity))
        .ok_or(RejectCode::ArithmeticOverflow)?;
    if open > limits.max_open_order_quantity {
        return Err(RejectCode::MaxOpenOrderQuantity);
    }
    let worst_case = match order.side {
        Side::Buy => position
            .quantity
            .checked_add(position.open_buy)
            .and_then(|value| value.checked_add(order.quantity)),
        Side::Sell => position
            .quantity
            .checked_sub(position.open_sell)
            .and_then(|value| value.checked_sub(order.quantity)),
    }
    .ok_or(RejectCode::ArithmeticOverflow)?;
    if worst_case.get().unsigned_abs() > limits.max_absolute_position.get().unsigned_abs() {
        return Err(RejectCode::PositionLimit);
    }
    let reservation = match order.side {
        Side::Buy if limits.cash_constrained => {
            let cash_per_lot = ledger
                .cash_per_lot(order.instrument_id, price, listing.fee_bound)
                .map_err(|_| RejectCode::ArithmeticOverflow)?;
            let required = cash_per_lot
                .get()
                .checked_mul(i128::from(order.quantity.get()))
                .ok_or(RejectCode::ArithmeticOverflow)?;
            let available = ledger
                .cash(order.participant_id, terms.currency)
                .available()
                .ok_or(RejectCode::ArithmeticOverflow)?;
            if available.get() < required {
                return Err(RejectCode::InsufficientCash);
            }
            Reservation {
                cash_per_lot,
                inventory: false,
            }
        }
        Side::Buy => Reservation::NONE,
        Side::Sell if terms.shortable => Reservation::NONE,
        Side::Sell => {
            if position.available().ok_or(RejectCode::ArithmeticOverflow)? < order.quantity {
                return Err(RejectCode::InsufficientInventory);
            }
            Reservation {
                cash_per_lot: MoneyMinor::new(0),
                inventory: true,
            }
        }
    };
    Ok(Admission {
        reservation_price: price,
        reservation,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use bunting_ledger::InstrumentTerms;
    use bunting_market_types::{CurrencyId, InstrumentId, OrderId, ParticipantId};

    const STOCK: InstrumentId = InstrumentId::new(1);
    const TRADER: ParticipantId = ParticipantId::new(7);

    fn ledger(shortable: bool) -> Ledger {
        let mut ledger = Ledger::new();
        ledger
            .configure_instrument(
                STOCK,
                InstrumentTerms {
                    currency: CurrencyId::new(1),
                    multiplier: 1,
                    shortable,
                },
            )
            .unwrap();
        ledger
            .post_cash(TRADER, CurrencyId::new(1), MoneyMinor::new(1_000))
            .unwrap();
        ledger
    }

    fn order(side: Side, quantity: i64, price: i64) -> SubmitOrder {
        SubmitOrder {
            order_id: OrderId::new(1),
            instrument_id: STOCK,
            participant_id: TRADER,
            side,
            quantity: QuantityLots::new(quantity),
            kind: OrderKind::Limit {
                price: PriceTicks::new(price),
            },
        }
    }

    fn listing() -> ListingTerms {
        ListingTerms {
            price_bounds: PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
            fee_bound: MoneyMinor::new(1),
        }
    }

    fn limits() -> RiskLimits {
        RiskLimits::new(
            QuantityLots::new(100),
            QuantityLots::new(150),
            QuantityLots::new(120),
        )
    }

    #[test]
    fn buy_reservation_includes_fee_bound_and_rejects_unfunded_orders() {
        let ledger = ledger(false);
        let admitted = admit(
            &limits(),
            true,
            &order(Side::Buy, 90, 10),
            listing(),
            &ledger,
            None,
        )
        .unwrap();
        assert_eq!(admitted.reservation.cash_per_lot, MoneyMinor::new(11));
        assert_eq!(
            admit(
                &limits(),
                true,
                &order(Side::Buy, 91, 10),
                listing(),
                &ledger,
                None
            ),
            Err(RejectCode::InsufficientCash)
        );
        let unconstrained = RiskLimits {
            cash_constrained: false,
            ..limits()
        };
        assert_eq!(
            admit(
                &unconstrained,
                true,
                &order(Side::Buy, 100, 10),
                listing(),
                &ledger,
                None
            )
            .unwrap()
            .reservation,
            Reservation::NONE
        );
    }

    #[test]
    fn short_sales_require_a_shortable_instrument() {
        assert_eq!(
            admit(
                &limits(),
                true,
                &order(Side::Sell, 5, 10),
                listing(),
                &ledger(false),
                None
            ),
            Err(RejectCode::InsufficientInventory)
        );
        assert_eq!(
            admit(
                &limits(),
                true,
                &order(Side::Sell, 5, 10),
                listing(),
                &ledger(true),
                None
            )
            .unwrap()
            .reservation,
            Reservation::NONE
        );
    }

    #[test]
    fn limits_use_open_quantity_and_worst_case_position() {
        let mut ledger = ledger(true);
        ledger
            .open_order(
                TRADER,
                STOCK,
                Side::Sell,
                QuantityLots::new(60),
                Reservation::NONE,
            )
            .unwrap();
        assert_eq!(
            admit(
                &limits(),
                true,
                &order(Side::Sell, 61, 10),
                listing(),
                &ledger,
                None
            ),
            Err(RejectCode::PositionLimit)
        );
        assert_eq!(
            admit(
                &limits(),
                true,
                &order(Side::Buy, 91, 1),
                listing(),
                &ledger,
                None
            ),
            Err(RejectCode::MaxOpenOrderQuantity)
        );
        assert_eq!(
            admit(
                &limits(),
                false,
                &order(Side::Buy, 1, 1),
                listing(),
                &ledger,
                None
            ),
            Err(RejectCode::ParticipantDisabled)
        );
        assert_eq!(
            admit(
                &limits(),
                true,
                &order(Side::Buy, 1, 1_001),
                listing(),
                &ledger,
                None
            ),
            Err(RejectCode::PriceOutOfBounds)
        );
    }
}
