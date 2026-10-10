#![forbid(unsafe_code)]
//! `bunting-trader`: trade on a hosted Bunting venue over the Bunting
//! Native Protocol (ADR 0040) with a participant certificate.

#[cfg(not(target_arch = "wasm32"))]
mod app;

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(error) = app::run() {
        eprintln!("bunting-trader: {error}");
        std::process::exit(1);
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}
