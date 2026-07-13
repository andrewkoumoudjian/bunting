#![forbid(unsafe_code)]
//! Development-only translated NBC matcher evidence.

mod matching;
#[doc(inline)]
pub use matching::{
    CancelOutcome, Fill, MAX_OPEN_ORDERS, MatchError, MatchOutcome, NBC_EXTERNAL_LOT_SIZE,
    NbcOrderBook, OpenOrder,
};

// Rust guideline compliant 2026-02-21
