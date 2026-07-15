// Rust guideline compliant 2026-02-21

use crate::{
    config::{ConnectionProfile, FIX_PROFILE_VERSION},
    transport::{self, BoxedFixStream},
};
use simfix_session::{ConnectionState, FixSession, SessionAction, SessionConfig, SessionSnapshot};
use simfix_wire::{Field, FixMessage, WireLimits};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io,
};
use time::{OffsetDateTime, macros::format_description};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    time::{Duration, timeout},
};

pub const MAX_FIX_LOGS: usize = 256;
pub const MAX_EXECUTIONS: usize = 128;
// Retain enough bounded FIX snapshots for the chart's zoomed-out time window.
pub const MAX_PRICE_SAMPLES: usize = 480;
pub const MAX_BOOK_LEVELS: usize = 64;

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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Portfolio {
    pub position: i64,
    pub cash: i128,
    pub bought: i64,
    pub sold: i64,
    pub last_fill_price: Option<i64>,
}

impl Portfolio {
    fn apply_fill(&mut self, side: &str, quantity: i64, price: i64) {
        let notional = i128::from(quantity).saturating_mul(i128::from(price));
        if side == "1" {
            self.position = self.position.saturating_add(quantity);
            self.bought = self.bought.saturating_add(quantity);
            self.cash = self.cash.saturating_sub(notional);
        } else if side == "2" {
            self.position = self.position.saturating_sub(quantity);
            self.sold = self.sold.saturating_add(quantity);
            self.cash = self.cash.saturating_add(notional);
        }
        self.last_fill_price = Some(price);
    }

    pub fn marked_value(self, mark: i64) -> i128 {
        self.cash
            .saturating_add(i128::from(self.position).saturating_mul(i128::from(mark)))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PriceSample {
    pub bid: i64,
    pub ask: i64,
    pub bid_quantity: i64,
    pub ask_quantity: i64,
}

pub struct FixClient {
    stream: Option<BoxedFixStream>,
    session: FixSession,
    profile: ConnectionProfile,
    credential_override: Option<String>,
    pub profile_name: String,
    pub logs: VecDeque<String>,
    pub executions: VecDeque<Execution>,
    pub prices: VecDeque<PriceSample>,
    pub book: Book,
    pub portfolio: Portfolio,
    pub status: String,
    pub book_sequence: String,
    pub committed_sequence: String,
    pub stale: bool,
    pub reset_reason: Option<String>,
    pub reconnect_attempts: u64,
    pub observed_message_types: BTreeSet<String>,
    recovery_request_pending: bool,
    order_sides: BTreeMap<String, String>,
}

impl FixClient {
    pub fn profile(&self) -> &ConnectionProfile {
        &self.profile
    }

    pub fn new(
        profile_name: String,
        profile: ConnectionProfile,
        credential_override: Option<String>,
    ) -> io::Result<Self> {
        profile
            .validate()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let config = profile_session_config(&profile, credential_override.as_deref(), false)?;
        let session = FixSession::try_new(config).map_err(|error| session_error(&error))?;
        Ok(Self {
            stream: None,
            session,
            profile,
            credential_override,
            profile_name,
            logs: VecDeque::new(),
            executions: VecDeque::new(),
            prices: VecDeque::new(),
            book: Book::default(),
            portfolio: Portfolio::default(),
            status: "disconnected; press R to connect".to_owned(),
            book_sequence: "-".to_owned(),
            committed_sequence: "-".to_owned(),
            stale: true,
            reset_reason: None,
            reconnect_attempts: 0,
            observed_message_types: BTreeSet::new(),
            recovery_request_pending: true,
            order_sides: BTreeMap::new(),
        })
    }

    pub async fn reconnect(&mut self) -> io::Result<()> {
        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
        self.status = format!(
            "connecting {} via {} (attempt {})",
            self.profile.endpoint,
            self.profile.transport.label(),
            self.reconnect_attempts
        );
        let stream = match transport::connect(&self.profile.endpoint, &self.profile.transport).await
        {
            Ok(stream) => stream,
            Err(error) => {
                self.mark_disconnected(&format!("connect failed: {error}"))?;
                return Err(error);
            }
        };
        self.stream = Some(stream);
        self.recovery_request_pending = true;
        "TCP/TLS connected; awaiting FIX Logon".clone_into(&mut self.status);
        let actions = self
            .session
            .connected_at(&timestamp(), now_millis())
            .map_err(|error| session_error(&error))?;
        self.apply(actions).await
    }

    pub fn connection_state(&self) -> ConnectionState {
        self.session.snapshot().state
    }

    pub async fn send(&mut self, message: FixMessage) -> io::Result<()> {
        if self.stream.is_none() {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "FIX session is disconnected; reconnect before sending",
            ));
        }
        let actions = self
            .session
            .send_application(message, &timestamp())
            .map_err(|error| session_error(&error))?;
        self.remember_order(&actions);
        self.apply(actions).await
    }

