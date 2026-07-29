use bunting_api_contract::{FIX_COMPETITION_PROFILE_VERSION, PRODUCT_CONTRACT_VERSION};
use bunting_server::config::ServerConfig;
#[cfg(feature = "tui")]
use bunting_tui::TuiOptions;
use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

const LOCAL_CONFIG: &str = include_str!("../../bunting-server/config/local.json");
const HOSTED_CONFIG: &str = include_str!("../../bunting-server/config/hosted-native.json");
const SCENARIO_CONFIG: &str = include_str!("../../bunting-server/config/scenario.json");

#[derive(Debug, Parser)]
#[command(
    name = "bunting",
    about = "Bunting market simulation and exchange testing",
    disable_version_flag = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the native FIX and administration server.
    Server {
        /// Versioned server configuration JSON. Omit for an ephemeral loopback server.
        config: Option<PathBuf>,
    },
    /// Run the native FIX participant/operator terminal.
    #[cfg(feature = "tui")]
    Tui {
        #[command(flatten)]
        options: TuiOptions,
    },
    /// Install versioned configuration templates without overwriting files.
    Init {
        /// Destination directory. Defaults to the platform Bunting config directory.
        #[arg(long)]
        config_dir: Option<PathBuf>,
    },
    /// Print executable and protocol contract versions.
    Version,
}

pub async fn run() {
    if let Err(error) = execute(std::env::args_os()).await {
        eprintln!("bunting: {error}");
        std::process::exit(2);
    }
}

async fn execute(arguments: impl IntoIterator<Item = OsString>) -> Result<(), String> {
    let arguments = compatibility_arguments(arguments);
    let cli = Cli::try_parse_from(arguments).map_err(|error| error.to_string())?;
    match cli.command {
        Command::Server { config } => run_server(config.as_deref()),
        #[cfg(feature = "tui")]
        Command::Tui { options } => bunting_tui::run(options).await,
        Command::Init { config_dir } => init(config_dir.as_deref()),
        Command::Version => {
            println!(
                "bunting {}\nproduct {}\nfix {}",
                bunting_rs::PRODUCT_VERSION,
                PRODUCT_CONTRACT_VERSION,
                FIX_COMPETITION_PROFILE_VERSION
            );
            Ok(())
        }
    }
}

fn compatibility_arguments(arguments: impl IntoIterator<Item = OsString>) -> Vec<OsString> {
    let mut arguments = arguments.into_iter();
    let executable = arguments
        .next()
        .unwrap_or_else(|| OsString::from("bunting"));
    let invoked_as = Path::new(&executable)
        .file_stem()
        .and_then(|value| value.to_str());
    let compatibility_command = match invoked_as {
        Some("bunting-server") => Some("server"),
        Some("bunting-tui") => Some("tui"),
        _ => None,
    };
    let mut normalized = vec![OsString::from("bunting")];
    if let Some(command) = compatibility_command {
        normalized.push(OsString::from(command));
    }
    normalized.extend(arguments);
    normalized
}

fn run_server(path: Option<&Path>) -> Result<(), String> {
    let config = path.map_or_else(
        || Ok(ServerConfig::local_default()),
        |path| ServerConfig::from_file(path).map_err(|error| error.to_string()),
    )?;
    bunting_server::runtime::run(&config)
}

fn init(config_dir: Option<&Path>) -> Result<(), String> {
    let destination = config_dir.map_or_else(default_config_dir, Path::to_path_buf);
    fs::create_dir_all(&destination).map_err(|error| {
        format!(
            "cannot create configuration directory {}: {error}",
            destination.display()
        )
    })?;
    for (name, contents) in [
        ("local.json", LOCAL_CONFIG),
        ("hosted-native.json", HOSTED_CONFIG),
        ("scenario.json", SCENARIO_CONFIG),
    ] {
        let path = destination.join(name);
        if path.exists() {
            continue;
        }
        fs::write(&path, contents)
            .map_err(|error| format!("cannot write {}: {error}", path.display()))?;
    }
    println!("initialized {}", destination.display());
    Ok(())
}

fn default_config_dir() -> PathBuf {
    if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(root).join("bunting/server");
    }
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".bunting/server"),
        |home| PathBuf::from(home).join(".config/bunting/server"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_names_route_to_unified_subcommands() {
        assert_eq!(
            compatibility_arguments(["/usr/bin/bunting-server", "local.json"].map(OsString::from)),
            ["bunting", "server", "local.json"].map(OsString::from)
        );
        assert_eq!(
            compatibility_arguments(["bunting-tui", "--fixture"].map(OsString::from)),
            ["bunting", "tui", "--fixture"].map(OsString::from)
        );
    }

    #[test]
    fn unified_commands_parse() {
        for arguments in [
            vec!["bunting", "server", "local.json"],
            vec!["bunting", "server"],
            vec!["bunting", "init"],
            vec!["bunting", "version"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_ok());
        }
        #[cfg(feature = "tui")]
        assert!(Cli::try_parse_from(["bunting", "tui", "--fixture"]).is_ok());
    }
}
