#![forbid(unsafe_code)]

use bunting_server::config::ServerConfig;
use std::path::Path;

fn main() {
    if let Err(error) = execute() {
        eprintln!("bunting-server: {error}");
        std::process::exit(2);
    }
}

fn execute() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| "usage: bunting-server <configuration.json>".to_owned())?;
    let config = ServerConfig::from_file(Path::new(&path)).map_err(|error| error.to_string())?;
    bunting_server::runtime::run(&config)
}