    fn remember_order(&mut self, actions: &[SessionAction]) {
        for action in actions {
            let SessionAction::Send(frame) = action else {
                continue;
            };
            let text = String::from_utf8_lossy(frame);
            let fields = text
                .split('\u{1}')
                .filter_map(|field| field.split_once('='));
            let values = fields.collect::<BTreeMap<_, _>>();
            match values.get("35").copied() {
                Some("D") => {
                    if let (Some(id), Some(side)) = (values.get("11"), values.get("54")) {
                        self.order_sides
                            .insert((*id).to_owned(), (*side).to_owned());
                    }
                }
                Some("G") => {
                    if let (Some(new_id), Some(old_id)) = (values.get("11"), values.get("41"))
                        && let Some(side) = self.order_sides.get(*old_id).cloned()
                    {
                        self.order_sides.insert((*new_id).to_owned(), side);
                    }
                }
                _ => {}
            }
        }
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
        while let Some(stream) = self.stream.as_mut() {
            let Ok(read_result) = timeout(Duration::from_millis(1), stream.read(&mut bytes)).await
            else {
                break;
            };
            match read_result {
                Ok(0) => {
                    self.mark_disconnected("peer closed the connection")?;
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
                Err(error) => {
                    self.mark_disconnected(&format!("transport read failed: {error}"))?;
                    return Err(error);
                }
            }
        }
        if self.stream.is_none() {
            return Ok(());
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
                    let Some(stream) = self.stream.as_mut() else {
                        return Err(io::Error::new(
                            io::ErrorKind::NotConnected,
                            "session attempted to send on a disconnected transport",
                        ));
                    };
                    stream.write_all(&frame).await?;
                }
                SessionAction::Application(message) => self.observe(&message),
                SessionAction::Disconnect => {
                    self.mark_disconnected("FIX peer requested disconnect")?;
                }
                SessionAction::Persist(_) => {}
            }
        }
        if self.connection_state() == ConnectionState::Established {
            "FIX established".clone_into(&mut self.status);
            self.stale = false;
        }
        Ok(())
    }

    fn mark_disconnected(&mut self, reason: &str) -> io::Result<()> {
        self.stream = None;
        self.stale = true;
        self.status = format!("disconnected: {reason}; press R to reconnect");
        let mut snapshot = self.session.snapshot();
        snapshot.state = ConnectionState::Disconnected;
        snapshot.outstanding_test_request = None;
        self.session = FixSession::restore(
            profile_session_config(&self.profile, self.credential_override.as_deref(), false)?,
            snapshot,
        )
        .map_err(|error| session_error(&error))?;
        Ok(())
    }

    pub async fn reset_and_reconnect(&mut self) -> io::Result<()> {
        if !self.profile.allow_sequence_reset {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "profile does not authorize ResetSeqNumFlag; set allow_sequence_reset explicitly",
            ));
        }
        self.stream = None;
        self.session = FixSession::try_new(profile_session_config(
            &self.profile,
            self.credential_override.as_deref(),
            true,
        )?)
        .map_err(|error| session_error(&error))?;
        self.book = Book::default();
        self.stale = true;
        self.reset_reason = Some("operator requested authorized FIX sequence reset".to_owned());
        self.reconnect().await
    }

    pub fn session_snapshot(&self) -> SessionSnapshot {
        self.session.snapshot()
    }

    pub fn take_recovery_request(&mut self) -> bool {
        std::mem::take(&mut self.recovery_request_pending)
    }

    fn observe(&mut self, message: &FixMessage) {
        self.observed_message_types.insert(message.msg_type.clone());
        if let Some(sequence) = message.value(10010) {
            sequence.clone_into(&mut self.committed_sequence);
        }
        match message.msg_type.as_str() {
            "W" => {
                self.book = parse_book(message);
                if let Some(sample) = price_sample(&self.book) {
                    if self.prices.len() == MAX_PRICE_SAMPLES {
                        self.prices.pop_front();
                    }
                    self.prices.push_back(sample);
                }
                message
                    .value(34)
                    .unwrap_or("?")
                    .clone_into(&mut self.book_sequence);
                self.status = format!("book sequence {}", self.book_sequence);
                self.stale = false;
                self.reset_reason = None;
                self.recovery_request_pending = false;
            }
            "X" => {
                apply_incremental(&mut self.book, message);
                if let Some(sample) = price_sample(&self.book) {
                    push_bounded(&mut self.prices, sample, MAX_PRICE_SAMPLES);
                }
                self.stale = false;
                self.status = format!(
                    "incremental book update sequence {}",
                    self.committed_sequence
                );
            }
            "UC" => {
                self.book = Book::default();
                self.stale = true;
                self.reset_reason = Some(
                    message
                        .value(10015)
                        .unwrap_or("server requested snapshot recovery")
                        .to_owned(),
                );
                self.recovery_request_pending = true;
                self.status = format!(
                    "market reset required: {}",
                    self.reset_reason.as_deref().unwrap_or("unknown reason")
                );
            }
            "8" => {
                if let (Some(order_id), Some(quantity), Some(price)) = (
                    message.value(37),
                    message
                        .value(32)
                        .and_then(|value| value.parse::<i64>().ok()),
                    message
                        .value(31)
                        .and_then(|value| value.parse::<i64>().ok()),
                ) && let Some(side) = self.order_sides.get(order_id)
                {
                    self.portfolio.apply_fill(side, quantity, price);
                }
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
        let readable = String::from_utf8_lossy(frame).replace('\u{1}', "|");
        self.logs
            .push_back(format!("{direction} {}", redact_fix(&readable)));
    }
}

fn push_bounded<T>(queue: &mut VecDeque<T>, value: T, maximum: usize) {
    if queue.len() == maximum {
        queue.pop_front();
    }
    queue.push_back(value);
}

fn redact_fix(frame: &str) -> String {
    frame
        .split('|')
        .map(|field| {
            if field.starts_with("554=") {
                "554=<redacted>"
            } else {
                field
            }
        })
        .collect::<Vec<_>>()
        .join("|")
}

fn apply_incremental(book: &mut Book, message: &FixMessage) {
    let mut action = "1";
    let mut side = None;
    let mut price = None;
    for field in &message.fields {
        match field.tag {
            279 => action = field.value.as_str(),
            269 => side = Some(field.value.as_str()),
            270 => price = field.value.parse::<i64>().ok(),
            271 => {
                if let (Some(side), Some(price), Ok(quantity)) =
                    (side, price.take(), field.value.parse::<i64>())
                {
                    let levels = if side == "0" {
                        &mut book.bids
                    } else if side == "1" {
                        &mut book.asks
                    } else {
                        continue;
                    };
                    levels.retain(|level| level.0 != price);
                    if action != "2" && quantity > 0 {
                        levels.push((price, quantity));
                    }
                }
            }
            _ => {}
        }
    }
    book.bids
        .sort_unstable_by(|left, right| right.0.cmp(&left.0));
    book.asks.sort_unstable_by_key(|level| level.0);
    book.bids.truncate(MAX_BOOK_LEVELS);
    book.asks.truncate(MAX_BOOK_LEVELS);
}

pub fn session_config(sender: &str, target: &str) -> SessionConfig {
    SessionConfig {
        sender_comp_id: sender.to_owned(),
        target_comp_id: target.to_owned(),
        heartbeat_seconds: 30,
        max_journal_messages: 512,
        max_pending_inbound: 64,
        wire_limits: WireLimits::default(),
        logon_fields: Vec::new(),
    }
}

fn profile_session_config(
    profile: &ConnectionProfile,
    credential_override: Option<&str>,
    reset: bool,
) -> io::Result<SessionConfig> {
    let password = credential_override
        .map_or_else(|| profile.password(), |password| Ok(password.to_owned()))
        .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, error))?;
    let mut logon_fields = vec![
        Field::new(553, profile.username.clone()),
        Field::new(554, password),
        Field::new(10000, FIX_PROFILE_VERSION),
        Field::new(10004, profile.role.as_str()),
    ];
    if let Some(run_id) = &profile.run_id {
        logon_fields.push(Field::new(10001, run_id.clone()));
    }
    if let Some(team_id) = &profile.team_id {
        logon_fields.push(Field::new(10005, team_id.clone()));
    }
    if reset {
        logon_fields.push(Field::new(141, "Y"));
    }
    Ok(SessionConfig {
        sender_comp_id: profile.sender_comp_id.clone(),
        target_comp_id: profile.target_comp_id.clone(),
        heartbeat_seconds: profile.heartbeat_seconds,
        max_journal_messages: 512,
        max_pending_inbound: 64,
        wire_limits: WireLimits::default(),
        logon_fields,
    })
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
                    && quantity > 0
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
    book.bids
        .sort_unstable_by(|left, right| right.0.cmp(&left.0));
    book.asks.sort_unstable_by_key(|level| level.0);
    book.bids.truncate(MAX_BOOK_LEVELS);
    book.asks.truncate(MAX_BOOK_LEVELS);
    book
}

