//! Bunting Native Protocol listener (ADR 0040): TLS 1.3 with mutual
//! authentication terminated in this process, identity from the registered
//! client certificate, and the same admission, commit and delivery path as
//! FIX (ADR 0035).
//!
//! Each connection's reader thread stamps *ciphertext* the moment it
//! arrives (`wake.rs`), so the team's real delay, including its TLS stack,
//! is inside the stamp. The session thread alone owns the TLS state: it
//! decrypts, admits each request without waiting, and writes every venue
//! message once it has crossed the virtual path `L(s, p)`. Probes, pongs
//! and heartbeats are session messages and are never delayed. The private
//! stream is keyed by committed event sequence, so a reconnecting client
//! resumes after its cursor from the distributor's retained batches.

use crate::acceptor::{ConnectionSlots, SLOT_WAIT};
use crate::admission::{
    AdmissionService, ConnectionHealth, Inbound, Interface, JobWork, Reply, reply_task,
};
use crate::bnp_trust::Trust;
use crate::config::{BnpConfig, BnpRosterEntry};
use crate::distributor::{PublishingOrigin, Resume};
use crate::outbound::OutboundHold;
use crate::public_feed::{FeedMessage, PublicFeeds};
use crate::wake::{SessionEvent, Waker};
use bnp_wire::{
    ClientMessage, Entry, EntryKind, FeedFlags, FrameDecoder, Level, Listing, PROTOCOL_VERSION,
    ResumeStatus, Role, ServerMessage, Welcome, decode_client, encode_server,
};
use bunting_admission_sequencer::{DelayEstimator, Endpoint};
use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_application::bnp::{
    BnpIdentity, account_balances, listing_key, listings, wire_listing,
};
use bunting_application::{
    ApplicationService, MarketDataEntryType, VerifiedActor, listing_for_command, project_market,
};
use bunting_engine::RunState;
use bunting_market_events::{Command, Side};
use bunting_market_types::{CorrelationId, ListingKey, ParticipantId, RunId};
use bunting_origin_store::OriginStore;
use rustls::ServerConnection;
use rustls::pki_types::CertificateDer;
use sha2::{Digest, Sha256};
use simfix_mapping::MarketDataIncrement;
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::mpsc::{RecvTimeoutError, SyncSender, sync_channel};
use std::time::{Duration, Instant};

/// Registered client certificates by SHA-256 fingerprint.
type Roster = BTreeMap<[u8; 32], BnpRosterEntry>;

pub(crate) fn run(
    config: &BnpConfig,
    origin: &Arc<PublishingOrigin>,
    admission: &Arc<AdmissionService>,
) -> Result<(), String> {
    let trust = Arc::new(Trust::load(config)?);
    trust.watch(config.clone())?;
    let roster = Arc::new(roster(config)?);
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind BNP listener {}: {error}", config.bind))?;
    let slots = Arc::new(ConnectionSlots::new(config.max_connections));
    loop {
        let (stream, _) = listener
            .accept()
            .map_err(|error| format!("BNP accept failed: {error}"))?;
        // The venue never adds Nagle delay to a participant's messages.
        stream
            .set_nodelay(true)
            .map_err(|error| format!("cannot disable Nagle on BNP socket: {error}"))?;
        if !slots.try_queue() {
            // Refused before TLS: nothing is sent to an unauthenticated peer.
            continue;
        }
        let config = config.clone();
        let trust = trust.clone();
        let roster = roster.clone();
        let origin = origin.clone();
        let admission = admission.clone();
        let slots = slots.clone();
        std::thread::Builder::new()
            .name("bunting-bnp-session".to_owned())
            .spawn(move || {
                let Some(_slot) = slots.acquire(SLOT_WAIT) else {
                    return;
                };
                if let Err(error) =
                    handle_connection(stream, &config, &trust, &roster, &origin, &admission)
                {
                    eprintln!("bunting-server: BNP connection closed: {error}");
                }
            })
            .map_err(|error| format!("cannot spawn BNP session: {error}"))?;
    }
}

fn roster(config: &BnpConfig) -> Result<Roster, String> {
    config
        .roster
        .iter()
        .map(|entry| {
            let mut fingerprint = [0_u8; 32];
            for (index, byte) in fingerprint.iter_mut().enumerate() {
                *byte = entry
                    .certificate_sha256
                    .get(index * 2..index * 2 + 2)
                    .and_then(|digits| u8::from_str_radix(digits, 16).ok())
                    .ok_or("bnp.roster fingerprint is not hex SHA-256")?;
            }
            Ok((fingerprint, entry.clone()))
        })
        .collect()
}

