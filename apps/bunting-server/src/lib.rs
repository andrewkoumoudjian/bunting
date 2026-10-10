#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native storage, configuration, and the FIX and BNP participant listeners.

mod acceptor;
mod admin;
mod admission;
#[cfg(not(target_arch = "wasm32"))]
mod bnp_host;
#[cfg(not(target_arch = "wasm32"))]
mod bnp_trust;
mod commit_journal;
pub mod config;
mod consolidated;
mod distributor;
mod outbound;
mod public_feed;
mod run_clock;
pub mod runtime;
mod scenario;
mod session_host;
pub mod storage;
mod tcp_rtt;
mod wake;

pub const SERVICE_NAME: &str = "bunting-server";