fn price_sample(book: &Book) -> Option<PriceSample> {
    let bid = book.bids.first()?.0;
    let ask = book.asks.first()?.0;
    let bid_quantity = book
        .bids
        .iter()
        .try_fold(0_i64, |total, level| total.checked_add(level.1))?;
    let ask_quantity = book
        .asks
        .iter()
        .try_fold(0_i64, |total, level| total.checked_add(level.1))?;
    Some(PriceSample {
        bid,
        ask,
        bid_quantity,
        ask_quantity,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
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
                bid_quantity: 1,
                ask_quantity: 1,
            });
        }
        assert_eq!(prices.len(), MAX_PRICE_SAMPLES);
        assert_eq!(prices.front().map(|sample| sample.bid), Some(1));
    }

    #[test]
    fn snapshot_levels_are_sorted_and_sampled_with_bounded_depth() {
        let mut message = FixMessage::new("W");
        for (tag, value) in [
            (269, "0"),
            (270, "98"),
            (271, "4"),
            (269, "1"),
            (270, "103"),
            (271, "7"),
            (269, "0"),
            (270, "99"),
            (271, "6"),
            (269, "1"),
            (270, "101"),
            (271, "3"),
        ] {
            message.push(tag, value);
        }
        let book = parse_book(&message);
        assert_eq!(book.bids, vec![(99, 6), (98, 4)]);
        assert_eq!(book.asks, vec![(101, 3), (103, 7)]);
        assert_eq!(
            price_sample(&book),
            Some(PriceSample {
                bid: 99,
                ask: 101,
                bid_quantity: 10,
                ask_quantity: 10,
            })
        );
    }

    #[test]
    fn portfolio_projects_buy_and_sell_fills() {
        let mut portfolio = Portfolio::default();
        portfolio.apply_fill("1", 5, 100);
        portfolio.apply_fill("2", 2, 103);

        assert_eq!(portfolio.position, 3);
        assert_eq!(portfolio.cash, -294);
        assert_eq!(portfolio.bought, 5);
        assert_eq!(portfolio.sold, 2);
        assert_eq!(portfolio.marked_value(101), 9);
    }

    #[test]
    fn incremental_updates_use_absolute_quantity_and_delete_semantics() {
        let mut book = Book {
            bids: vec![(99, 4)],
            asks: vec![(101, 7)],
        };
        let mut update = FixMessage::new("X");
        for (tag, value) in [
            (279, "1"),
            (269, "0"),
            (270, "99"),
            (271, "9"),
            (279, "2"),
            (269, "1"),
            (270, "101"),
            (271, "0"),
        ] {
            update.push(tag, value);
        }
        apply_incremental(&mut book, &update);
        assert_eq!(book.bids, vec![(99, 9)]);
        assert!(book.asks.is_empty());
    }

    #[test]
    fn reset_marks_projection_stale_until_a_new_snapshot() {
        let profile = crate::config::TerminalConfig::default()
            .profile("local")
            .unwrap();
        let mut client =
            FixClient::new("local".to_owned(), profile, Some("test-only".to_owned())).unwrap();
        client.book.bids.push((99, 1));
        let mut reset = FixMessage::new("UC");
        reset.push(10010, "42");
        reset.push(10015, "cursor outside retention");
        client.observe(&reset);
        assert!(client.stale);
        assert!(client.book.bids.is_empty());
        assert_eq!(client.committed_sequence, "42");
        assert_eq!(
            client.reset_reason.as_deref(),
            Some("cursor outside retention")
        );
    }

    #[test]
    fn raw_fix_diagnostics_redact_passwords() {
        assert_eq!(
            redact_fix("35=A|553=student|554=secret|10000=profile|"),
            "35=A|553=student|554=<redacted>|10000=profile|"
        );
    }
}
