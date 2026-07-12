#![forbid(unsafe_code)]
//! WASM-safe compatibility contract for the legacy QUARCC trading engine.
//!
//! This crate preserves the public service names and wire field layout used by
//! `quarcc.v1` without inheriting the reference implementation's threads,
//! sockets, `SQLite` access, callbacks, or wall clock. Floating-point values exist
//! only at this legacy compatibility boundary; production Bunting commands must
//! convert them into checked fixed-point units before entering domain logic.

use core::fmt;
use serde::{Deserialize, Serialize};

/// Side values from `quarcc.v1.Side`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(i32)]
pub enum Side {
    /// The caller did not provide a side.
    #[default]
    Unknown = 0,
    /// Buy-side order flow.
    Buy = 1,
    /// Sell-side order flow.
    Sell = 2,
}

/// Order types from `quarcc.v1.OrderType`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(i32)]
pub enum OrderType {
    /// The caller did not provide an order type.
    #[default]
    Unknown = 0,
    /// A market order.
    Market = 1,
    /// A limit order.
    Limit = 2,
    /// A stop order.
    Stop = 3,
    /// A stop-limit order.
    StopLimit = 4,
}

/// Order states from `quarcc.v1.OrderStatus`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(i32)]
pub enum OrderStatus {
    /// A newly created order.
    #[default]
    New = 0,
    /// The order was submitted.
    Submitted = 1,
    /// The order was partially filled.
    PartialFill = 2,
    /// The order was completely filled.
    Filled = 3,
    /// The order was canceled.
    Cancelled = 4,
    /// The order was rejected.
    Rejected = 5,
}

/// Time-in-force values from `quarcc.v1.TimeInForce`.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[repr(i32)]
pub enum TimeInForce {
    /// Day order.
    #[default]
    Day = 0,
    /// Good until canceled.
    Gtc = 1,
    /// Immediate or cancel.
    Ioc = 2,
    /// Fill or kill.
    Fok = 3,
}

/// Legacy money payload.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Money {
    /// ISO-style currency code.
    pub currency: String,
    /// Legacy floating-point amount.
    pub amount: f64,
}

/// Legacy strategy submission.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StrategySignal {
    /// Strategy identifier.
    pub strategy_id: String,
    /// Venue symbol.
    pub symbol: String,
    /// Requested side.
    pub side: Side,
    /// Legacy target quantity.
    pub target_quantity: f64,
    /// Strategy confidence.
    pub confidence: f64,
    /// Caller-supplied generation timestamp.
    pub generated_at: String,
}

/// Legacy cancellation request.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelSignal {
    /// Strategy identifier.
    pub strategy_id: String,
    /// Existing order identifier.
    pub order_id: String,
    /// Caller-supplied generation timestamp.
    pub generated_at: String,
}

/// Legacy replacement request.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ReplaceSignal {
    /// Strategy identifier.
    pub strategy_id: String,
    /// Venue symbol.
    pub symbol: String,
    /// Requested side.
    pub side: Side,
    /// Legacy target quantity.
    pub target_quantity: f64,
    /// Strategy confidence.
    pub confidence: f64,
    /// Caller-supplied generation timestamp.
    pub generated_at: String,
    /// Existing order identifier.
    pub order_id: String,
}

/// Legacy order record.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Order {
    /// Order identifier.
    pub id: String,
    /// Venue symbol.
    pub symbol: String,
    /// Order side.
    pub side: Side,
    /// Legacy quantity.
    pub quantity: f64,
    /// Legacy price.
    pub price: f64,
    /// Order type.
    pub r#type: OrderType,
    /// Current status.
    pub status: OrderStatus,
    /// Time in force.
    pub time_in_force: TimeInForce,
    /// Account identifier.
    pub account_id: String,
    /// Strategy identifier.
    pub strategy_id: String,
    /// Recorded creation timestamp.
    pub created_at: String,
}

/// Legacy execution report.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ExecutionReport {
    /// Broker order identifier.
    pub broker_order_id: String,
    /// Venue symbol.
    pub symbol: String,
    /// Filled side.
    pub side: Side,
    /// Legacy filled quantity.
    pub filled_quantity: f64,
    /// Legacy average fill price.
    pub avg_fill_price: f64,
    /// Recorded fill timestamp.
    pub fill_time: String,
}

/// Legacy position query.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GetPositionRequest {
    /// Venue symbol.
    pub symbol: String,
}

/// Legacy position response.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Position {
    /// Venue symbol.
    pub symbol: String,
    /// Signed position quantity.
    pub quantity: f64,
    /// Average open price.
    pub avg_price: f64,
    /// Unrealized profit and loss.
    pub unrealized_pnl: f64,
    /// Realized profit and loss.
    pub realized_pnl: f64,
}

