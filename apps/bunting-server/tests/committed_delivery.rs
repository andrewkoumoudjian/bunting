//! End-to-end FIX acceptance for committed-event delivery (exploration-note
//! Step 1): reports reach every affected participant regardless of which
//! connection caused them, and live-order limits are engine-owned.

mod support;

use bunting_server::config::{AdmissionConfig, ScenarioConfig, ServerConfig, StorageKind};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT, cancel, order};

/// Starts the real venue on a free loopback port with the checked-in local
/// scenario (participants 1 and 2 capped at 256 live orders), given 1,000
/// shares each so either side can sell; no admin listener, no built-in agents
/// and a 1 ms admission interval.
fn start_server(storage: StorageKind) -> Result<u16, String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    for participant in ["1", "2"] {
        scenario["participants"][participant]["initial_positions"] =
            serde_json::json!({ "1": 1_000 });
    }
    let path = std::env::temp_dir().join(format!("bunting-delivery-{port}.json"));
    std::fs::write(&path, scenario.to_string()).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    if storage == StorageKind::File {
        let directory = std::env::temp_dir().join(format!("bunting-delivery-{port}"));
        std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
        config.storage.kind = StorageKind::File;
        config.storage.path = Some(directory.join("origin.json").display().to_string());
    }
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: path.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.rate_limit_window_ms = 1;
    // Equalized admission with a 2 ms `D`: the full ADR 0034 path, kept
    // short so the end-to-end tests stay fast.
    fix.admission = AdmissionConfig::equalized(2_000);
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

#[test]
fn resting_maker_receives_fill_caused_by_another_connection() -> Result<(), String> {
    let port = start_server(StorageKind::Memory)?;
    let mut maker = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut taker = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;

    maker.send(order("101", "sell", 5, Some(100)))?;
    maker.wait_report("101", "0")?;

    taker.send(order("201", "buy", 5, Some(100)))?;
    taker.wait_report("201", "F")?;

    // The maker sent nothing further; its fill arrives unsolicited.
    maker.wait_report("101", "F")?;
    let fill = maker
        .reports_for("101")
        .into_iter()
        .find(|message| message.value(150) == Some("F"))
        .ok_or("maker fill missing")?;
    assert_eq!(fill.value(32), Some("5"));
    assert_eq!(fill.value(31), Some("100"));
    assert_eq!(fill.value(39), Some("2"));
    // Isolation: neither side sees the other's client order identifiers.
    assert!(maker.reports_for("201").is_empty());
    assert!(taker.reports_for("101").is_empty());
    Ok(())
}

#[test]
fn filled_orders_never_consume_the_live_order_limit() -> Result<(), String> {
    let port = start_server(StorageKind::Memory)?;
    let mut seller = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    let mut buyer = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    seller.send(order("301", "sell", 300, Some(100)))?;
    seller.wait_report("301", "0")?;
    // More fully filled orders than the old per-connection limit of 256.
    for index in 0..300 {
        let id = (1_000 + index).to_string();
        buyer.send(order(&id, "buy", 1, Some(100)))?;
        buyer.wait_report(&id, "F")?;
    }
    assert!(
        buyer
            .received
            .iter()
            .all(|message| message.value(150) != Some("8"))
    );
    Ok(())
}

#[test]
fn reconnecting_cannot_exceed_the_engine_live_order_limit() -> Result<(), String> {
    // Durable storage, as in the hosted profile: the FIX session and its
    // client-order mapping survive the reconnect.
    let port = start_server(StorageKind::File)?;
    let mut trader = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    for index in 0..256 {
        let id = (5_000 + index).to_string();
        trader.send(order(&id, "buy", 1, Some(10 + (index % 50))))?;
        trader.wait_report(&id, "0")?;
    }
    trader
        .stream
        .shutdown(std::net::Shutdown::Both)
        .map_err(|error| error.to_string())?;
    let mut trader = trader.reconnect(port)?;
    trader.send(order("9001", "buy", 1, Some(10)))?;
    trader.wait_report("9001", "8")?;
    let rejection = trader.reports_for("9001")[0];
    assert!(
        rejection
            .value(58)
            .is_some_and(|text| text.contains("MaxLiveOrders")),
        "{:?}",
        rejection.value(58)
    );

    // An order placed before the reconnect can still be cancelled, and the
    // released slot admits the next order.
    trader.send(cancel("9002", "5000"))?;
    trader.wait_report("5000", "4")?;
    trader.send(order("9003", "buy", 1, Some(10)))?;
    trader.wait_report("9003", "0")?;
    Ok(())
}
