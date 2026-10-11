//! FIX session recovery across a hard crash (ADR 0045): the venue process is
//! killed with SIGKILL and restarted on the same files. A participant that
//! keeps its own FIX session state must be able to continue its sequence
//! numbers, have earlier execution reports resent, cancel orders placed
//! before the crash, and never have an order lost or committed twice.

mod support;

use bunting_market_events::CommandPayload;
use bunting_market_types::{ParticipantId, RunId};
use bunting_origin_store::JournalInput;
use bunting_server::config::{ScenarioConfig, ServerConfig, StorageKind};
use bunting_server::storage::read_run_journal;
use simfix_wire::FixMessage;
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT, TIMESTAMP, cancel, now_millis, order};

struct Folder(PathBuf);

impl Folder {
    fn new(name: &str) -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        let path =
            std::env::temp_dir().join(format!("bunting-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
        Ok(Self(path))
    }
}

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

impl Server {
    /// `SIGKILL` on Unix, `TerminateProcess` on Windows: no shutdown code runs.
    fn crash(mut self) -> Result<(), String> {
        self.0.kill().map_err(|error| error.to_string())?;
        self.0.wait().map_err(|error| error.to_string())?;
        Ok(())
    }
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

/// A file-backed venue without built-in agents, so every command is ours.
fn write_config(folder: &Path, port: u16) -> Result<PathBuf, String> {
    // Both participants hold inventory, so either side may sell.
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    for participant in ["1", "2"] {
        scenario["participants"][participant]["initial_positions"] =
            serde_json::json!({ "1": 1_000 });
    }
    let scenario_path = folder.join("scenario.json");
    std::fs::write(&scenario_path, scenario.to_string()).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: scenario_path.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    config.storage.kind = StorageKind::File;
    config.storage.path = Some(folder.join("origin.json").display().to_string());
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.max_messages_per_interval = 10_000;
    let path = folder.join("server.json");
    std::fs::write(
        &path,
        serde_json::to_vec(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(path)
}

/// Starts the venue; its diagnostics go to `server.log` beside `config`.
fn start(config: &Path, port: u16) -> Result<Server, String> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.with_file_name("server.log"))
        .map_err(|error| error.to_string())?;
    let mut server = Server(
        Command::new(env!("CARGO_BIN_EXE_bunting-server"))
            .arg(config)
            .stderr(log)
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

/// Closes `client` cleanly and requires the venue's own check that its
/// session log replays to the live state (debug builds, ADR 0045).
fn close_and_check(client: Client, folder: &Path) -> Result<(), String> {
    drop(client);
    std::thread::sleep(Duration::from_millis(300));
    let log = std::fs::read_to_string(folder.join("server.log")).unwrap_or_default();
    if log.contains("diverged") {
        return Err(format!("session log replay diverged: {log}"));
    }
    Ok(())
}

fn logon(port: u16) -> Result<Client, String> {
    Client::logon(port, "HUMAN", "participant", "bunting-local-dev")
}

/// Sends a `ResendRequest` for everything and returns the raw frames the venue
/// sends back until each wanted client order ID appears in a possible
/// duplicate `ExecutionReport`. The frames are also fed to the client session,
/// which ignores resent copies and keeps sequencing anything new.
fn resend_all(client: &mut Client, wanted: &[&str]) -> Result<Vec<FixMessage>, String> {
    let mut request = FixMessage::new("2");
    request.push(7, "1");
    request.push(16, "0");
    client.send(request)?;
    let deadline = Instant::now() + TIMEOUT;
    let mut raw = Vec::new();
    let mut buffer = [0_u8; 16_384];
    loop {
        let frames = split_frames(&raw);
        let resent = |id: &str| {
            frames.iter().any(|frame| {
                frame.msg_type == "8" && frame.value(43) == Some("Y") && frame.value(11) == Some(id)
            })
        };
        if wanted.iter().all(|id| resent(id)) {
            return Ok(frames);
        }
        if Instant::now() > deadline {
            return Err(format!(
                "resend incomplete: {:?}",
                frames
                    .iter()
                    .map(|frame| (frame.msg_type.clone(), frame.value(11), frame.value(43)))
                    .collect::<Vec<_>>()
            ));
        }
        match client.stream.read(&mut buffer) {
            Ok(0) => return Err("server closed the connection".to_owned()),
            Ok(count) => {
                raw.extend_from_slice(&buffer[..count]);
                let actions = client
                    .session
                    .receive_bytes_at(&buffer[..count], TIMESTAMP, now_millis())
                    .map_err(|error| format!("{error:?}"))?;
                client.apply(actions)?;
            }
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn split_frames(raw: &[u8]) -> Vec<FixMessage> {
    let text = String::from_utf8_lossy(raw);
    text.split("8=FIXT.1.1\u{1}")
        .filter(|frame| frame.contains("\u{1}10="))
        .map(|frame| {
            let mut message = FixMessage::new("");
            for field in frame.split('\u{1}') {
                if let Some((tag, value)) = field.split_once('=') {
                    match tag.parse::<u32>() {
                        Ok(35) => value.clone_into(&mut message.msg_type),
                        Ok(tag) => message.push(tag, value),
                        Err(_) => {}
                    }
                }
            }
            message
        })
        .collect()
}

/// Accepted order submissions per participant in the journal, by order ID.
fn submissions(origin: &Path) -> Result<BTreeMap<u128, usize>, String> {
    let (_, records) =
        read_run_journal(origin, RunId::new(1)).map_err(|error| error.to_string())?;
    let mut orders = BTreeMap::new();
    for record in records {
        if let JournalInput::Command(command) = &record.input
            && command.actor == ParticipantId::new(1)
            && let CommandPayload::SubmitOrder(order)
            | CommandPayload::SubmitOrderAtListing { order, .. } = &command.payload
        {
            *orders.entry(order.order_id.get()).or_insert(0) += 1;
        }
    }
    Ok(orders)
}

#[test]
fn sequence_numbers_resend_and_order_ids_survive_a_kill() -> Result<(), String> {
    let folder = Folder::new("fix-crash")?;
    let port = free_port()?;
    let config = write_config(&folder.0, port)?;

    let server = start(&config, port)?;
    let mut client = logon(port)?;
    for id in ["11", "12", "13"] {
        client.send(order(id, "buy", 1, Some(90)))?;
        client.wait_report(id, "0")?;
    }
    server.crash()?;

    let _server = start(&config, port)?;
    // The client continues its own sequence numbers; a venue that lost its
    // outgoing sequence would send numbers the client already saw, which the
    // client's session rejects.
    let mut client = client.reconnect(port)?;
    resend_all(&mut client, &["11", "12", "13"])?;
    // The ClOrdID-to-order mapping survived: an order placed before the
    // crash can be cancelled by its original ClOrdID.
    client.send(cancel("21", "11"))?;
    // The venue reports the cancel under the original order's ClOrdID.
    client.wait_report("11", "4")?;
    client.send(order("14", "buy", 1, Some(90)))?;
    client.wait_report("14", "0")?;
    close_and_check(client, &folder.0)
}

#[test]
fn orders_in_flight_at_a_kill_are_neither_lost_nor_committed_twice() -> Result<(), String> {
    // Kill right after sending, after a few acknowledgements, and late.
    for acknowledged in [None, Some(2), Some(30)] {
        in_flight_at_a_kill(acknowledged)?;
    }
    Ok(())
}

fn in_flight_at_a_kill(acknowledged: Option<usize>) -> Result<(), String> {
    let folder = Folder::new("fix-crash-flight")?;
    let port = free_port()?;
    let config = write_config(&folder.0, port)?;
    let ids = (101..=140).map(|id| id.to_string()).collect::<Vec<_>>();

    let server = start(&config, port)?;
    let mut client = logon(port)?;
    for id in &ids {
        client.send(order(id, "buy", 1, Some(80)))?;
    }
    if let Some(index) = acknowledged {
        client.wait_report(&ids[index], "0")?;
    }
    server.crash()?;

    let _server = start(&config, port)?;
    // Reconnecting with a higher sequence number makes the venue ask for
    // whatever it never durably received; the client resends those as
    // possible duplicates.
    let mut client = client.reconnect(port)?;
    let answered = |client: &Client, id: &str| {
        client
            .reports_for(id)
            .iter()
            .any(|report| matches!(report.value(150), Some("0" | "8")))
    };
    // Every order is answered exactly as if nothing happened: before the
    // kill, from the venue's resend, or after it from the origin.
    let all = ids.clone();
    client
        .pump_until(move |client| all.iter().all(|id| answered(client, id)))
        .map_err(|error| format!("kill after {acknowledged:?} acks: {error}"))?;
    close_and_check(client, &folder.0)?;

    let orders = submissions(&folder.0.join("origin.json"))?;
    assert_eq!(
        orders.len(),
        ids.len(),
        "kill after {acknowledged:?} acks: every order is committed: {orders:?}"
    );
    assert!(
        orders.values().all(|count| *count == 1),
        "kill after {acknowledged:?} acks: an order was committed twice: {orders:?}"
    );
    Ok(())
}

#[test]
fn a_fill_while_disconnected_is_delivered_on_reconnect() -> Result<(), String> {
    for crash in [false, true] {
        fill_while_disconnected(crash)?;
    }
    Ok(())
}

/// A maker rests an order and disconnects; a taker trades against it; the
/// maker reconnects
/// (after a venue crash and restart, with `crash`) and gets its fill.
fn fill_while_disconnected(crash: bool) -> Result<(), String> {
    let folder = Folder::new("fix-crash-fill")?;
    let port = free_port()?;
    let config = write_config(&folder.0, port)?;

    let server = start(&config, port)?;
    let mut maker = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    maker.send(order("11", "sell", 2, Some(90)))?;
    maker.wait_report("11", "0")?;
    let _ = maker.stream.shutdown(std::net::Shutdown::Both);

    let mut taker = logon(port)?;
    taker.send(order("51", "buy", 1, Some(90)))?;
    taker.wait_report("51", "F")?;
    drop(taker);
    let _server = if crash {
        server.crash()?;
        start(&config, port)?
    } else {
        server
    };

    let mut maker = maker.reconnect(port)?;
    maker
        .wait_report("11", "F")
        .map_err(|error| format!("crash {crash}: {error}"))?;
    // Exactly one fill: the catch-up never repeats a delivered report.
    maker.send(order("12", "sell", 1, Some(95)))?;
    maker.wait_report("12", "0")?;
    let fills = maker
        .reports_for("11")
        .iter()
        .filter(|report| report.value(150) == Some("F"))
        .count();
    assert_eq!(fills, 1, "crash {crash}: one fill report");
    close_and_check(maker, &folder.0)
}
