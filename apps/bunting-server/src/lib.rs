#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native storage, configuration, and FIX acceptor adapters.

mod acceptor;
mod admin;
pub mod config;
pub mod runtime;
mod scenario;
mod session_host;
pub mod storage;
mod writer;

pub const SERVICE_NAME: &str = "bunting-server";