/// How long a reconnecting participant waits for its previous BNP session
/// to finish closing.
const SESSION_HANDOVER: Duration = Duration::from_secs(5);
/// Longest sleep without an event.
const DELIVERY_POLL: Duration = Duration::from_millis(20);
/// Probes sent right after `Welcome`, before the first order.
const LOGON_PROBES: usize = 3;
const MAX_OUTSTANDING_PROBES: usize = 16;
/// Bytes read from the socket at a time.
const READ_BYTES: usize = 16_384;

/// One authenticated BNP connection, from handshake to close.
#[expect(
    clippy::too_many_lines,
    reason = "the handshake/session/delivery loop keeps commit-before-response ordering visible"
)]
fn handle_connection(
    mut stream: TcpStream,
    config: &BnpConfig,
    trust: &Trust,
    roster: &Roster,
    origin: &PublishingOrigin,
    admission: &AdmissionService,
) -> Result<(), String> {
    let handshake = Duration::from_millis(config.handshake_timeout_ms);
    let deadline = Instant::now() + handshake;
    stream
        .set_read_timeout(Some(handshake))
        .map_err(|error| format!("cannot configure BNP handshake timeout: {error}"))?;
    // Read the generation first: a reload during the handshake makes the
    // session re-verify against the newer trust.
    let mut trust_generation = trust.generation();
    let mut connection = ServerConnection::new(trust.current()?.tls.clone())
        .map_err(|error| format!("cannot start BNP TLS: {error}"))?;
    while connection.is_handshaking() {
        connection
            .complete_io(&mut stream)
            .map_err(|error| format!("BNP TLS handshake failed: {error}"))?;
        if Instant::now() > deadline {
            return Err("BNP TLS handshake timed out".to_owned());
        }
    }
    // The verifier proved the chain; the roster decides who it is. The
    // chain is kept so the session can re-verify it after a trust reload.
    let peer_chain: Vec<CertificateDer<'static>> = connection
        .peer_certificates()
        .map(|chain| chain.iter().map(|der| der.clone().into_owned()).collect())
        .unwrap_or_default();
    let fingerprint: [u8; 32] = peer_chain
        .first()
        .map(|leaf| Sha256::digest(leaf.as_ref()).into())
        .ok_or("BNP client presented no certificate")?;
    let mut channel = Channel {
        connection,
        stream,
        decoder: FrameDecoder::new(config.max_frame_bytes),
        last_sent: Instant::now(),
    };
    let Some(entry) = roster.get(&fingerprint) else {
        channel.send(&[ServerMessage::Logout {
            reason: "certificate is not registered for this run".to_owned(),
        }])?;
        return Err(format!(
            "unregistered BNP client certificate {}",
            hex(&fingerprint)
        ));
    };
    let participant = ParticipantId::new(entry.participant_id);
    let hello_frames = channel.read_until_frame(deadline)?;
    let hello_us = admission.clock().now_us();
    let mut frames = hello_frames.into_iter();
    let resume_after = match frames.next().map(|frame| decode_client(&frame)) {
        Some(Ok(ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            resume_after,
            ..
        })) => resume_after,
        Some(Ok(ClientMessage::Hello { version, .. })) => {
            channel.send(&[ServerMessage::Logout {
                reason: format!(
                    "unsupported BNP version {version}; this venue speaks {PROTOCOL_VERSION}"
                ),
            }])?;
            return Err(format!("BNP client version {version}"));
        }
        Some(Ok(_)) | None => {
            channel.send(&[ServerMessage::Logout {
                reason: "the first message must be Hello".to_owned(),
            }])?;
            return Err("BNP client did not start with Hello".to_owned());
        }
        Some(Err(error)) => {
            channel.send(&[ServerMessage::Logout {
                reason: format!("invalid Hello: {error}"),
            }])?;
            return Err(format!("invalid BNP Hello: {error}"));
        }
    };
    let pipelined: Vec<Vec<u8>> = frames.collect();
    let _claim = match admission.claim_participant(Interface::Bnp, participant, SESSION_HANDOVER) {
        Ok(claim) => claim,
        Err(reason) => {
            channel.send(&[ServerMessage::Logout {
                reason: reason.clone(),
            }])?;
            return Err(reason);
        }
    };
    let actor = VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(entry.participant_id),
        role: ActorRole::Participant,
        participant_id: Some(UnsignedDecimalString::new(entry.participant_id)),
        team_id: None,
    })
    .map_err(|error| format!("invalid configured BNP actor: {error}"))?;
    let run_id = RunId::new(config.run_id);
    let identity = BnpIdentity::new(run_id, participant);
    channel
        .stream
        .set_read_timeout(None)
        .map_err(|error| format!("cannot configure BNP socket: {error}"))?;
    let clock = *admission.clock();
    let (events, waker) =
        crate::wake::connect(&channel.stream, READ_BYTES, move || clock.now_us())?;
    let _shutdown = crate::wake::ShutdownOnDrop::new(&channel.stream)?;
    let run_sequence = || {
        origin
            .read_run(run_id, |state| state.event_sequence().get())
            .ok()
    };
    // Subscribe before anything is admitted, so no committed batch falls
    // between the resume point and the live stream.
    let (subscription, resume) = match resume_after {
        Some(after) => {
            origin
                .distributor()
                .subscribe_resuming(Some(waker.clone()), after, run_sequence)?
        }
        None => (
            origin.distributor().subscribe(Some(waker.clone()))?,
            Resume::Replay(Vec::new()),
        ),
    };
    let status = match (&resume, resume_after) {
        (_, None) => ResumeStatus::Live,
        (Resume::Replay(_), Some(_)) => ResumeStatus::Replaying,
        (Resume::Gap, Some(_)) => ResumeStatus::Gap,
    };
    channel.send(&[ServerMessage::Welcome(Welcome {
        version: PROTOCOL_VERSION,
        run_id: config.run_id,
        participant_id: entry.participant_id,
        role: Role::Participant,
        heartbeat_ms: config.heartbeat_ms,
        max_frame_bytes: u32::try_from(config.max_frame_bytes).unwrap_or(u32::MAX),
        resume: status,
        run_sequence: run_sequence().unwrap_or(0),
    })])?;
    let mut latency = Latency::new(admission, participant);
    latency.refresh_kernel(&channel.stream);
    for _ in 0..LOGON_PROBES {
        latency.probe(&mut channel)?;
    }
    let mut outbound = OutboundHold::new(admission.config().max_outbound_hold);
    // Replay what the client missed, each batch over its path as when it
    // was live (already due), then mark the end of the replay.
    if let (Resume::Replay(batches), Some(after)) = (resume, resume_after) {
        let mut through = after;
        let mut last_send = 0;
        for batch in &batches {
            let send_at = batch
                .durable_us
                .saturating_add(latency.delay_from(batch.source)?);
            last_send = last_send.max(send_at);
            for message in identity.private_messages(&batch.events) {
                if message.stamp().is_some_and(|stamp| stamp.sequence > after) {
                    outbound.hold(message, send_at)?;
                }
            }
            through = batch
                .events
                .last()
                .map_or(through, |event| event.sequence.get());
        }
        outbound.hold(
            ServerMessage::ReplayComplete {
                through_sequence: through,
            },
            last_send,
        )?;
    }
    let (reply_sender, replies) = sync_channel(admission.config().max_admission_queue);
    let reply_to = ReplyTo {
        replies: &reply_sender,
        waker: &waker,
    };
    let mut feeds = PublicFeeds::new(encode_increment);
    let mut state = SessionState {
        rate_window_started: Instant::now(),
        rate_messages: 0,
        release_floors: BTreeMap::new(),
        next_correlation: 1,
    };
    let context = SessionContext {
        config,
        origin,
        admission,
        actor: &actor,
        identity: &identity,
        run_id,
    };
    let heartbeat = Duration::from_millis(u64::from(config.heartbeat_ms));
    let mut last_received = Instant::now();
    for frame in pipelined {
        if !handle_frame(
            &frame,
            hello_us,
            &context,
            &mut channel,
            &mut state,
            &mut latency,
            &mut feeds,
            &mut outbound,
            &reply_to,
        )? {
            return Ok(());
        }
    }
    loop {
        let now_us = admission.clock().now_us();
        let wait = [outbound.next_due_us(), Some(latency.next_probe_us())]
            .into_iter()
            .flatten()
            .min()
            .map_or(DELIVERY_POLL, |due| {
                Duration::from_micros(due.saturating_sub(now_us)).min(DELIVERY_POLL)
            });
        let first = match events.recv_timeout(wait) {
            Ok(event) => Some(event),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return Err("BNP reader stopped".to_owned()),
        };
        for event in first.into_iter().chain(events.try_iter()) {
            let (received_us, bytes) = match event {
                SessionEvent::Bytes { received_us, bytes } => (received_us, bytes),
                SessionEvent::Closed(None) => return Ok(()),
                SessionEvent::Closed(Some(reason)) => return Err(reason),
                SessionEvent::Wake => continue,
            };
            last_received = Instant::now();
            // Messages decrypted from one read share the reader's stamp.
            let Some(frames) = channel.receive(&bytes)? else {
                return Ok(());
            };
            for frame in frames {
                if !handle_frame(
                    &frame,
                    received_us,
                    &context,
                    &mut channel,
                    &mut state,
                    &mut latency,
                    &mut feeds,
                    &mut outbound,
                    &reply_to,
                )? {
                    return Ok(());
                }
            }
        }
        // A reloaded revocation list applies to live sessions too.
        if trust.generation() != trust_generation {
            trust_generation = trust.generation();
            if let Err(error) = trust.current()?.verify(&peer_chain) {
                channel.send(&[ServerMessage::Logout {
                    reason: format!("certificate no longer trusted: {error}"),
                }])?;
                return Err(format!("BNP client certificate no longer trusted: {error}"));
            }
        }
        if last_received.elapsed() > heartbeat.saturating_mul(3) {
            channel.send(&[ServerMessage::Logout {
                reason: "heartbeat timeout".to_owned(),
            }])?;
            return Err("BNP client heartbeat timeout".to_owned());
        }
        if admission.clock().now_us() >= latency.next_probe_us() {
            latency.refresh_kernel(&channel.stream);
            latency.probe(&mut channel)?;
        }
        for reply in replies.try_iter() {
            let messages = match reply.result {
                Ok(messages) => messages,
                Err(reason) => vec![ServerMessage::Reject {
                    request_type: 0,
                    reference: 0,
                    reason,
                }],
            };
            let failed = messages
                .iter()
                .any(|message| matches!(message, ServerMessage::Reject { .. }));
            if let Some(feed) = &reply.feed
                && failed
            {
                feeds.unsubscribe(feed);
            }
            let mut send_at = reply
                .completed_us
                .saturating_add(latency.delay_from(reply.source)?);
            if let Some(feed) = reply.feed.as_deref().filter(|_| !failed) {
                send_at = feeds.send_time(feed, send_at);
            }
            for message in messages {
                outbound.hold(message, send_at)?;
            }
            if let Some(feed) = reply.feed.as_deref().filter(|_| !failed) {
                for increment in feeds.activate_after(feed, reply.published_before, 0) {
                    hold_feed(&mut outbound, &mut feeds, &latency, increment)?;
                }
            }
        }
        // Every committed batch, whoever caused it, maps to this
        // participant's private messages; each travels from where it was
        // applied to this team (ADR 0035).
        for batch in subscription.drain()? {
            for increment in feeds.on_batch(&batch)? {
                hold_feed(&mut outbound, &mut feeds, &latency, increment)?;
            }
            let messages = identity.private_messages(&batch.events);
            if messages.is_empty() {
                continue;
            }
            let send_at = batch
                .durable_us
                .saturating_add(latency.delay_from(batch.source)?);
            for message in messages {
                outbound.hold(message, send_at)?;
            }
        }
        let due = outbound.take_due(admission.clock().now_us());
        if !due.is_empty() {
            channel.send(&due)?;
        } else if channel.last_sent.elapsed() >= heartbeat {
            channel.send(&[ServerMessage::Heartbeat])?;
        }
    }
}

