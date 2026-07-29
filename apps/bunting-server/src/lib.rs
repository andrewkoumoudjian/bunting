#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native storage, configuration, and FIX acceptor adapters.

mod acceptor;
pub mod config;
pub mod runtime;
pub mod storage;
mod writer;

pub const SERVICE_NAME: &str = "bunting-server";
