#![forbid(unsafe_code)]
//! Participant trading state over the Bunting Native Protocol (ADR 0047).
//!
//! Every Bunting trading app (the terminal, the desktop app) shares this
//! crate and only renders it:
//!
//! - [`Model`] is the participant's view of one venue session, rebuilt from
//!   what the venue sent: listings, each direct feed's order-by-order book,
//!   each instrument's consolidated tape, the trades they carried, orders,
//!   fills, the account, the venue's published access delay, and the
//!   connection with its client-measured round trips. It is pure: callers pass the time and send the
//!   requests it returns.
//! - [`OrderTicket`] is an order before it is sent, venue included.
//! - [`Session`] (native) keeps a `bunting_client::Client` connected and
//!   turns it into [`SessionEvent`]s.
//! - [`local_venue`] (native) starts a private `bunting server` on this
//!   machine for practice; apps connect to it exactly as to a hosted venue.

mod model;
mod ticket;

#[cfg(not(target_arch = "wasm32"))]
pub mod local_venue;
#[cfg(not(target_arch = "wasm32"))]
mod session;

pub use bunting_client;
pub use model::{
    AccessDelay, AccountView, Connection, Feed, FeedState, FillRow, Instrument, MAX_FEEDS,
    MAX_FILLS, MAX_NOTICES, MAX_ORDERS, MAX_PRINTS, Model, Notice, NoticeLevel, OrderRow,
    OrderStatus, ROUND_TRIP_WINDOW, RoundTrips, SessionEvent, Tape, TradePrint, VenueRoundTrip,
};
#[cfg(not(target_arch = "wasm32"))]
pub use session::{SendError, Session};
pub use ticket::{OrderTicket, TicketError, TicketKind, TicketTif};