/// Holds one public feed message until it has crossed the virtual path from
/// its source, never ahead of the feed's previous message.
fn hold_feed(
    outbound: &mut OutboundHold<ServerMessage>,
    feeds: &mut PublicFeeds<ServerMessage>,
    latency: &Latency<'_>,
    increment: FeedMessage<ServerMessage>,
) -> Result<(), String> {
    let proposed = increment
        .available_us
        .saturating_add(latency.delay_from(increment.source)?);
    let send_at = feeds.send_time(&increment.request_id, proposed);
    outbound.hold(increment.message, send_at)
}

/// A feed increment as a BNP `MarketUpdate`.
///
/// BNP v1 feeds are direct, price-level feeds of one listing, so every entry
/// names the same listing and none is an order-by-order entry.
fn encode_increment(
    request_id: &str,
    first_report: u64,
    increments: &[(ListingKey, MarketDataIncrement)],
) -> ServerMessage {
    let listing = increments.first().map_or(
        Listing {
            venue_id: 0,
            instrument_id: 0,
        },
        |(key, _)| wire_listing(*key),
    );
    ServerMessage::MarketUpdate {
        request_id: request_id.parse().unwrap_or(0),
        listing,
        first_report,
        entries: increments
            .iter()
            .filter_map(|(_, increment)| match *increment {
                MarketDataIncrement::Trade {
                    price, quantity, ..
                } => Some(Entry {
                    kind: EntryKind::Trade,
                    price: price.get(),
                    quantity: quantity.get(),
                }),
                MarketDataIncrement::Level {
                    side,
                    price,
                    quantity,
                    ..
                } => Some(Entry {
                    kind: match side {
                        Side::Buy => EntryKind::Bid,
                        Side::Sell => EntryKind::Ask,
                    },
                    price: price.get(),
                    quantity: quantity.get(),
                }),
                MarketDataIncrement::Order { .. } => None,
            })
            .collect(),
    }
}

