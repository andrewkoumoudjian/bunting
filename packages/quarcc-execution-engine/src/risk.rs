use crate::config::ExecutionConfig;
use crate::error::ExecutionError;
use crate::order::DesiredOrder;
use crate::positions::PositionProjection;
use bunting_market_events::Side;

/// Applies participant-side pre-submit limits.
///
/// # Errors
///
/// Returns an error when the order violates a configured limit or arithmetic
/// needed to calculate the projected position overflows.
pub fn validate_submit(
    config: &ExecutionConfig,
    order: &DesiredOrder,
    open_orders: usize,
    position: Option<&PositionProjection>,
) -> Result<(), ExecutionError> {
    if order.quantity.get() <= 0 {
        return Err(ExecutionError::InvalidQuantity);
    }
    if order.quantity > config.max_order_quantity {
        return Err(ExecutionError::PositionLimit);
    }
    if open_orders >= config.max_orders {
        return Err(ExecutionError::OpenOrderLimit);
    }
    let current = position.map_or(0, |value| value.quantity.get());
    let projected = match order.side {
        Side::Buy => current.checked_add(order.quantity.get()),
        Side::Sell => current.checked_sub(order.quantity.get()),
    }
    .ok_or(ExecutionError::ArithmeticOverflow)?;
    if projected.unsigned_abs() > config.max_absolute_position.get().unsigned_abs() {
        return Err(ExecutionError::PositionLimit);
    }
    Ok(())
}
