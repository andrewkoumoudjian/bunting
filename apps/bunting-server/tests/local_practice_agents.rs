//! Every listing of the local practice venue has built-in agents (BNT and
//! QRC, on venue 1 and venue 2), so a participant's order meets a market
//! wherever it goes. This starts the real server on the practice market's
//! own agent configuration and trades QRC on venue 2 and the other books.
//! The run then replays from its journal to the same state.

mod support;

use bunting_market_types::RunId;
use bunting_origin_store::JournalInput;
use bunting_rs::CompetitionArchive;
use bunting_server::config::{ScenarioConfig, ServerConfig, StorageKind};
use bunting_server::storage::read_run_journal;
use bunting_trading_state::local_venue::practice_runtime;
use simfix_wire::FixMessage;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use support::{Client, Levels, TIMEOUT, book_levels};

const SCENARIO: &str =
    include_str!("../../../packages/bunting-trading-state/src/local/scenario.json");

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

fn write_config(folder: &Path, port: u16) -> Result<PathBuf, String> {
    let scenario = folder.join("scenario.json");
    std::fs::write(&scenario, SCENARIO).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.storage.kind = StorageKind::File;
    config.storage.path = Some(folder.join("origin.json").display().to_string());
    config.scenario = Some(ScenarioConfig {
        path: scenario.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    config.runtime =
        Some(serde_json::from_value(practice_runtime()).map_err(|error| error.to_string())?);
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

/// Bid and ask levels of one listing.
fn book_of(client: &mut Client, instrument: u32, venue: u32) -> Result<(Levels, Levels), String> {
    let id = format!("book-{instrument}-{venue}-{}", client.received.len());
    let mut message = FixMessage::new("V");
    for (tag, value) in [
        (262, id.clone()),
        (263, "0".to_owned()),
        (264, "10".to_owned()),
        (267, "2".to_owned()),
        (269, "0".to_owned()),
        (269, "1".to_owned()),
        (48, instrument.to_string()),
        (207, venue.to_string()),
    ] {
        message.push(tag, value);
    }
    client.send(message)?;
    let wanted = id.clone();
    client.pump_until(move |client| {
        client
            .received
            .iter()
            .any(|message| message.msg_type == "W" && message.value(262) == Some(&wanted))
    })?;
    client
        .received
        .iter()
        .rev()
        .find(|message| message.msg_type == "W" && message.value(262) == Some(&id))
        .map(book_levels)
        .ok_or_else(|| "snapshot missing".to_owned())
}

fn wait_for_quotes(client: &mut Client, instrument: u32, venue: u32) -> Result<Levels, String> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let (bids, asks) = book_of(client, instrument, venue)?;
        if bids.len() >= 3 && asks.len() >= 3 {
            return Ok(asks);
        }
        if Instant::now() > deadline {
            return Err(format!(
                "instrument {instrument} on venue {venue} has no market: bids {bids:?} asks {asks:?}"
            ));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn buy(id: &str, instrument: u32, venue: u32, quantity: i64, price: Option<i64>) -> FixMessage {
    let mut message = FixMessage::new("D");
    message.push(11, id);
    message.push(48, instrument.to_string());
    message.push(207, venue.to_string());
    message.push(54, "1");
    message.push(38, quantity.to_string());
    if let Some(price) = price {
        message.push(40, "2");
        message.push(44, price.to_string());
    } else {
        message.push(40, "1");
    }
    message
}

#[test]
fn a_participant_trades_every_listing_of_the_practice_venue() -> Result<(), String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let folder = Folder(std::env::temp_dir().join(format!(
        "bunting-practice-agents-{}-{nanos}",
        std::process::id()
    )));
    std::fs::create_dir_all(&folder.0).map_err(|error| error.to_string())?;
    let port = free_port()?;
    let config = write_config(&folder.0, port)?;
    let server = start(&config, port)?;
    let mut team = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;

    // QRC on venue 2 (the book that had no agents): a market buy fills.
    wait_for_quotes(&mut team, 2, 2)?;
    team.send(buy("901", 2, 2, 5, None))?;
    team.wait_report("901", "F")?;

    // A QRC limit at the touch fills within a few seconds, on both venues,
    // and so does BNT on venue 2.
    for (instrument, venue) in [(2_u32, 2_u32), (2, 1), (1, 2), (1, 1)] {
        let asks = wait_for_quotes(&mut team, instrument, venue)?;
        let touch = asks.iter().map(|(price, _)| *price).min().ok_or("no ask")?;
        let id = format!("9{instrument}{venue}");
        team.send(buy(&id, instrument, venue, 5, Some(touch)))?;
        team.wait_report(&id, "F")?;
    }
    drop(team);
    std::thread::sleep(Duration::from_millis(300));
    drop(server);

    // The whole history replays from genesis to the same state, and both
    // the team's and the agents' commands are in it.
    let (genesis, records) = read_run_journal(&folder.0.join("origin.json"), RunId::new(1))
        .map_err(|error| error.to_string())?;
    let archive =
        CompetitionArchive::from_journal(genesis, records).map_err(|error| error.to_string())?;
    let decoded = CompetitionArchive::from_json(&archive.to_json().map_err(|e| e.to_string())?)
        .map_err(|error| error.to_string())?;
    let replay = decoded.replay().map_err(|error| error.to_string())?;
    assert_eq!(replay.final_state_hash, archive.final_state_hash);
    let actors = archive
        .records
        .iter()
        .filter_map(|record| match &record.input {
            JournalInput::Command(command) => Some(command.actor.get()),
            JournalInput::Simulation(_) => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    for agent in [10_u128, 20, 28, 36] {
        assert!(actors.contains(&agent), "agent {agent} never committed");
    }
    Ok(())
}