/// The connection's TLS state and socket, owned by the session thread.
struct Channel {
    connection: ServerConnection,
    stream: TcpStream,
    decoder: FrameDecoder,
    last_sent: Instant,
}

impl Channel {
    /// Encrypts and writes messages as one TLS write, so a batch of reports
    /// leaves in as few segments as possible.
    fn send(&mut self, messages: &[ServerMessage]) -> Result<(), String> {
        let mut bytes = Vec::new();
        for message in messages {
            encode_server(message, &mut bytes)
                .map_err(|error| format!("cannot encode BNP message: {error}"))?;
        }
        self.connection
            .writer()
            .write_all(&bytes)
            .map_err(|error| format!("BNP TLS write failed: {error}"))?;
        self.flush()?;
        self.last_sent = Instant::now();
        Ok(())
    }

    fn flush(&mut self) -> Result<(), String> {
        while self.connection.wants_write() {
            self.connection
                .write_tls(&mut self.stream)
                .map_err(|error| format!("BNP socket write failed: {error}"))?;
        }
        Ok(())
    }

    /// Feeds ciphertext received by the reader thread and returns the
    /// complete frames it decrypts to; `None` once the peer closed TLS.
    fn receive(&mut self, mut bytes: &[u8]) -> Result<Option<Vec<Vec<u8>>>, String> {
        while !bytes.is_empty() {
            self.connection
                .read_tls(&mut bytes)
                .map_err(|error| format!("BNP TLS read failed: {error}"))?;
            if let Err(error) = self.connection.process_new_packets() {
                // Send the alert before closing.
                let _ = self.flush();
                return Err(format!("BNP TLS error: {error}"));
            }
        }
        let frames = self.plaintext_frames()?;
        // Key updates and alerts may need a reply.
        self.flush()?;
        Ok(frames)
    }

