//! End-to-end FIX acceptance for committed-event delivery (exploration-note
//! Step 1): reports reach every affected participant regardless of which
//! connection caused them, and live-order limits are engine-owned.

use bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION;
use bunting_server::config::{ScenarioConfig, ServerConfig, StorageKind};
use simfix_session::{ConnectionState, FixSession, SessionAction, SessionConfig};
use simfix_wire::{Field, FixMessage, WireLimits};
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TIMEOUT: Duration = Duration::from_secs(10);

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

const TIMESTAMP: &str = "20261009-00:00:00.000";

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
    fix.matching_interval_ms = 1;
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

struct Client {
    stream: TcpStream,
    session: FixSession,
    received: Vec<FixMessage>,
}

impl Client {
    fn logon(port: u16, comp_id: &str, username: &str, password: &str) -> Result<Self, String> {
        let stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| error.to_string())?;
        stream
            .set_read_timeout(Some(Duration::from_millis(5)))
            .map_err(|error| error.to_string())?;
        let field = |tag: u32, value: &str| Field {
            tag,
            value: value.to_owned(),
        };
        let mut session = FixSession::try_new(SessionConfig {
            sender_comp_id: comp_id.to_owned(),
            target_comp_id: "BUNTING".to_owned(),
            heartbeat_seconds: 30,
            max_journal_messages: 4_096,
            max_pending_inbound: 64,
            wire_limits: WireLimits::default(),
            logon_fields: vec![
                field(553, username),
                field(554, password),
                field(10000, FIX_COMPETITION_PROFILE_VERSION),
                field(10004, "participant"),
            ],
        })
        .map_err(|error| format!("{error:?}"))?;
        let actions = session
            .connected_at(TIMESTAMP, now_millis())
            .map_err(|error| format!("{error:?}"))?;
        let mut client = Self {
            stream,
            session,
            received: Vec::new(),
        };
        client.apply(actions)?;
        client
            .pump_until(|client| client.session.snapshot().state == ConnectionState::Established)?;
        Ok(client)
    }

    /// Reconnects with the same FIX session state, continuing its sequence
    /// numbers as a persistent FIX client does.
    fn reconnect(mut self, port: u16) -> Result<Self, String> {
        self.stream = TcpStream::connect(("127.0.0.1", port)).map_err(|error| error.to_string())?;
        self.stream
            .set_read_timeout(Some(Duration::from_millis(5)))
            .map_err(|error| error.to_string())?;
        let actions = self
            .session
            .connected_at(TIMESTAMP, now_millis())
            .map_err(|error| format!("{error:?}"))?;
        self.apply(actions)?;
        self.pump_until(|client| client.session.snapshot().state == ConnectionState::Established)?;
        Ok(self)
    }

    fn apply(&mut self, actions: Vec<SessionAction>) -> Result<(), String> {
        for action in actions {
            match action {
                SessionAction::Send(bytes) => self
                    .stream
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?,
                SessionAction::Application(message) => self.received.push(message),
                SessionAction::Disconnect => return Err("server disconnected".to_owned()),
                SessionAction::Persist(_) | SessionAction::PeerLogon(_) => {}
            }
        }
        Ok(())
    }

    fn pump_until(&mut self, done: impl Fn(&Self) -> bool) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        let mut buffer = [0_u8; 16_384];
        while !done(self) {
            if Instant::now() > deadline {
                return Err(format!(
                    "timed out; received {:?}",
                    self.received
                        .iter()
                        .map(|message| (
                            message.msg_type.clone(),
                            message.value(11),
                            message.value(150),
                            message.value(58)
                        ))
                        .collect::<Vec<_>>()
                ));
            }
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err("server closed the connection".to_owned()),
                Ok(count) => {
                    let actions = self
                        .session
                        .receive_bytes_at(&buffer[..count], TIMESTAMP, now_millis())
                        .map_err(|error| format!("{error:?}"))?;
                    self.apply(actions)?;
                }
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    }

    fn send(&mut self, message: FixMessage) -> Result<(), String> {
        let actions = self
            .session
            .send_application(message, TIMESTAMP)
            .map_err(|error| format!("{error:?}"))?;
        self.apply(actions)
    }

    fn reports_for(&self, client_order_id: &str) -> Vec<&FixMessage> {
        self.received
            .iter()
            .filter(|message| message.msg_type == "8" && message.value(11) == Some(client_order_id))
            .collect()
    }

    fn has_report(&self, client_order_id: &str, exec_type: &str) -> bool {
        self.reports_for(client_order_id)
            .iter()
            .any(|message| message.value(150) == Some(exec_type))
    }

    fn wait_report(&mut self, client_order_id: &str, exec_type: &str) -> Result<(), String> {
        let id = client_order_id.to_owned();
        let kind = exec_type.to_owned();
        self.pump_until(move |client| client.has_report(&id, &kind))
    }
}

fn order(client_order_id: &str, side: &str, quantity: i64, price: Option<i64>) -> FixMessage {
    let mut message = FixMessage::new("D");
    message.push(11, client_order_id);
    message.push(48, "1");
    message.push(207, "1");
    message.push(54, if side == "buy" { "1" } else { "2" });
    message.push(38, quantity.to_string());
    if let Some(price) = price {
        message.push(40, "2");
        message.push(44, price.to_string());
    } else {
        message.push(40, "1");
    }
    message
}

fn cancel(client_order_id: &str, original: &str) -> FixMessage {
    let mut message = FixMessage::new("F");
    message.push(11, client_order_id);
    message.push(41, original);
    message.push(48, "1");
    message.push(54, "1");
    message
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
