//! A FIX client that stops reading must not keep its session, and so its
//! own reconnect, blocked until the kernel abandons the connection: a venue
//! write that makes no progress for `write_timeout_ms` ends the session.
#![cfg(unix)]

mod support;

use bunting_admission_sequencer::{LatencyMap, PathLatency};
use bunting_server::config::{AdmissionConfig, ScenarioConfig, ServerConfig};
use simfix_wire::FixMessage;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT, order};

/// Short, so the test is quick; heartbeats stay at 30 s, so only the write
/// timeout can end the stuck session within the test.
const WRITE_TIMEOUT_MS: u64 = 500;

fn start_server() -> Result<u16, String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    scenario["participants"]["2"]["initial_positions"] = serde_json::json!({ "1": 1_000 });
    let path = std::env::temp_dir().join(format!("bunting-slow-consumer-{port}.json"));
    std::fs::write(&path, scenario.to_string()).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: path.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.write_timeout_ms = WRITE_TIMEOUT_MS;
    fix.rate_limit_window_ms = 1;
    fix.max_messages_per_interval = 100_000;
    let mut admission = AdmissionConfig::with_map(LatencyMap {
        local: PathLatency {
            latency_us: 1_000,
            jitter_us: 0,
        },
        ..LatencyMap::default()
    });
    admission.max_admission_queue = 100_000;
    admission.max_outbound_hold = 100_000;
    fix.admission = admission;
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(port)
}

/// A loopback connection whose receive buffer is a few kilobytes from the
/// handshake on, so the venue's writes stall after little data.
fn small_window(port: u16) -> std::io::Result<TcpStream> {
    use rustix::net::{AddressFamily, SocketType, ipproto, sockopt};
    let socket = rustix::net::socket(AddressFamily::INET, SocketType::STREAM, Some(ipproto::TCP))?;
    sockopt::set_socket_recv_buffer_size(&socket, 4_096)?;
    rustix::net::connect(&socket, &std::net::SocketAddr::from(([127, 0, 0, 1], port)))?;
    Ok(TcpStream::from(socket))
}

/// A full-depth snapshot request for venue 1's book.
fn snapshot_request(index: usize) -> FixMessage {
    let mut message = FixMessage::new("V");
    for (tag, value) in [
        (262, format!("flood-{index}")),
        (263, "0".to_owned()),
        (264, "0".to_owned()),
        (267, "2".to_owned()),
        (269, "0".to_owned()),
        (269, "1".to_owned()),
        (48, "1".to_owned()),
        (207, "1".to_owned()),
    ] {
        message.push(tag, value);
    }
    message
}

#[test]
fn a_client_that_stops_reading_loses_its_session_and_can_log_on_again() -> Result<(), String> {
    let port = start_server()?;
    // Seventy levels make every snapshot a few kilobytes, within the
    // 256-field message limit.
    let mut maker = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    for index in 0..35 {
        maker.send(order(
            &(1_000 + index).to_string(),
            "sell",
            1,
            Some(200 + index),
        ))?;
        maker.send(order(
            &(2_000 + index).to_string(),
            "buy",
            1,
            Some(100 + index),
        ))?;
    }
    maker.wait_report("1034", "0")?;
    maker.wait_report("2034", "0")?;
    // The orders rest after the maker leaves, and its connection slot frees.
    drop(maker);
    // Team 1 asks for far more than its socket buffers hold, then never
    // reads again and never closes: a client that vanished without a FIN.
    let mut silent = Client::logon_on(
        small_window(port).map_err(|error| error.to_string())?,
        "HUMAN",
        "participant",
        "bunting-local-dev",
    )?;
    // The snapshot itself is deliverable: only the reader is at fault.
    silent.send(snapshot_request(0))?;
    silent.pump_until(|client| {
        client
            .received
            .iter()
            .any(|message| message.msg_type == "W")
    })?;
    // Ask for about twice what the venue's send buffer holds (Linux grows
    // it to 4 MiB, about 2,800 of these snapshots). The requests are small
    // and the venue reads them all, so these sends never block (the timeout
    // guards the test); they fail once the venue has closed the session.
    silent
        .stream
        .set_write_timeout(Some(TIMEOUT))
        .map_err(|error| error.to_string())?;
    for index in 1..6_000 {
        if silent.send(snapshot_request(index)).is_err() {
            break;
        }
    }
    // Team 1 reconnects. Its old session is stuck writing until the write
    // times out; then the new logon is accepted.
    let deadline = Instant::now() + TIMEOUT * 3;
    let reconnected = loop {
        match Client::logon(port, "HUMAN", "participant", "bunting-local-dev") {
            Ok(client) => break client,
            Err(error) if Instant::now() < deadline => {
                eprintln!("reconnect refused for now: {error}");
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(error) => return Err(format!("the stuck session was never released: {error}")),
        }
    };
    drop(silent);
    let mut reconnected = reconnected;
    reconnected.send(order("1", "buy", 1, Some(200)))?;
    reconnected.wait_report("1", "F")?;
    Ok(())
}