    fn plaintext_frames(&mut self) -> Result<Option<Vec<Vec<u8>>>, String> {
        let mut frames = Vec::new();
        let mut buffer = [0_u8; READ_BYTES];
        loop {
            match self.connection.reader().read(&mut buffer) {
                Ok(0) => return Ok(None),
                Ok(count) => frames.extend(
                    self.decoder
                        .push(&buffer[..count])
                        .map_err(|error| format!("invalid BNP framing: {error}"))?,
                ),
                Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(Some(frames)),
                Err(error) => return Err(format!("BNP TLS read failed: {error}")),
            }
        }
    }

    /// Reads from the socket directly (before the reader thread starts)
    /// until at least one frame is complete or `deadline` passes.
    fn read_until_frame(&mut self, deadline: Instant) -> Result<Vec<Vec<u8>>, String> {
        let mut buffer = [0_u8; READ_BYTES];
        loop {
            match self.plaintext_frames()? {
                None => return Err("BNP client closed before Hello".to_owned()),
                Some(frames) if !frames.is_empty() => return Ok(frames),
                Some(_) => {}
            }
            if Instant::now() > deadline {
                return Err("BNP Hello timed out".to_owned());
            }
            let count = self
                .stream
                .read(&mut buffer)
                .map_err(|error| format!("cannot read BNP Hello: {error}"))?;
            if count == 0 {
                return Err("BNP client closed before Hello".to_owned());
            }
            let mut bytes = &buffer[..count];
            while !bytes.is_empty() {
                self.connection
                    .read_tls(&mut bytes)
                    .map_err(|error| format!("BNP TLS read failed: {error}"))?;
                self.connection
                    .process_new_packets()
                    .map_err(|error| format!("BNP TLS error: {error}"))?;
            }
            self.flush()?;
        }
    }
}

/// Where a released job's reply goes, and how its session is woken.
struct ReplyTo<'a> {
    replies: &'a SyncSender<Reply<ServerMessage>>,
    waker: &'a Waker,
}

/// What every request on this session needs.
struct SessionContext<'a> {
    config: &'a BnpConfig,
    origin: &'a PublishingOrigin,
    admission: &'a AdmissionService,
    actor: &'a VerifiedActor,
    identity: &'a BnpIdentity,
    run_id: RunId,
}

/// Per-session counters and per-destination release floors.
struct SessionState {
    rate_window_started: Instant,
    rate_messages: usize,
    /// Last release per destination: one path delivers in order (TCP), but
    /// an order to a near venue may overtake an earlier one to a far venue.
    release_floors: BTreeMap<Endpoint, u64>,
    next_correlation: u128,
}

