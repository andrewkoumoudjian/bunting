#![forbid(unsafe_code)]
//! Bunting Native Protocol client (ADR 0040).
//!
//! [`Client::connect`] opens TCP with Nagle off, completes TLS 1.3 with the
//! participant's certificate, sends `Hello` (with an optional resume
//! cursor) and returns once the venue's `Welcome` arrives. A reader thread
//! then drains the socket continuously: it answers every `Probe` the moment
//! it is decrypted (the venue measures access latency from it), sends a
//! heartbeat when the connection is idle, and queues every other server
//! message for [`Client::recv_timeout`]. [`FeedBook`] rebuilds a venue's
//! book from a feed's snapshot and updates and detects gaps.
//!
//! The client is deliberately good and never compensating: nothing here
//! smooths or delays timing (ADR 0035). A slow application that stops
//! reading fills the bounded queue and then TCP, and the venue disconnects
//! it at its own bounds.

mod book;
#[cfg(not(target_arch = "wasm32"))]
mod connection;

pub use bnp_wire;
pub use book::{FeedBook, FeedError};
#[cfg(not(target_arch = "wasm32"))]
pub use connection::{Client, ClientConfig, ClientError, certificate_fingerprint};
