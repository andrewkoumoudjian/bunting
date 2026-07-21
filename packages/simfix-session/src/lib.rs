#![forbid(unsafe_code)]
//! Deterministic FIX session state behind explicit clock, store, and transport traits.

use serde::{Deserialize, Serialize};
use simfix_wire::{Decoder, FIX_50_SP2_APPL_VER_ID, Field, FixMessage, WireError, WireLimits};
use std::collections::BTreeMap;

pub trait SessionClock {
    fn now_millis(&self) -> u64;
    fn now_fix_timestamp(&self) -> String;
}

#[allow(clippy::missing_errors_doc)]
pub trait MessageStore {
    type Error;
    fn save(&mut self, sequence: u64, frame: &[u8]) -> Result<(), Self::Error>;
    fn load(&self, sequence: u64) -> Result<Option<Vec<u8>>, Self::Error>;
}

#[allow(clippy::missing_errors_doc)]
pub trait SessionTransport {
    type Error;
    fn send(&mut self, frame: &[u8]) -> Result<(), Self::Error>;
    fn disconnect(&mut self) -> Result<(), Self::Error>;
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionConfig {
    pub sender_comp_id: String,
    pub target_comp_id: String,
    pub heartbeat_seconds: u32,
    pub max_journal_messages: usize,
    pub max_pending_inbound: usize,
    pub wire_limits: WireLimits,
    /// Additional bounded Logon fields such as credentials and profile identity.
    /// Standard session header fields are rejected because the session owns them.
    #[serde(default)]
    pub logon_fields: Vec<Field>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    AwaitingLogon,
    Established,
    LogoutPending,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct JournalEntry {
    pub sequence: u64,
    pub frame: Vec<u8>,
    /// Identifies application state whose older resend copies may be gap-filled.
    #[serde(default)]
    pub replacement_key: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SessionSnapshot {
    pub version: u16,
    pub incoming_sequence: u64,
    pub outgoing_sequence: u64,
    pub reconnect_generation: u64,
    pub state: ConnectionState,
    pub journal: BTreeMap<u64, JournalEntry>,
    pub pending_resend_begin: Option<u64>,
    pub pending_inbound: BTreeMap<u64, FixMessage>,
    pub last_received_millis: u64,
    pub last_sent_millis: u64,
    pub outstanding_test_request: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionAction {
    Send(Vec<u8>),
    /// Reports the validated peer Logon after the session becomes established.
    PeerLogon(FixMessage),
    Application(FixMessage),
    Persist(SessionSnapshot),
    Disconnect,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionError {
    Wire(WireError),
    InvalidConfig,
    MissingSequence,
    InvalidSequence,
    InvalidCompId,
    InvalidApplicationVersion,
    InvalidSnapshot,
    InvalidReplacementKey,
    PendingInboundFull,
    NotEstablished,
    ArithmeticOverflow,
}

impl From<WireError> for SessionError {
    fn from(value: WireError) -> Self {
        Self::Wire(value)
    }
}

pub struct FixSession {
    config: SessionConfig,
    snapshot: SessionSnapshot,
    decoder: Decoder,
}

impl FixSession {
    /// Creates a disconnected session with sequence numbers starting at one.
    ///
    /// # Errors
    /// Returns an error when identifiers, heartbeat, or bounds are invalid.
    pub fn try_new(config: SessionConfig) -> Result<Self, SessionError> {
        validate_config(&config)?;
        Ok(Self {
            decoder: Decoder::try_new(config.wire_limits)?,
            config,
            snapshot: SessionSnapshot {
                version: 2,
                incoming_sequence: 1,
                outgoing_sequence: 1,
                reconnect_generation: 0,
                state: ConnectionState::Disconnected,
                journal: BTreeMap::new(),
                pending_resend_begin: None,
                pending_inbound: BTreeMap::new(),
                last_received_millis: 0,
                last_sent_millis: 0,
                outstanding_test_request: None,
            },
        })
    }

    /// Restores persisted FIX session state.
    ///
    /// # Errors
    /// Returns an error for an incompatible or oversized snapshot.
    pub fn restore(config: SessionConfig, snapshot: SessionSnapshot) -> Result<Self, SessionError> {
        validate_config(&config)?;
        if snapshot.version != 2
            || snapshot.journal.len() > config.max_journal_messages
            || snapshot.journal.iter().any(|(sequence, entry)| {
                entry.sequence != *sequence
                    || entry.replacement_key.as_ref().is_some_and(|key| {
                        key.is_empty() || key.len() > config.wire_limits.max_field_bytes
                    })
            })
            || snapshot.pending_inbound.len() > config.max_pending_inbound
            || snapshot.incoming_sequence == 0
            || snapshot.outgoing_sequence == 0
        {
            return Err(SessionError::InvalidSnapshot);
        }
        Ok(Self {
            decoder: Decoder::try_new(config.wire_limits)?,
            config,
            snapshot,
        })
    }

    #[must_use]
    pub fn snapshot(&self) -> SessionSnapshot {
        self.snapshot.clone()
    }

    /// Emits Logon for a newly established outbound TCP connection.
    ///
    /// # Errors
    /// Returns an error for overflow or framing.
    pub fn connected(&mut self, timestamp: &str) -> Result<Vec<SessionAction>, SessionError> {
        self.connected_at(timestamp, self.snapshot.last_received_millis)
    }

    /// Emits Logon and records the explicit host clock value.
    ///
    /// # Errors
    /// Returns an error for overflow or framing.
    pub fn connected_at(
        &mut self,
        timestamp: &str,
        now_millis: u64,
    ) -> Result<Vec<SessionAction>, SessionError> {
        self.snapshot.reconnect_generation = self
            .snapshot
            .reconnect_generation
            .checked_add(1)
            .ok_or(SessionError::ArithmeticOverflow)?;
        self.snapshot.state = ConnectionState::AwaitingLogon;
        self.snapshot.last_received_millis = now_millis;
        self.snapshot.last_sent_millis = now_millis;
        self.decoder = Decoder::try_new(self.config.wire_limits)?;
        let mut message = FixMessage::new("A");
        message.push(98, "0");
        message.push(108, self.config.heartbeat_seconds.to_string());
        message.push(1137, FIX_50_SP2_APPL_VER_ID);
        for field in &self.config.logon_fields {
            message.push(field.tag, field.value.clone());
        }
        self.send_new(message, timestamp, now_millis, true, None)
    }

    /// Decodes and applies every complete message in one socket read.
    ///
    /// # Errors
    /// Returns an error for invalid framing or sequencing.
    pub fn receive_bytes(
        &mut self,
        bytes: &[u8],
        timestamp: &str,
    ) -> Result<Vec<SessionAction>, SessionError> {
        self.receive_bytes_at(bytes, timestamp, self.snapshot.last_received_millis)
    }

    /// Decodes bytes using an explicit host clock value.
    ///
    /// # Errors
    /// Returns an error for invalid framing, identity, sequencing, or bounds.
    pub fn receive_bytes_at(
        &mut self,
        bytes: &[u8],
        timestamp: &str,
        now_millis: u64,
    ) -> Result<Vec<SessionAction>, SessionError> {
        let messages = self.decoder.push(bytes)?;
        let mut actions = Vec::new();
        for message in messages {
            actions.extend(self.receive(message, timestamp, now_millis)?);
        }
        Ok(actions)
    }

    /// Emits one sequenced application message.
    ///
    /// # Errors
    /// Returns an error unless Logon has established the session.
    pub fn send_application(
        &mut self,
        message: FixMessage,
        timestamp: &str,
    ) -> Result<Vec<SessionAction>, SessionError> {
        if self.snapshot.state != ConnectionState::Established {
            return Err(SessionError::NotEstablished);
        }
        self.send_new(
            message,
            timestamp,
            self.snapshot.last_sent_millis,
            true,
            None,
        )
    }

    /// Emits one sequenced application message and retains only its latest
    /// resend copy for the supplied logical stream key.
    ///
    /// Sequence numbers remain monotonic. A resend spanning a superseded copy
    /// receives a SequenceReset-GapFill before the newest retained copy.
    ///
    /// # Errors
    /// Returns an error unless Logon has established the session or the key is
    /// empty or exceeds the configured field bound.
    pub fn send_replaceable_application(
        &mut self,
        message: FixMessage,
        timestamp: &str,
        replacement_key: &str,
    ) -> Result<Vec<SessionAction>, SessionError> {
        if self.snapshot.state != ConnectionState::Established {
            return Err(SessionError::NotEstablished);
        }
        if replacement_key.is_empty()
            || replacement_key.len() > self.config.wire_limits.max_field_bytes
        {
            return Err(SessionError::InvalidReplacementKey);
        }
        self.send_new(
            message,
            timestamp,
            self.snapshot.last_sent_millis,
            true,
            Some(replacement_key),
        )
    }

    /// Initiates an orderly FIX logout handshake.
    ///
    /// # Errors
    /// Returns an error unless the session is established or framing fails.
    pub fn request_logout(
        &mut self,
        timestamp: &str,
        reason: Option<&str>,
    ) -> Result<Vec<SessionAction>, SessionError> {
        if self.snapshot.state != ConnectionState::Established {
            return Err(SessionError::NotEstablished);
        }
        self.snapshot.state = ConnectionState::LogoutPending;
        let mut logout = FixMessage::new("5");
        if let Some(reason) = reason {
            logout.push(58, reason);
        }
        self.send_new(
            logout,
            timestamp,
            self.snapshot.last_sent_millis,
            true,
            None,
        )
    }

    /// Emits heartbeat, test-request, or disconnect actions from explicit time.
    ///
    /// # Errors
    /// Returns an error for arithmetic or framing.
    pub fn poll(
        &mut self,
        now_millis: u64,
        timestamp: &str,
    ) -> Result<Vec<SessionAction>, SessionError> {
        if self.snapshot.state != ConnectionState::Established {
            return Ok(Vec::new());
        }
        let interval = u64::from(self.config.heartbeat_seconds)
            .checked_mul(1_000)
            .ok_or(SessionError::ArithmeticOverflow)?;
        if let Some(_) = self.snapshot.outstanding_test_request
            && now_millis.saturating_sub(self.snapshot.last_received_millis) >= interval * 2
        {
            self.snapshot.state = ConnectionState::Disconnected;
            return Ok(vec![
                SessionAction::Persist(self.snapshot()),
                SessionAction::Disconnect,
            ]);
        }
        if now_millis.saturating_sub(self.snapshot.last_received_millis) >= interval
            && self.snapshot.outstanding_test_request.is_none()
        {
            let id = format!("test-{now_millis}");
            self.snapshot.outstanding_test_request = Some(id.clone());
            let mut request = FixMessage::new("1");
            request.push(112, id);
            return self.send_new(request, timestamp, now_millis, true, None);
        }
        if now_millis.saturating_sub(self.snapshot.last_sent_millis) >= interval {
            return self.send_new(FixMessage::new("0"), timestamp, now_millis, true, None);
        }
        Ok(Vec::new())
    }

    fn send_new(
        &mut self,
        message: FixMessage,
        timestamp: &str,
        now_millis: u64,
        journal: bool,
        replacement_key: Option<&str>,
    ) -> Result<Vec<SessionAction>, SessionError> {
        let sequence = self.snapshot.outgoing_sequence;
        let next_sequence = sequence
            .checked_add(1)
            .ok_or(SessionError::ArithmeticOverflow)?;
        let frame = self.encode_at(message, sequence, timestamp, None)?;

        if journal {
            if let Some(key) = replacement_key {
                self.snapshot
                    .journal
                    .retain(|_, entry| entry.replacement_key.as_deref() != Some(key));
            }
            while self.snapshot.journal.len() >= self.config.max_journal_messages {
                let Some(oldest) = self.snapshot.journal.first_key_value().map(|(key, _)| *key)
                else {
                    break;
                };
                self.snapshot.journal.remove(&oldest);
            }
            self.snapshot.journal.insert(
                sequence,
                JournalEntry {
                    sequence,
                    frame: frame.clone(),
                    replacement_key: replacement_key.map(ToOwned::to_owned),
                },
            );
        }
        self.snapshot.outgoing_sequence = next_sequence;
        self.snapshot.last_sent_millis = now_millis;
        Ok(vec![
            SessionAction::Send(frame),
            SessionAction::Persist(self.snapshot()),
        ])
    }

    fn encode_at(
        &self,
        mut message: FixMessage,
        sequence: u64,
        timestamp: &str,
        original_timestamp: Option<&str>,
    ) -> Result<Vec<u8>, SessionError> {
        message
            .fields
            .retain(|field| !matches!(field.tag, 49 | 56 | 34 | 43 | 52 | 122));
        message.push(49, self.config.sender_comp_id.clone());
        message.push(56, self.config.target_comp_id.clone());
        message.push(34, sequence.to_string());
        if let Some(original) = original_timestamp {
            message.push(43, "Y");
            message.push(122, original);
        }
        message.push(52, timestamp);
        Ok(message.encode(&self.config.wire_limits)?)
    }

    fn receive(
        &mut self,
        message: FixMessage,
        timestamp: &str,
        now_millis: u64,
    ) -> Result<Vec<SessionAction>, SessionError> {
        if message.value(49) != Some(self.config.target_comp_id.as_str())
            || message.value(56) != Some(self.config.sender_comp_id.as_str())
        {
            return Err(SessionError::InvalidCompId);
        }
        let sequence = parse(&message, 34).ok_or(SessionError::MissingSequence)?;
        if sequence > self.snapshot.incoming_sequence {
            if self.snapshot.pending_inbound.len() >= self.config.max_pending_inbound {
                return Err(SessionError::PendingInboundFull);
            }
            self.snapshot
                .pending_inbound
                .entry(sequence)
                .or_insert(message);
            if self.snapshot.pending_resend_begin.is_some() {
                return Ok(vec![SessionAction::Persist(self.snapshot())]);
            }
            self.snapshot.pending_resend_begin = Some(self.snapshot.incoming_sequence);
            let mut resend = FixMessage::new("2");
            resend.push(7, self.snapshot.incoming_sequence.to_string());
            resend.push(16, "0");
            return self.send_new(resend, timestamp, now_millis, true, None);
        }
        if sequence < self.snapshot.incoming_sequence {
            return if message.value(43) == Some("Y") && message.value(122).is_some() {
                Ok(Vec::new())
            } else {
                Err(SessionError::InvalidSequence)
            };
        }
        let mut actions = self.process_in_sequence(message, timestamp, now_millis)?;
        while let Some(next) = self
            .snapshot
            .pending_inbound
            .remove(&self.snapshot.incoming_sequence)
        {
            actions.extend(self.process_in_sequence(next, timestamp, now_millis)?);
        }
        if self.snapshot.pending_inbound.is_empty() {
            self.snapshot.pending_resend_begin = None;
        }
        actions.push(SessionAction::Persist(self.snapshot()));
        Ok(actions)
    }

    fn process_in_sequence(
        &mut self,
        message: FixMessage,
        timestamp: &str,
        now_millis: u64,
    ) -> Result<Vec<SessionAction>, SessionError> {
        let sequence = self.snapshot.incoming_sequence;
        self.snapshot.incoming_sequence = sequence
            .checked_add(1)
            .ok_or(SessionError::ArithmeticOverflow)?;
        self.snapshot.last_received_millis = now_millis;
        match message.msg_type.as_str() {
            "A" => {
                if message.value(1137) != Some(FIX_50_SP2_APPL_VER_ID) {
                    return Err(SessionError::InvalidApplicationVersion);
                }
                self.snapshot.state = ConnectionState::Established;
                Ok(vec![SessionAction::PeerLogon(message)])
            }
            "0" => {
                if self.snapshot.outstanding_test_request.as_deref() == message.value(112) {
                    self.snapshot.outstanding_test_request = None;
                }
                Ok(Vec::new())
            }
            "1" => {
                let mut heartbeat = FixMessage::new("0");
                if let Some(id) = message.value(112) {
                    heartbeat.push(112, id);
                }
                self.send_new(heartbeat, timestamp, now_millis, true, None)
            }
            "2" => self.replay_requested(&message, timestamp, now_millis),
            "4" => {
                let new_sequence = parse(&message, 36).ok_or(SessionError::MissingSequence)?;
                if new_sequence < self.snapshot.incoming_sequence {
                    return Err(SessionError::InvalidSequence);
                }
                self.snapshot.incoming_sequence = new_sequence;
                Ok(Vec::new())
            }
            "5" => {
                self.snapshot.state = ConnectionState::Disconnected;
                let mut logout = FixMessage::new("5");
                logout.push(58, "logout acknowledged");
                let mut output = self.send_new(logout, timestamp, now_millis, false, None)?;
                output.push(SessionAction::Disconnect);
                Ok(output)
            }
            _ => Ok(vec![SessionAction::Application(message)]),
        }
    }

    fn replay_requested(
        &mut self,
        request: &FixMessage,
        timestamp: &str,
        now_millis: u64,
    ) -> Result<Vec<SessionAction>, SessionError> {
        let begin = parse(request, 7).ok_or(SessionError::MissingSequence)?;
        let requested_end = parse(request, 16).ok_or(SessionError::MissingSequence)?;
        let last_sent = self.snapshot.outgoing_sequence.saturating_sub(1);
        let end = if requested_end == 0 {
            last_sent
        } else {
            requested_end.min(last_sent)
        };
        if begin == 0 || begin > end.saturating_add(1) {
            return Err(SessionError::InvalidSequence);
        }
        let mut actions = Vec::new();
        let mut sequence = begin;
        while sequence <= end {
            if let Some(entry) = self.snapshot.journal.get(&sequence) {
                let original = decode_one(&entry.frame, self.config.wire_limits)?;
                let original_time = original.value(52).unwrap_or(timestamp).to_owned();
                let frame = self.encode_at(original, sequence, timestamp, Some(&original_time))?;
                actions.push(SessionAction::Send(frame));
                sequence = sequence
                    .checked_add(1)
                    .ok_or(SessionError::ArithmeticOverflow)?;
            } else {
                let next = self
                    .snapshot
                    .journal
                    .range(sequence..=end)
                    .next()
                    .map_or(end.saturating_add(1), |(value, _)| *value);
                let mut gap_fill = FixMessage::new("4");
                gap_fill.push(123, "Y");
                gap_fill.push(36, next.to_string());
                actions.push(SessionAction::Send(self.encode_at(
                    gap_fill,
                    sequence,
                    timestamp,
                    Some(timestamp),
                )?));
                sequence = next;
            }
        }
        self.snapshot.last_sent_millis = now_millis;
        actions.push(SessionAction::Persist(self.snapshot()));
        Ok(actions)
    }
}

fn validate_config(config: &SessionConfig) -> Result<(), SessionError> {
    if config.sender_comp_id.is_empty()
        || config.target_comp_id.is_empty()
        || config.sender_comp_id.len() > config.wire_limits.max_field_bytes
        || config.target_comp_id.len() > config.wire_limits.max_field_bytes
        || config.heartbeat_seconds == 0
        || config.max_journal_messages == 0
        || config.max_pending_inbound == 0
        || config.logon_fields.len() > 16
        || config.logon_fields.iter().any(|field| {
            matches!(
                field.tag,
                8 | 9 | 10 | 34 | 35 | 49 | 52 | 56 | 98 | 108 | 1137
            ) || field.value.len() > config.wire_limits.max_field_bytes
        })
    {
        return Err(SessionError::InvalidConfig);
    }
    let mut tags = config
        .logon_fields
        .iter()
        .map(|field| field.tag)
        .collect::<Vec<_>>();
    tags.sort_unstable();
    if tags.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(SessionError::InvalidConfig);
    }
    Ok(())
}

fn decode_one(frame: &[u8], limits: WireLimits) -> Result<FixMessage, SessionError> {
    Decoder::try_new(limits)?
        .push(frame)?
        .into_iter()
        .next()
        .ok_or(SessionError::InvalidSequence)
}

fn parse(message: &FixMessage, tag: u32) -> Option<u64> {
    message.value(tag)?.parse().ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn config() -> SessionConfig {
        SessionConfig {
            sender_comp_id: "BUNTING".to_owned(),
            target_comp_id: "ACCEPTOR".to_owned(),
            heartbeat_seconds: 30,
            max_journal_messages: 32,
            max_pending_inbound: 8,
            wire_limits: WireLimits::default(),
            logon_fields: Vec::new(),
        }
    }

    #[test]
    fn logon_includes_validated_profile_and_credentials() {
        let mut config = config();
        config.logon_fields = vec![
            Field::new(553, "participant"),
            Field::new(554, "secret"),
            Field::new(10000, "bunting.fixlatest.competition.v1"),
        ];
        let mut session = FixSession::try_new(config).unwrap();
        let actions = session.connected_at("20260713-12:00:00.000", 0).unwrap();
        let frame = actions
            .iter()
            .find_map(|action| match action {
                SessionAction::Send(frame) => Some(frame),
                _ => None,
            })
            .unwrap();
        let logon = decode_one(frame, WireLimits::default()).unwrap();
        assert_eq!(logon.value(553), Some("participant"));
        assert_eq!(logon.value(554), Some("secret"));
        assert_eq!(logon.value(1137), Some(FIX_50_SP2_APPL_VER_ID));
        assert_eq!(logon.value(10000), Some("bunting.fixlatest.competition.v1"));
    }

    #[test]
    fn logon_rejects_session_owned_or_duplicate_fields() {
        let mut reserved = config();
        reserved.logon_fields = vec![Field::new(49, "OTHER")];
        assert!(matches!(
            FixSession::try_new(reserved),
            Err(SessionError::InvalidConfig)
        ));

        let mut duplicate = config();
        duplicate.logon_fields = vec![Field::new(553, "one"), Field::new(553, "two")];
        assert!(matches!(
            FixSession::try_new(duplicate),
            Err(SessionError::InvalidConfig)
        ));
    }

    fn inbound(msg_type: &str, sequence: u64) -> Vec<u8> {
        let mut message = FixMessage::new(msg_type);
        if msg_type == "A" {
            message.push(1137, FIX_50_SP2_APPL_VER_ID);
        }
        message.push(49, "ACCEPTOR");
        message.push(56, "BUNTING");
        message.push(34, sequence.to_string());
        message.push(52, "20260713-12:00:00.000");
        message.encode(&WireLimits::default()).unwrap()
    }

    fn inbound_with(msg_type: &str, sequence: u64, fields: &[(u32, &str)]) -> Vec<u8> {
        let mut message = FixMessage::new(msg_type);
        if msg_type == "A" {
            message.push(1137, FIX_50_SP2_APPL_VER_ID);
        }
        for (tag, value) in fields {
            message.push(*tag, *value);
        }
        message.push(49, "ACCEPTOR");
        message.push(56, "BUNTING");
        message.push(34, sequence.to_string());
        message.push(52, "20260713-12:00:00.000");
        message.encode(&WireLimits::default()).unwrap()
    }

    fn establish(session: &mut FixSession) {
        session.connected_at("20260713-12:00:00.000", 0).unwrap();
        session
            .receive_bytes_at(&inbound("A", 1), "20260713-12:00:00.001", 1)
            .unwrap();
    }

    #[test]
    fn gaps_are_bounded_and_drained_after_missing_message() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        let high = inbound("D", 3);
        let actions = session
            .receive_bytes_at(&high, "20260713-12:00:00.002", 2)
            .unwrap();
        assert!(
            actions
                .iter()
                .any(|action| matches!(action, SessionAction::Send(_)))
        );
        let missing = inbound("0", 2);
        let actions = session
            .receive_bytes_at(&missing, "20260713-12:00:00.003", 3)
            .unwrap();
        assert!(actions.iter().any(|action| matches!(action, SessionAction::Application(message) if message.msg_type == "D")));
        assert_eq!(session.snapshot().incoming_sequence, 4);
    }

    #[test]
    fn possdup_requires_original_sending_time() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        let mut duplicate = FixMessage::new("0");
        duplicate.push(49, "ACCEPTOR");
        duplicate.push(56, "BUNTING");
        duplicate.push(34, "1");
        duplicate.push(43, "Y");
        duplicate.push(52, "20260713-12:00:00.000");
        let frame = duplicate.encode(&WireLimits::default()).unwrap();
        assert_eq!(
            session.receive_bytes(&frame, "20260713-12:00:00.004"),
            Err(SessionError::InvalidSequence)
        );
    }

    #[test]
    fn heartbeat_timeout_uses_explicit_clock() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        let first = session.poll(30_001, "20260713-12:00:30.001").unwrap();
        assert!(
            first
                .iter()
                .any(|action| matches!(action, SessionAction::Send(_)))
        );
        let second = session.poll(60_001, "20260713-12:01:00.001").unwrap();
        assert!(second.contains(&SessionAction::Disconnect));
    }

    #[test]
    fn full_journal_evicts_oldest_and_gap_fills_resend() {
        let mut bounded = config();
        bounded.max_journal_messages = 4;
        let mut session = FixSession::try_new(bounded).unwrap();
        establish(&mut session);
        for id in 0..20 {
            let mut message = FixMessage::new("8");
            message.push(17, id.to_string());
            session
                .send_application(message, "20260713-12:00:01.000")
                .unwrap();
        }

        let snapshot = session.snapshot();
        assert_eq!(snapshot.journal.len(), 4);
        assert_eq!(
            snapshot.journal.keys().copied().collect::<Vec<_>>(),
            vec![18, 19, 20, 21]
        );

        let request = inbound_with("2", 2, &[(7, "1"), (16, "0")]);
        let actions = session
            .receive_bytes_at(&request, "20260713-12:00:02.000", 2_000)
            .unwrap();
        let replayed = actions
            .iter()
            .filter_map(|action| match action {
                SessionAction::Send(frame) => {
                    Some(decode_one(frame, WireLimits::default()).unwrap())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(replayed.len(), 5);
        assert_eq!(replayed[0].msg_type, "4");
        assert_eq!(replayed[0].value(34), Some("1"));
        assert_eq!(replayed[0].value(123), Some("Y"));
        assert_eq!(replayed[0].value(36), Some("18"));
        assert_eq!(replayed[1].value(34), Some("18"));
        assert_eq!(replayed[4].value(34), Some("21"));
    }

    #[test]
    fn replaceable_application_retains_only_latest_resend_copy() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        let mut old_book = FixMessage::new("W");
        old_book.push(262, "book-agents");
        old_book.push(270, "100");
        session
            .send_replaceable_application(
                old_book,
                "20260713-12:00:01.000",
                "market-data:book-agents",
            )
            .unwrap();
        session
            .send_application(FixMessage::new("8"), "20260713-12:00:01.100")
            .unwrap();
        let mut new_book = FixMessage::new("W");
        new_book.push(262, "book-agents");
        new_book.push(270, "101");
        session
            .send_replaceable_application(
                new_book,
                "20260713-12:00:01.200",
                "market-data:book-agents",
            )
            .unwrap();

        assert_eq!(
            session
                .snapshot()
                .journal
                .keys()
                .copied()
                .collect::<Vec<_>>(),
            vec![1, 3, 4]
        );
        let request = inbound_with("2", 2, &[(7, "2"), (16, "4")]);
        let actions = session
            .receive_bytes_at(&request, "20260713-12:00:02.000", 2_000)
            .unwrap();
        let replayed = actions
            .iter()
            .filter_map(|action| match action {
                SessionAction::Send(frame) => {
                    Some(decode_one(frame, WireLimits::default()).unwrap())
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(replayed.len(), 3);
        assert_eq!(replayed[0].msg_type, "4");
        assert_eq!(replayed[0].value(36), Some("3"));
        assert_eq!(replayed[1].msg_type, "8");
        assert_eq!(replayed[2].msg_type, "W");
        assert_eq!(replayed[2].value(270), Some("101"));
    }

    #[test]
    fn snapshot_restore_preserves_sequences_and_journal() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        session
            .send_application(FixMessage::new("D"), "20260713-12:00:01.000")
            .unwrap();
        let snapshot = session.snapshot();
        let restored = FixSession::restore(config(), snapshot.clone()).unwrap();
        assert_eq!(restored.snapshot(), snapshot);
    }

    #[test]
    fn resend_request_replays_with_possdup_and_original_time() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        session
            .send_application(FixMessage::new("D"), "20260713-12:00:01.000")
            .unwrap();
        let request = inbound_with("2", 2, &[(7, "1"), (16, "0")]);
        let actions = session
            .receive_bytes_at(&request, "20260713-12:00:02.000", 2_000)
            .unwrap();
        let replayed: Vec<_> = actions
            .iter()
            .filter_map(|action| match action {
                SessionAction::Send(frame) => {
                    Some(decode_one(frame, WireLimits::default()).unwrap())
                }
                _ => None,
            })
            .collect();
        assert!(
            replayed
                .iter()
                .any(|message| message.value(43) == Some("Y") && message.value(122).is_some())
        );
    }

    #[test]
    fn sequence_reset_gap_fill_drains_buffered_application() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        session
            .receive_bytes_at(&inbound("D", 3), "20260713-12:00:01.000", 1_000)
            .unwrap();
        let gap_fill = inbound_with("4", 2, &[(123, "Y"), (36, "3")]);
        let actions = session
            .receive_bytes_at(&gap_fill, "20260713-12:00:02.000", 2_000)
            .unwrap();
        assert!(actions.iter().any(|action| matches!(action, SessionAction::Application(message) if message.msg_type == "D")));
        assert_eq!(session.snapshot().incoming_sequence, 4);
    }

    #[test]
    fn logout_and_reconnect_preserve_monotonic_outbound_sequence() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        session
            .request_logout("20260713-12:00:01.000", Some("test complete"))
            .unwrap();
        let before = session.snapshot().outgoing_sequence;
        let snapshot = session.snapshot();
        let mut restored = FixSession::restore(config(), snapshot).unwrap();
        restored
            .connected_at("20260713-12:00:02.000", 2_000)
            .unwrap();
        assert_eq!(restored.snapshot().outgoing_sequence, before + 1);
        assert_eq!(restored.snapshot().reconnect_generation, 2);
    }

    #[test]
    fn fixt_upgrade_preserves_the_legacy_bounded_recovery_transition() {
        let mut session = FixSession::try_new(config()).unwrap();
        establish(&mut session);
        session
            .receive_bytes_at(&inbound("D", 3), "20260713-12:00:01.000", 1_000)
            .unwrap();

        let pending = session.snapshot();
        let legacy_golden = (2, 3, Some(2), vec![3], vec![1, 2]);
        assert_eq!(
            (
                pending.incoming_sequence,
                pending.outgoing_sequence,
                pending.pending_resend_begin,
                pending.pending_inbound.keys().copied().collect::<Vec<_>>(),
                pending.journal.keys().copied().collect::<Vec<_>>(),
            ),
            legacy_golden
        );

        let mut restored = FixSession::restore(config(), pending).unwrap();
        restored
            .receive_bytes_at(&inbound("0", 2), "20260713-12:00:02.000", 2_000)
            .unwrap();
        let recovered = restored.snapshot();
        assert_eq!(recovered.incoming_sequence, 4);
        assert!(recovered.pending_inbound.is_empty());
        assert_eq!(recovered.pending_resend_begin, None);
    }
}