/// Handles one decoded frame; returns `false` when the session should end.
#[expect(
    clippy::too_many_arguments,
    reason = "the session's parts stay separately borrowed so reply and feed bookkeeping cannot alias"
)]
fn handle_frame(
    frame: &[u8],
    received_us: u64,
    context: &SessionContext<'_>,
    channel: &mut Channel,
    state: &mut SessionState,
    latency: &mut Latency<'_>,
    feeds: &mut PublicFeeds<ServerMessage>,
    outbound: &mut OutboundHold<ServerMessage>,
    reply_to: &ReplyTo<'_>,
) -> Result<bool, String> {
    let message = match decode_client(frame) {
        Ok(message) => message,
        Err(error) => {
            channel.send(&[ServerMessage::Logout {
                reason: format!("invalid message: {error}"),
            }])?;
            return Err(format!("invalid BNP message: {error}"));
        }
    };
    // Session messages: answered at once, never delayed or rate limited.
    match &message {
        ClientMessage::Heartbeat => return Ok(true),
        ClientMessage::ProbeReply { probe_id } => {
            latency.on_response(*probe_id);
            return Ok(true);
        }
        ClientMessage::Ping { nonce } => {
            channel.send(&[ServerMessage::Pong {
                nonce: *nonce,
                server_time_us: context.admission.clock().now_us(),
            }])?;
            return Ok(true);
        }
        ClientMessage::Logout { .. } => {
            channel.send(&[ServerMessage::Logout {
                reason: "logout acknowledged".to_owned(),
            }])?;
            return Ok(false);
        }
        ClientMessage::Hello { .. } => {
            channel.send(&[ServerMessage::Logout {
                reason: "Hello is only valid once".to_owned(),
            }])?;
            return Err("repeated BNP Hello".to_owned());
        }
        _ => {}
    }
    let request_type = message.msg_type();
    let reference = request_reference(&message);
    let reject = |reason: String| ServerMessage::Reject {
        request_type,
        reference,
        reason,
    };
    if state.rate_window_started.elapsed()
        >= Duration::from_millis(context.config.rate_limit_window_ms)
    {
        state.rate_window_started = Instant::now();
        state.rate_messages = 0;
    }
    let refusal = if state.rate_messages >= context.config.max_messages_per_interval {
        Some(format!(
            "max_messages_per_interval limit {}",
            context.config.max_messages_per_interval
        ))
    } else {
        state.rate_messages = state.rate_messages.saturating_add(1);
        admit(
            &message,
            received_us,
            context,
            state,
            latency,
            feeds,
            reply_to,
        )?
    };
    if let Some(reason) = refusal {
        outbound.hold(
            reject(reason),
            received_us.saturating_add(latency.delay_from(Endpoint::Hub)?),
        )?;
    }
    Ok(true)
}

const fn request_reference(message: &ClientMessage) -> u64 {
    match message {
        ClientMessage::NewOrder(order) => order.client_order_id,
        ClientMessage::CancelOrder { client_order_id } => *client_order_id,
        ClientMessage::KillSwitch { request_id } => *request_id,
        ClientMessage::Subscribe { request_id, .. }
        | ClientMessage::Unsubscribe { request_id }
        | ClientMessage::SnapshotRequest { request_id, .. }
        | ClientMessage::ListingsRequest { request_id }
        | ClientMessage::OpenOrdersRequest { request_id }
        | ClientMessage::AccountRequest { request_id } => *request_id as u64,
        _ => 0,
    }
}

