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
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    scenario["simulation"]["scheduled_actions"] = serde_json::json!([{
        "action_id": 1,
        "effective_at": expiry_ns,
        "kind": { "expire_instrument": { "instrument_id": 1 } }
    }]);
    start_with(&scenario)
}

fn start_with(scenario: &serde_json::Value) -> Result<(u16, u16), String> {
    let (port, admin_port) = (free_port()?, free_port()?);
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

/// Run time of the opening auction in the session test.
const OPEN_NS: u64 = 1_500_000_000;

#[test]
fn a_pre_open_collects_crossing_orders_and_the_open_fills_them_at_one_price() -> Result<(), String>
{
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    scenario["participants"]["2"]["initial_positions"] = serde_json::json!({ "1": 1_000 });
    // The run starts in venue 1's pre-open; the open is at 1.5 s.
    scenario["calendar"] = serde_json::json!({
        "day_ns": 60_000_000_000_u64,
        "days": 1,
        "sessions": [{
            "venue_id": 1,
            "pre_open_ns": 0,
            "open_ns": OPEN_NS,
            "closing_call_ns": 50_000_000_000_u64,
            "close_ns": 55_000_000_000_u64
        }]
    });
    let (port, admin_port) = start_with(&scenario)?;
    let mut buyer = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut seller = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    buyer.send(order("1", "buy", 5, Some(101)))?;
    buyer.wait_report("1", "0")?;
    seller.send(order("2", "sell", 5, Some(99)))?;
    seller.wait_report("2", "0")?;
    // Crossed, but nothing trades before the open.
    let (_, before) = run_view(admin_port)?;
    assert!(
        before < OPEN_NS,
        "orders arrived after the open ({before} ns)"
    );
    assert!(!buyer.has_report("1", "F"));
    // The venue timer opens the market at one price for both sides: 99 and
    // 101 each match 5 and are equally near the opening mark (100), so the
    // lower one.
    buyer.wait_report("1", "F")?;
    seller.wait_report("2", "F")?;
    for (client, id) in [(&buyer, "1"), (&seller, "2")] {
        let fill = client
            .reports_for(id)
            .into_iter()
            .find(|message| message.value(150) == Some("F"))
            .ok_or("fill missing")?;
        assert_eq!(fill.value(31), Some("99"));
        assert_eq!(fill.value(32), Some("5"));
    }
    Ok(())
}
