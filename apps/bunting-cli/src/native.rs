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
    /// Replay and verify one deterministic competition archive.
    Replay { archive: PathBuf },
    /// Print the settled score report from a verified archive.
    Score { archive: PathBuf },
    /// Verify archives and write leaderboard.json plus leaderboard.html.
    Judge {
        #[arg(required = true)]
        archives: Vec<PathBuf>,
    },
    /// Validate the local venue configuration and installed contracts.
    Doctor { config: Option<PathBuf> },
    /// Run a participant program's self-reported conformance probe.
    Conformance {
        #[arg(long)]
        agent: String,
    },
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
        Command::Server { config } => run_server(config.as_deref()).await,
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
        Command::Replay { archive } => {
            let result = load_archive(&archive)?
                .replay()
                .map_err(|error| error.to_string())?;
            println!(
                "verified {} commands, {} events, final hash {}",
                result.command_count, result.event_count, result.final_state_hash
            );
            Ok(())
        }
        Command::Score { archive } => {
            let result = load_archive(&archive)?
                .replay()
                .map_err(|error| error.to_string())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&result.scores)
                    .map_err(|error| format!("cannot encode scores: {error}"))?
            );
            Ok(())
        }
        Command::Judge { archives } => judge(&archives),
        Command::Doctor { config } => doctor(config.as_deref()),
        Command::Conformance { agent } => conformance(&agent),
    }
}

fn doctor(path: Option<&Path>) -> Result<(), String> {
    let config = path.map_or_else(
        || Ok(ServerConfig::local_default()),
        |path| ServerConfig::from_file(path).map_err(|error| error.to_string()),
    )?;
    config.validate().map_err(|error| error.to_string())?;
    let fix = config
        .fix
        .as_ref()
        .ok_or_else(|| "configuration has no native FIX listener".to_owned())?;
    println!(
        "ok: product={} fix={} roster={} interval_ms={} max_connections={}",
        PRODUCT_CONTRACT_VERSION,
        FIX_COMPETITION_PROFILE_VERSION,
        fix.roster.len(),
        fix.matching_interval_ms,
        fix.max_connections
    );
    Ok(())
}

fn conformance(agent: &str) -> Result<(), String> {
    let output = std::process::Command::new("sh")
        .args(["-c", agent])
        .env("BUNTING_CONFORMANCE", "1")
        .output()
        .map_err(|error| format!("cannot start agent conformance command: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "agent conformance command exited with {}",
            output.status
        ));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|_| "agent conformance output is not UTF-8".to_owned())?;
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .map_err(|error| format!("agent conformance output is not JSON: {error}"))?;
    if value.get("contract").and_then(serde_json::Value::as_str) != Some("bunting.conformance.v1") {
        return Err("agent must report contract=bunting.conformance.v1".to_owned());
    }
    println!("agent conformance: ok");
    Ok(())
}

fn load_archive(path: &Path) -> Result<bunting_rs::CompetitionArchive, String> {
    let json = fs::read_to_string(path)
        .map_err(|error| format!("cannot read archive {}: {error}", path.display()))?;
    bunting_rs::CompetitionArchive::from_json(&json)
        .map_err(|error| format!("invalid archive {}: {error}", path.display()))
}

fn judge(paths: &[PathBuf]) -> Result<(), String> {
    let mut entries = Vec::new();
    for path in paths {
        let result = load_archive(path)?
            .replay()
            .map_err(|error| format!("archive {} failed replay: {error}", path.display()))?;
        entries.extend(result.scores);
    }
    entries.sort_by_key(|entry| (std::cmp::Reverse(entry.score), entry.participant_id));
    let json = serde_json::to_string_pretty(&entries)
        .map_err(|error| format!("cannot encode leaderboard: {error}"))?;
    fs::write("leaderboard.json", &json)
        .map_err(|error| format!("cannot write leaderboard.json: {error}"))?;
    let rows = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                index.saturating_add(1),
                entry.participant_id,
                entry.score
            )
        })
        .collect::<String>();
    let html = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Bunting leaderboard</title><h1>Bunting leaderboard</h1><table><thead><tr><th>Rank</th><th>Participant</th><th>Score</th></tr></thead><tbody>{rows}</tbody></table>"
    );
    fs::write("leaderboard.html", html)
        .map_err(|error| format!("cannot write leaderboard.html: {error}"))?;
    println!("wrote leaderboard.json and leaderboard.html");
    Ok(())
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

async fn run_server(path: Option<&Path>) -> Result<(), String> {
    let config = path.map_or_else(
        || Ok(ServerConfig::local_default()),
        |path| ServerConfig::from_file(path).map_err(|error| error.to_string()),
    )?;
    bunting_server::runtime::run(&config).await
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
            vec!["bunting", "replay", "archive.json"],
            vec!["bunting", "score", "archive.json"],
            vec!["bunting", "judge", "a.json", "b.json"],
            vec!["bunting", "doctor"],
            vec!["bunting", "conformance", "--agent", "python client.py"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_ok());
        }
        #[cfg(feature = "tui")]
        assert!(Cli::try_parse_from(["bunting", "tui", "--fixture"]).is_ok());
    }
}
