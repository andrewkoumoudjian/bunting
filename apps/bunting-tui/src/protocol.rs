// Rust guideline compliant 2026-02-21

use simfix_session::{ConnectionState, FixSession, SessionAction, SessionConfig};
use simfix_wire::{FixMessage, WireLimits};
use std::{collections::VecDeque, io};
use time::{OffsetDateTime, macros::format_description};
use tokio::{io::AsyncWriteExt, net::TcpStream};

pub const MAX_FIX_LOGS: usize = 256;
pub const MAX_EXECUTIONS: usize = 128;
pub const MAX_PRICE_SAMPLES: usize = 240;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Book {
    pub bids: Vec<(i64, i64)>,
    pub asks: Vec<(i64, i64)>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Execution {
    pub order_id: String,
    pub kind: String,
    pub order_status: String,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PriceSample {
    pub bid: i64,
    pub ask: i64,
}

pub struct FixClient {
    stream: TcpStream,
    session: FixSession,
    pub logs: VecDeque<String>,
    pub executions: VecDeque<Execution>,
    pub prices: VecDeque<PriceSample>,
    pub book: Book,
    pub status: String,
    pub book_sequence: String,
}

impl FixClient {
    pub async fn connect(address: &str) -> io::Result<Self> {
        let stream = TcpStream::connect(address).await?;
        let config = session_config("HUMAN", "BUNTING");
        let session = FixSession::try_new(config).map_err(|error| session_error(&error))?;
        let mut client = Self {
            stream,
            session,
            logs: VecDeque::new(),
            executions: VecDeque::new(),
            prices: VecDeque::new(),
            book: Book::default(),
            status: "logging on".to_owned(),
            book_sequence: "-".to_owned(),
        };
        let actions = client
            .session
            .connected_at(&timestamp(), now_millis())
            .map_err(|error| session_error(&error))?;
        client.apply(actions).await?;
        Ok(client)
    }

    pub fn connection_state(&self) -> ConnectionState {
        self.session.snapshot().state
    }

    pub async fn send(&mut self, message: FixMessage) -> io::Result<()> {
        let actions = self
            .session
            .send_application(message, &timestamp())
            .map_err(|error| session_error(&error))?;
        self.apply(actions).await
    }

    pub async fn logout(&mut self) -> io::Result<()> {
        let actions = self
            .session
            .request_logout(&timestamp(), Some("operator logout"))
            .map_err(|error| session_error(&error))?;
        self.apply(actions).await
    }

    pub async fn poll(&mut self) -> io::Result<()> {
        let mut bytes = [0_u8; 16_384];
        loop {
            match self.stream.try_read(&mut bytes) {
                Ok(0) => {
                    "disconnected".clone_into(&mut self.status);
                    break;
                }
                Ok(read) => {
                    self.log("IN ", &bytes[..read]);
                    let actions = self
                        .session
                        .receive_bytes_at(&bytes[..read], &timestamp(), now_millis())
                        .map_err(|error| session_error(&error))?;
                    self.apply(actions).await?;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
        let actions = self
            .session
            .poll(now_millis(), &timestamp())
            .map_err(|error| session_error(&error))?;
        self.apply(actions).await
    }

    async fn apply(&mut self, actions: Vec<SessionAction>) -> io::Result<()> {
        for action in actions {
            match action {
                SessionAction::Send(frame) => {
                    self.log("OUT", &frame);
                    self.stream.write_all(&frame).await?;
                }
                SessionAction::Application(message) => self.observe(&message),
                SessionAction::Disconnect => {
                    "disconnected".clone_into(&mut self.status);
                }
                SessionAction::Persist(_) => {}
            }
        }
        if self.connection_state() == ConnectionState::Established {
            "FIX established".clone_into(&mut self.status);
        }
        Ok(())
    }

    fn observe(&mut self, message: &FixMessage) {
        match message.msg_type.as_str() {
            "W" => {
                self.book = parse_book(message);
                if let (Some((bid, _)), Some((ask, _))) =
                    (self.book.bids.first(), self.book.asks.first())
                {
                    if self.prices.len() == MAX_PRICE_SAMPLES {
                        self.prices.pop_front();
                    }
                    self.prices.push_back(PriceSample {
                        bid: *bid,
                        ask: *ask,
                    });
                }
                message
                    .value(34)
                    .unwrap_or("?")
                    .clone_into(&mut self.book_sequence);
                self.status = format!("book sequence {}", self.book_sequence);
            }
            "8" => {
                if self.executions.len() == MAX_EXECUTIONS {
                    self.executions.pop_front();
                }
                self.executions.push_back(Execution {
                    order_id: message.value(37).unwrap_or("?").to_owned(),
                    kind: message.value(150).unwrap_or("?").to_owned(),
                    order_status: message.value(39).unwrap_or("?").to_owned(),
                    reason: message.value(58).unwrap_or("").to_owned(),
                });
                self.status = format!(
                    "execution order={} status={} {}",
                    message.value(37).unwrap_or("?"),
                    message.value(39).unwrap_or("?"),
                    message.value(58).unwrap_or("")
                );
            }
            "9" | "j" => {
                self.status = format!("FIX reject: {}", message.value(58).unwrap_or("unknown"));
            }
            _ => {}
        }
    }

    fn log(&mut self, direction: &str, frame: &[u8]) {
        if self.logs.len() == MAX_FIX_LOGS {
            self.logs.pop_front();
        }
        self.logs.push_back(format!(
            "{direction} {}",
            String::from_utf8_lossy(frame).replace('\u{1}', "|")
        ));
    }
}

pub fn session_config(sender: &str, target: &str) -> SessionConfig {
    SessionConfig {
        sender_comp_id: sender.to_owned(),
        target_comp_id: target.to_owned(),
        heartbeat_seconds: 30,
        max_journal_messages: 512,
        max_pending_inbound: 64,
        wire_limits: WireLimits::default(),
    }
}

pub fn timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(format_description!(
            "[year][month][day]-[hour]:[minute]:[second]"
        ))
        .unwrap_or_else(|_| "19700101-00:00:00".to_owned())
}

pub fn now_millis() -> u64 {
    u64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap_or(0)
}

pub fn session_error(error: &simfix_session::SessionError) -> io::Error {
    io::Error::other(format!("FIX session error: {error:?}"))
}

pub fn new_order(id: u128, side: &str, quantity: i64, price: Option<i64>) -> FixMessage {
    let mut message = FixMessage::new("D");
    message.push(11, id.to_string());
    message.push(48, "1");
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

pub fn cancel(id: u128, replacement_id: u128) -> FixMessage {
    let mut message = FixMessage::new("F");
    message.push(11, replacement_id.to_string());
    message.push(41, id.to_string());
    message.push(48, "1");
    message.push(54, "1");
    message
}

pub fn replace(old_id: u128, new_id: u128, quantity: i64, price: i64) -> FixMessage {
    let mut message = FixMessage::new("G");
    message.push(11, new_id.to_string());
    message.push(41, old_id.to_string());
    message.push(38, quantity.to_string());
    message.push(40, "2");
    message.push(44, price.to_string());
    message
}

pub fn status(id: u128) -> FixMessage {
    let mut message = FixMessage::new("H");
    message.push(37, id.to_string());
    message
}

pub fn book_request(id: u128) -> FixMessage {
    let mut message = FixMessage::new("V");
    message.push(262, format!("book-{id}"));
    message.push(263, "1");
    message.push(264, "20");
    message.push(267, "2");
    message.push(269, "0");
    message.push(269, "1");
    message.push(48, "1");
    message
}

fn parse_book(message: &FixMessage) -> Book {
    let mut book = Book::default();
    let mut side = None;
    let mut price = None;
    for field in &message.fields {
        match field.tag {
            269 => side = Some(field.value.as_str()),
            270 => price = field.value.parse::<i64>().ok(),
            271 => {
                if let (Some(side), Some(price), Ok(quantity)) =
                    (side, price.take(), field.value.parse::<i64>())
                {
                    if side == "0" {
                        book.bids.push((price, quantity));
                    } else if side == "1" {
                        book.asks.push((price, quantity));
                    }
                }
            }
            _ => {}
        }
    }
    book
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fix_market_snapshot_levels() {
        let mut message = FixMessage::new("W");
        for (tag, value) in [
            (269, "0"),
            (270, "99"),
            (271, "4"),
            (269, "1"),
            (270, "101"),
            (271, "7"),
        ] {
            message.push(tag, value);
        }
        assert_eq!(
            parse_book(&message),
            Book {
                bids: vec![(99, 4)],
                asks: vec![(101, 7)]
            }
        );
    }

    #[test]
    fn price_history_is_bounded() {
        let mut prices = VecDeque::new();
        for price in 0..=MAX_PRICE_SAMPLES {
            if prices.len() == MAX_PRICE_SAMPLES {
                prices.pop_front();
            }
            prices.push_back(PriceSample {
                bid: i64::try_from(price).unwrap_or(i64::MAX),
                ask: i64::try_from(price.saturating_add(2)).unwrap_or(i64::MAX),
            });
        }
        assert_eq!(prices.len(), MAX_PRICE_SAMPLES);
        assert_eq!(prices.front().map(|sample| sample.bid), Some(1));
    }
}
