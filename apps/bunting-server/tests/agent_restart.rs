//! Built-in agents survive a hard crash (Step 4): the venue process is
//! killed while its agent quotes, restarted on the same files, and the agent
//! resumes from its checkpoint — still knowing the fill a team caused —
//! without re-committing or colliding with anything it committed before.

mod support;

use bunting_market_types::RunId;
use bunting_origin_store::JournalInput;
use bunting_rs::CompetitionArchive;
use bunting_server::config::{ScenarioConfig, ServerConfig, StorageKind};
use bunting_server::storage::read_run_journal;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use support::{Client, Levels, TIMEOUT, book, order, wait_for_book};

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

/// A file-backed venue with one inventory-skewed agent ticking every 20 ms.
fn write_config(folder: &Path, port: u16) -> Result<PathBuf, String> {
    let scenario = folder.join("scenario.json");
    std::fs::write(&scenario, include_str!("../config/scenario.json"))
        .map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.storage.kind = StorageKind::File;
    config.storage.path = Some(folder.join("origin.json").display().to_string());
    config.scenario = Some(ScenarioConfig {
        path: scenario.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let runtime = config.runtime.as_mut().ok_or("local profile has agents")?;
    runtime.wall_tick_ms = 20;
    runtime.scheduler.agents[0].kind =
        serde_json::from_value(serde_json::json!("inventory_skewed_market_maker"))
            .map_err(|error| error.to_string())?;
    config.fix.as_mut().ok_or("local profile has FIX")?.bind = format!("127.0.0.1:{port}");
    let path = folder.join("server.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path)
}

fn start(config: &Path, port: u16) -> Result<Server, String> {
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_bunting-server"))
            .arg(config)
            .spawn()
            .map_err(|error| error.to_string())?,
    );
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if let Ok(Some(status)) = server.0.try_wait() {
            return Err(format!("server exited during startup: {status}"));
        }
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(server)
}

fn quantity_at(levels: &Levels, price: i64) -> i64 {
    levels
        .iter()
        .find(|(level, _)| *level == price)
        .map_or(0, |(_, quantity)| *quantity)
}

#[test]
fn agents_resume_after_the_venue_is_killed_mid_run() -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let folder = Folder(std::env::temp_dir().join(format!(
        "bunting-agent-restart-{}-{nanos}",
        std::process::id()
    )));
    std::fs::create_dir_all(&folder.0).map_err(|error| error.to_string())?;
    let port = free_port()?;
    let config = write_config(&folder.0, port)?;

    let mut server = start(&config, port)?;
    let mut team = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    wait_for_book(&mut team, |bids, asks| {
        bids.contains(&98) && asks.contains(&102)
    })?;
    team.send(order("801", "buy", 5, Some(102)))?;
    team.wait_report("801", "F")?;
    // Short, the agent offers at 104; let it keep quoting, then kill the
    // process without warning while it does.
    wait_for_book(&mut team, |_, asks| asks.contains(&104))?;
    std::thread::sleep(Duration::from_millis(150));
    drop(team);
    server.0.kill().map_err(|error| error.to_string())?;
    server.0.wait().map_err(|error| error.to_string())?;
    assert!(folder.0.join("origin.agents.json").exists());

    // Same files, new process: the agent must resume (a fresh agent is
    // refused on a run it already traded in) and keep quoting short.
    let mut server = start(&config, port)?;
    let mut observer = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    let (_, before) = book(&mut observer)?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(Some(status)) = server.0.try_wait() {
            return Err(format!("restarted venue exited: {status}"));
        }
        let (_, asks) = book(&mut observer)?;
        if quantity_at(&asks, 104) > quantity_at(&before, 104) {
            break;
        }
        if Instant::now() > deadline {
            return Err(format!(
                "the resumed agent added no offers at 104: before {before:?}, now {asks:?}"
            ));
        }
    }
    // Still running: nothing it resumed collided with the journal.
    std::thread::sleep(Duration::from_millis(200));
    assert!(matches!(server.0.try_wait(), Ok(None)));
    drop(observer);
    drop(server);

    // The whole history across the crash exports as a version 2 archive
    // that replays from genesis: team orders with their admission records
    // and the agent's commands from both processes.
    let (genesis, records) = read_run_journal(&folder.0.join("origin.json"), RunId::new(1))
        .map_err(|error| error.to_string())?;
    let archive =
        CompetitionArchive::from_journal(genesis, records).map_err(|error| error.to_string())?;
    let decoded = CompetitionArchive::from_json(&archive.to_json().map_err(|e| e.to_string())?)
        .map_err(|error| error.to_string())?;
    let replay = decoded.replay().map_err(|error| error.to_string())?;
    assert_eq!(replay.final_state_hash, archive.final_state_hash);
    assert!(
        archive
            .records
            .iter()
            .any(|record| record.admission.is_some())
    );
    let actors = archive
        .records
        .iter()
        .filter_map(|record| match &record.input {
            JournalInput::Command(command) => Some(command.actor),
            JournalInput::Simulation(_) => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        actors.len() >= 2,
        "expected team and agent commands: {actors:?}"
    );
    Ok(())
}
