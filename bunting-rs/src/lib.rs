#![forbid(unsafe_code)]
//! Curated, portable entry point for Bunting's stable first-party contracts.

pub use bunting_market_events::{Command, EventEnvelope};
pub use bunting_market_types::{
    EventSequence, InstrumentId, LogicalTimeNs, MoneyMinor, OrderId, ParticipantId, PriceTicks,
    QuantityLots, RunId,
};

/// Product name used in build and release metadata.
pub const PRODUCT_NAME: &str = "Bunting";

/// Version of this composition package.
pub const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");
