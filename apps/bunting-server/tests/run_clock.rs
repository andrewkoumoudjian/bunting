//! One run clock (ADR 0037 stage 1), end to end: under a paced clock the
//! venue stamps every input with run time that follows venue time, and its
//! timer applies whatever falls due with no other input.

mod support;

use bunting_admission_sequencer::{LatencyMap, PathLatency};
use bunting_server::config::{AdminConfig, AdmissionConfig, ScenarioConfig, ServerConfig};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT, order};

const TOKEN: &str = "run-clock-test-token";

/// Run time at which the timer test's scenario expires instrument 1
/// (halting it).
const EXPIRY_NS: u64 = 400_000_000;

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

/// Starts the venue with the checked-in scenario (a real-time paced clock)
/// plus one scheduled expiry at `expiry_ns`, and returns the FIX and admin
/// ports.
fn start_server(expiry_ns: u64) -> Result<(u16, u16), String> {
    let (port, admin_port) = (free_port()?, free_port()?);
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    scenario["simulation"]["scheduled_actions"] = serde_json::json!([{
        "action_id": 1,
        "effective_at": expiry_ns,
        "kind": { "expire_instrument": { "instrument_id": 1 } }
    }]);
    let path = std::env::temp_dir().join(format!("bunting-run-clock-{port}.json"));
    std::fs::write(&path, scenario.to_string()).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = Some(AdminConfig {
        bind: format!("127.0.0.1:{admin_port}"),
        bearer_token: TOKEN.to_owned(),
        max_request_bytes: 4_096,
    });
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: path.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.rate_limit_window_ms = 1;
    fix.admission = AdmissionConfig::with_map(LatencyMap {
        local: PathLatency {
            latency_us: 1_000,
            jitter_us: 0,
        },
        ..LatencyMap::default()
    });
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    // Probe the admin port with a real request: the admin host does not
    // survive a peer that closes without one.
    while TcpStream::connect(("127.0.0.1", port)).is_err() || run_view(admin_port).is_err() {
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok((port, admin_port))
}

/// The admin run view: committed sequence and run time.
fn run_view(admin_port: u16) -> Result<(u64, u64), String> {
    let mut stream =
        TcpStream::connect(("127.0.0.1", admin_port)).map_err(|error| error.to_string())?;
    // One write: the admin host reads the request with a single read.
    stream
        .write_all(
            format!("GET /admin/runs/1 HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\n\r\n")
                .as_bytes(),
        )
        .map_err(|error| error.to_string())?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| error.to_string())?;
    let body = response
        .split("\r\n\r\n")
        .nth(1)
        .ok_or("admin response has no body")?;
    let body: serde_json::Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    let number = |key: &str| {
        body[key]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| format!("admin view lacks {key}: {body}"))
    };
    Ok((number("committedSequence")?, number("runTimeNs")?))
}

#[test]
fn the_venue_timer_applies_a_scheduled_action_with_no_other_input() -> Result<(), String> {
    let (port, admin_port) = start_server(EXPIRY_NS)?;
    // No participant sends anything: the only committed input can be the
    // timer's tick, at the action's run time.
    let deadline = Instant::now() + TIMEOUT;
    let (sequence, run_time) = loop {
        let view = run_view(admin_port)?;
        if view.0 > 0 {
            break view;
        }
        if Instant::now() > deadline {
            return Err("the timer never ticked".to_owned());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(sequence, 1, "exactly one tick applies the one due action");
    assert!(run_time >= EXPIRY_NS, "ticked at {run_time} ns");
    // Applied on time: well before a second of run time had passed.
    assert!(
        run_time < EXPIRY_NS + 500_000_000,
        "ticked at {run_time} ns"
    );
    // The expiry halted the instrument, so a new order is refused.
    let mut trader = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    trader.send(order("1", "buy", 1, Some(100)))?;
    trader.wait_report("1", "8")?;
    Ok(())
}

#[test]
fn paced_inputs_are_stamped_with_run_time_that_follows_venue_time() -> Result<(), String> {
    let (port, admin_port) = start_server(60_000_000_000)?;
    let mut trader = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    trader.send(order("1", "buy", 1, Some(90)))?;
    trader.wait_report("1", "0")?;
    let (_, first) = run_view(admin_port)?;
    std::thread::sleep(Duration::from_millis(200));
    trader.send(order("2", "buy", 1, Some(91)))?;
    trader.wait_report("2", "0")?;
    let (_, second) = run_view(admin_port)?;
    // Real-time pace: at least the 200 ms the trader waited.
    assert!(
        second >= first + 200_000_000,
        "run time {first} ns then {second} ns"
    );
    Ok(())
}