/// Maps one request and admits it without waiting for its release.
/// Returns a rejection reason when the request is refused but the session
/// stays up.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per request type keeps each destination beside its work"
)]
fn admit(
    message: &ClientMessage,
    received_us: u64,
    context: &SessionContext<'_>,
    state: &mut SessionState,
    latency: &Latency<'_>,
    feeds: &mut PublicFeeds<ServerMessage>,
    reply_to: &ReplyTo<'_>,
) -> Result<Option<String>, String> {
    let correlation = CorrelationId::new(state.next_correlation);
    state.next_correlation = state.next_correlation.wrapping_add(1);
    let request_type = message.msg_type();
    let reference = request_reference(message);
    let run_id = context.run_id;
    let mut feed = None;
    let (destination, work): (Endpoint, JobWork<ServerMessage>) = match message {
        ClientMessage::NewOrder(order) => {
            let command = match context.identity.new_order(order, correlation) {
                Ok(command) => command,
                Err(error) => return Ok(Some(error.to_string())),
            };
            (
                command_destination(context, &command)?,
                command_job(
                    command,
                    context.actor.clone(),
                    run_id,
                    request_type,
                    reference,
                ),
            )
        }
        ClientMessage::CancelOrder { client_order_id } => {
            let command = match context.identity.cancel(*client_order_id, correlation) {
                Ok(command) => command,
                Err(error) => return Ok(Some(error.to_string())),
            };
            (
                command_destination(context, &command)?,
                command_job(
                    command,
                    context.actor.clone(),
                    run_id,
                    request_type,
                    reference,
                ),
            )
        }
        ClientMessage::KillSwitch { request_id } => {
            let command = match context.identity.kill_switch(*request_id, correlation) {
                Ok(command) => command,
                Err(error) => return Ok(Some(error.to_string())),
            };
            (
                Endpoint::Hub,
                command_job(
                    command,
                    context.actor.clone(),
                    run_id,
                    request_type,
                    reference,
                ),
            )
        }
        ClientMessage::Subscribe {
            request_id,
            listing,
            flags,
        } => {
            let id = request_id.to_string();
            let mut entry_types = Vec::new();
            for (flag, entry) in [
                (FeedFlags::BIDS, MarketDataEntryType::Bid),
                (FeedFlags::OFFERS, MarketDataEntryType::Offer),
                (FeedFlags::TRADES, MarketDataEntryType::Trade),
            ] {
                if flags.contains(flag) {
                    entry_types.push(entry);
                }
            }
            // Registered before admission, so no batch committed after
            // the snapshot is missed.
            if let Some(reason) = feeds.subscribe(&id, listing_key(*listing), &entry_types, true) {
                return Ok(Some(reason));
            }
            feed = Some(id);
            (
                Endpoint::Venue(listing_key(*listing).venue_id),
                snapshot_job(*request_id, *listing, usize::MAX, 1, run_id, request_type),
            )
        }
        ClientMessage::Unsubscribe { request_id } => {
            return Ok((!feeds.unsubscribe(&request_id.to_string()))
                .then(|| format!("no market data subscription {request_id}")));
        }
        ClientMessage::SnapshotRequest {
            request_id,
            listing,
            depth,
        } => (
            Endpoint::Venue(listing_key(*listing).venue_id),
            snapshot_job(
                *request_id,
                *listing,
                if *depth == 0 {
                    usize::MAX
                } else {
                    usize::from(*depth)
                },
                0,
                run_id,
                request_type,
            ),
        ),
        ClientMessage::ListingsRequest { request_id } => {
            let request_id = *request_id;
            (
                Endpoint::Hub,
                Box::new(move |job| {
                    let listings = ApplicationService::new(job.origin)
                        .read(run_id, listings)
                        .map_err(|error| format!("run read failed: {error}"))?;
                    Ok(vec![ServerMessage::Listings {
                        request_id,
                        listings,
                    }])
                }),
            )
        }
        ClientMessage::OpenOrdersRequest { request_id } => {
            let request_id = *request_id;
            let identity = *context.identity;
            (
                Endpoint::Hub,
                Box::new(move |job| {
                    let (run_sequence, orders) = ApplicationService::new(job.origin)
                        .read(run_id, |state| {
                            (state.event_sequence().get(), identity.open_orders(state))
                        })
                        .map_err(|error| format!("run read failed: {error}"))?;
                    Ok(vec![ServerMessage::OpenOrders {
                        request_id,
                        run_sequence,
                        orders,
                    }])
                }),
            )
        }
        ClientMessage::AccountRequest { request_id } => {
            let request_id = *request_id;
            let actor = context.actor.clone();
            (
                Endpoint::Hub,
                Box::new(move |job| {
                    let read = ApplicationService::new(job.origin)
                        .read(run_id, |state| {
                            account_balances(state, &actor).map(|(cash, positions)| {
                                (state.event_sequence().get(), cash, positions)
                            })
                        })
                        .map_err(|error| format!("run read failed: {error}"))?;
                    Ok(vec![match read {
                        Ok((run_sequence, cash, positions)) => ServerMessage::Account {
                            request_id,
                            run_sequence,
                            cash,
                            positions,
                        },
                        Err(error) => ServerMessage::Reject {
                            request_type,
                            reference: u64::from(request_id),
                            reason: format!("account unavailable: {error}"),
                        },
                    }])
                }),
            )
        }
        ClientMessage::Hello { .. }
        | ClientMessage::Heartbeat
        | ClientMessage::ProbeReply { .. }
        | ClientMessage::Ping { .. }
        | ClientMessage::Logout { .. } => return Ok(None),
    };
    let inbound = Inbound {
        received_us,
        estimator: latency.estimator(),
        participant: context.identity.participant(),
        destination,
        floor_us: state.release_floors.get(&destination).copied().unwrap_or(0),
    };
    let task = reply_task(
        work,
        reply_to.replies.clone(),
        reply_to.waker.clone(),
        destination,
        feed.clone(),
    );
    match context.admission.admit(&inbound, task) {
        Ok(record) => {
            state.release_floors.insert(destination, record.release_us);
            Ok(None)
        }
        Err(error) => {
            if let Some(feed) = &feed {
                feeds.unsubscribe(feed);
            }
            Ok(Some(error.to_string()))
        }
    }
}

/// The venue an order or cancel is addressed to; the hub when the order is
/// unknown (its cancel is rejected by the engine).
fn command_destination(
    context: &SessionContext<'_>,
    command: &Command,
) -> Result<Endpoint, String> {
    Ok(context
        .origin
        .read_run(context.run_id, |state| listing_for_command(state, command))
        .map_err(|error| format!("run read failed: {error}"))?
        .map_or(Endpoint::Hub, |listing| Endpoint::Venue(listing.venue_id)))
}

/// Commits one command on the sequencer thread. Its reports, accepted or
/// rejected by the engine, arrive through the committed-event distributor;
/// only a failure to commit at all is answered here.
fn command_job(
    mut command: Command,
    actor: VerifiedActor,
    run_id: RunId,
    request_type: u8,
    reference: u64,
) -> JobWork<ServerMessage> {
    Box::new(move |context| {
        let service = ApplicationService::new(context.origin);
        command.expected_sequence = service
            .read(run_id, RunState::sequence)
            .map_err(|error| format!("run read failed: {error}"))?;
        command.logical_time = context.run_time(run_id)?;
        Ok(
            match service.execute_admitted(&actor, &command, context.admission) {
                Ok(_) => Vec::new(),
                Err(error) => vec![ServerMessage::Reject {
                    request_type,
                    reference,
                    reason: format!("command not committed: {error}"),
                }],
            },
        )
    })
}

