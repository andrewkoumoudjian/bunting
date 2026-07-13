use bunting_market_events::Side;
use bunting_market_types::{InstrumentId, MoneyMinor, PriceTicks, QuantityLots};
use serde::{Deserialize, Serialize};

use crate::error::ExecutionError;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PositionProjection {
    pub quantity: QuantityLots,
    pub average_price: Option<PriceTicks>,
    pub realized_pnl: MoneyMinor,
}

impl PositionProjection {
    /// Applies one venue-confirmed fill to the participant-local projection.
    ///
    /// # Errors
    ///
    /// Returns an error for non-positive inputs or checked-arithmetic overflow.
    pub fn apply_fill(
        &mut self,
        side: Side,
        quantity: QuantityLots,
        price: PriceTicks,
    ) -> Result<(), ExecutionError> {
        if quantity.get() <= 0 || price.get() <= 0 {
            return Err(ExecutionError::InvalidQuantity);
        }
        let signed = match side {
            Side::Buy => quantity.get(),
            Side::Sell => quantity
                .get()
                .checked_neg()
                .ok_or(ExecutionError::ArithmeticOverflow)?,
        };
        let old = self.quantity.get();
        let new = old
            .checked_add(signed)
            .ok_or(ExecutionError::ArithmeticOverflow)?;
        let reducing = old != 0 && old.signum() != signed.signum();
        if reducing {
            let closed = old.unsigned_abs().min(signed.unsigned_abs());
            let average = self.average_price.map_or(price.get(), PriceTicks::get);
            let per_unit = if old > 0 {
                price.get().checked_sub(average)
            } else {
                average.checked_sub(price.get())
            }
            .ok_or(ExecutionError::ArithmeticOverflow)?;
            let pnl = i128::from(per_unit)
                .checked_mul(i128::from(closed))
                .ok_or(ExecutionError::ArithmeticOverflow)?;
            self.realized_pnl = self
                .realized_pnl
                .checked_add(MoneyMinor::new(pnl))
                .ok_or(ExecutionError::ArithmeticOverflow)?;
        }
        self.average_price = if new == 0 {
            None
        } else if old == 0 || old.signum() != new.signum() {
            Some(price)
        } else if old.signum() == signed.signum() {
            let old_notional = i128::from(old)
                .checked_mul(i128::from(
                    self.average_price.map_or(price.get(), PriceTicks::get),
                ))
                .ok_or(ExecutionError::ArithmeticOverflow)?;
            let added = i128::from(signed)
                .checked_mul(i128::from(price.get()))
                .ok_or(ExecutionError::ArithmeticOverflow)?;
            let weighted = old_notional
                .checked_add(added)
                .ok_or(ExecutionError::ArithmeticOverflow)?
                / i128::from(new);
            Some(PriceTicks::new(
                i64::try_from(weighted).map_err(|_| ExecutionError::ArithmeticOverflow)?,
            ))
        } else {
            self.average_price
        };
        self.quantity = QuantityLots::new(new);
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuthoritativePosition {
    pub instrument_id: InstrumentId,
    pub quantity: QuantityLots,
    pub average_price: Option<PriceTicks>,
    pub realized_pnl: MoneyMinor,
}