/// Legacy position-list response.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PositionList {
    /// All known positions.
    pub positions: Vec<Position>,
}

/// Legacy kill-switch request.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct KillSwitchRequest {
    /// Operator reason.
    pub reason: String,
    /// Operator identity.
    pub initiated_by: String,
}

/// Legacy market-data subscription.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubscribeMarketDataRequest {
    /// Strategy identifier.
    pub strategy_id: String,
}

/// Legacy tick payload.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TickEvent {
    /// Venue symbol.
    pub symbol: String,
    /// Best bid.
    pub bid: f64,
    /// Best ask.
    pub ask: f64,
    /// Last trade price.
    pub last: f64,
    /// Bid size.
    pub bid_size: f64,
    /// Ask size.
    pub ask_size: f64,
    /// Last trade size.
    pub last_size: f64,
    /// Source timestamp in nanoseconds.
    pub timestamp_ns: i64,
}

/// Legacy bar payload.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BarEvent {
    /// Venue symbol.
    pub symbol: String,
    /// Bar period.
    pub period: String,
    /// Open price.
    pub open: f64,
    /// High price.
    pub high: f64,
    /// Low price.
    pub low: f64,
    /// Close price.
    pub close: f64,
    /// Traded volume.
    pub volume: f64,
    /// Volume-weighted average price.
    pub vwap: f64,
    /// Bar-open timestamp in nanoseconds.
    pub timestamp_ns: i64,
}

/// Legacy market-data union.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "event", content = "payload")]
pub enum MarketDataEvent {
    /// Tick update.
    Tick(TickEvent),
    /// Bar update.
    Bar(BarEvent),
}

/// Common accepted/rejected response fields.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SubmitSignalResponse {
    /// Whether the engine accepted the request.
    pub accepted: bool,
    /// Assigned order identifier.
    pub order_id: String,
    /// Stable rejection text when rejected.
    pub rejection_reason: String,
    /// Recorded receipt timestamp.
    pub received_at: String,
}

/// Legacy cancellation response.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CancelOrderResponse {
    /// Whether the engine accepted the request.
    pub accepted: bool,
    /// Stable rejection text when rejected.
    pub rejection_reason: String,
    /// Recorded receipt timestamp.
    pub received_at: String,
}

/// Legacy replacement response.
pub type ReplaceOrderResponse = SubmitSignalResponse;

/// Portable service error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceError {
    code: &'static str,
    message: String,
}

impl ServiceError {
    /// Creates a stable service error.
    #[must_use]
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// Returns the stable machine-readable code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.code
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ServiceError {}

/// Transport-neutral equivalent of the legacy `ExecutionService` RPC surface.
pub trait ExecutionService {
    /// Submits one strategy signal.
    ///
    /// # Errors
    /// Returns [`ServiceError`] when validation, risk, or transport processing fails.
    fn submit_signal(
        &mut self,
        request: StrategySignal,
    ) -> Result<SubmitSignalResponse, ServiceError>;

    /// Cancels one order.
    ///
    /// # Errors
    /// Returns [`ServiceError`] when the order cannot be canceled.
    fn cancel_order(&mut self, request: CancelSignal) -> Result<CancelOrderResponse, ServiceError>;

    /// Replaces one order.
    ///
    /// # Errors
    /// Returns [`ServiceError`] when replacement validation or processing fails.
    fn replace_order(
        &mut self,
        request: ReplaceSignal,
    ) -> Result<ReplaceOrderResponse, ServiceError>;

    /// Returns one position.
    ///
    /// # Errors
    /// Returns [`ServiceError`] when the position cannot be queried.
    fn get_position(&self, request: GetPositionRequest) -> Result<Position, ServiceError>;

    /// Returns all positions.
    ///
    /// # Errors
    /// Returns [`ServiceError`] when positions cannot be queried.
    fn get_all_positions(&self) -> Result<PositionList, ServiceError>;

    /// Activates the operational kill switch.
    ///
    /// # Errors
    /// Returns [`ServiceError`] when authorization or activation fails.
    fn activate_kill_switch(&mut self, request: KillSwitchRequest) -> Result<(), ServiceError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_discriminants_preserve_v1_contract() {
        assert_eq!(Side::Buy as i32, 1);
        assert_eq!(OrderType::StopLimit as i32, 4);
        assert_eq!(TimeInForce::Gtc as i32, 1);
    }

    #[test]
    fn compatibility_types_are_send() {
        const fn assert_send<T: Send>() {}
        assert_send::<StrategySignal>();
        assert_send::<ExecutionReport>();
    }
}
