#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native storage, configuration, FIX acceptor, and relay adapters.

pub mod config;
pub mod relay;
pub mod runtime;
pub mod storage;

pub const SERVICE_NAME: &str = "bunting-server";
