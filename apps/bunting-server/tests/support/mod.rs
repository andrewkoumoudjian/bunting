//! Shared real-socket FIX test client for the venue's end-to-end tests.
#![allow(dead_code)]

use bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION;
use simfix_session::{ConnectionState, FixSession, SessionAction, SessionConfig};
use simfix_wire::{Field, FixMessage, WireLimits};
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const TIMEOUT: Duration = Duration::from_secs(10);

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

pub const TIMESTAMP: &str = "20261009-00:00:00.000";

pub struct Client {
    pub stream: TcpStream,
    pub session: FixSession,
    pub received: Vec<FixMessage>,
}

impl Client {
    pub fn logon(port: u16, comp_id: &str, username: &str, password: &str) -> Result<Self, String> {
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
    pub fn reconnect(mut self, port: u16) -> Result<Self, String> {
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

    pub fn apply(&mut self, actions: Vec<SessionAction>) -> Result<(), String> {
        for action in actions {
            match action {
                SessionAction::Send(bytes) => self
                    .stream
                    .write_all(&bytes)
                    .map_err(|error| error.to_string())?,
                SessionAction::Application(message) => self.received.push(message),
                SessionAction::Disconnect => return Err("server disconnected".to_owned()),
                SessionAction::Persist(_)
                | SessionAction::PeerLogon(_)
                | SessionAction::TestResponse(_) => {}
            }
        }
        Ok(())
    }

    pub fn pump_until(&mut self, done: impl Fn(&Self) -> bool) -> Result<(), String> {
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

    pub fn send(&mut self, message: FixMessage) -> Result<(), String> {
        let actions = self
            .session
            .send_application(message, TIMESTAMP)
            .map_err(|error| format!("{error:?}"))?;
        self.apply(actions)
    }

    pub fn reports_for(&self, client_order_id: &str) -> Vec<&FixMessage> {
        self.received
            .iter()
            .filter(|message| message.msg_type == "8" && message.value(11) == Some(client_order_id))
            .collect()
    }

    pub fn has_report(&self, client_order_id: &str, exec_type: &str) -> bool {
        self.reports_for(client_order_id)
            .iter()
            .any(|message| message.value(150) == Some(exec_type))
    }

    pub fn wait_report(&mut self, client_order_id: &str, exec_type: &str) -> Result<(), String> {
        let id = client_order_id.to_owned();
        let kind = exec_type.to_owned();
        self.pump_until(move |client| client.has_report(&id, &kind))
    }
}

pub fn order(client_order_id: &str, side: &str, quantity: i64, price: Option<i64>) -> FixMessage {
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

pub fn cancel(client_order_id: &str, original: &str) -> FixMessage {
    let mut message = FixMessage::new("F");
    message.push(11, client_order_id);
    message.push(41, original);
    message.push(48, "1");
    message.push(54, "1");
    message
}