/// A full or depth-limited snapshot of one listing, taken at its venue.
/// `next_report` is 1 for a feed's starting snapshot and 0 for a one-shot.
fn snapshot_job(
    request_id: u32,
    listing: Listing,
    depth: usize,
    next_report: u64,
    run_id: RunId,
    request_type: u8,
) -> JobWork<ServerMessage> {
    Box::new(move |context| {
        let projection = ApplicationService::new(context.origin)
            .read(run_id, |state| project_market(state, listing_key(listing)))
            .map_err(|error| format!("run read failed: {error}"))?;
        let levels = |levels: &[(i64, i64)]| {
            levels
                .iter()
                .take(depth)
                .map(|&(price, quantity)| Level { price, quantity })
                .collect()
        };
        Ok(vec![match projection {
            Ok(projection) => ServerMessage::MarketSnapshot {
                request_id,
                listing,
                next_report,
                bids: levels(&projection.bids),
                asks: levels(&projection.asks),
            },
            Err(_) => ServerMessage::Reject {
                request_type,
                reference: u64::from(request_id),
                reason: "unknown listing".to_owned(),
            },
        }])
    })
}

/// One connection's measured access latency: published, never used for
/// ordering (ADR 0035 §2).
struct Latency<'a> {
    admission: &'a AdmissionService,
    connection: u64,
    participant: ParticipantId,
    estimator: DelayEstimator,
    probes: BTreeMap<u64, u64>,
    next_probe_id: u64,
    next_probe_us: u64,
}

impl<'a> Latency<'a> {
    fn new(admission: &'a AdmissionService, participant: ParticipantId) -> Self {
        Self {
            admission,
            connection: admission.register_connection(),
            participant,
            estimator: DelayEstimator::new(),
            probes: BTreeMap::new(),
            next_probe_id: 0,
            next_probe_us: 0,
        }
    }

    const fn estimator(&self) -> &DelayEstimator {
        &self.estimator
    }

    const fn next_probe_us(&self) -> u64 {
        self.next_probe_us
    }

    fn delay_from(&self, source: Endpoint) -> Result<u64, String> {
        self.admission.delay_to_us(source, self.participant)
    }

    fn refresh_kernel(&mut self, stream: &TcpStream) {
        if let Some(reading) = crate::tcp_rtt::kernel_rtt(stream) {
            self.estimator.record_kernel_min(reading.min_rtt_us);
            self.report();
        }
    }

    fn probe(&mut self, channel: &mut Channel) -> Result<(), String> {
        let id = self.next_probe_id;
        self.next_probe_id = self.next_probe_id.wrapping_add(1);
        // Stamp as late as possible: just before the frame is written.
        let sent_us = self.admission.clock().now_us();
        channel.send(&[ServerMessage::Probe { probe_id: id }])?;
        self.probes.insert(id, sent_us);
        while self.probes.len() > MAX_OUTSTANDING_PROBES {
            self.probes.pop_first();
        }
        let base_us = self
            .admission
            .config()
            .probe_interval_ms
            .saturating_mul(1_000)
            .max(2);
        // Spread probes over 50–150% of the interval, as for FIX, so a
        // client that polls its socket still answers some at once.
        self.next_probe_us = sent_us
            .saturating_add(base_us / 2)
            .saturating_add(spread(self.connection, id) % base_us);
        Ok(())
    }

    fn on_response(&mut self, id: u64) {
        if let Some(sent_us) = self.probes.remove(&id) {
            self.estimator
                .record_probe(self.admission.clock().now_us().saturating_sub(sent_us));
            self.report();
        }
    }

    fn report(&self) {
        self.admission.report_connection(
            self.connection,
            ConnectionHealth {
                participant_id: self.participant.get().to_string(),
                rtt_source: self.estimator.source(),
                kernel_min_rtt_us: self.estimator.kernel_min_rtt_us(),
                probe_min_rtt_us: self.estimator.probe_min_rtt_us(),
                probe_samples: self.estimator.probe_samples(),
                access_one_way_us: self.estimator.one_way_us(),
            },
        );
    }
}

impl Drop for Latency<'_> {
    fn drop(&mut self) {
        self.admission.remove_connection(self.connection);
    }
}

/// `SplitMix64` of a connection and probe number: an even spread of probe
/// times. It never orders or times market events.
const fn spread(connection: u64, probe: u64) -> u64 {
    let mut z = (connection ^ probe.rotate_left(32)).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}
