use crate::admission::{AdmissionService, ConnectionHealth, Inbound, JobWork, Reply};
use crate::config::{FixConfig, RosterEntry};
use crate::distributor::PublishingOrigin;
use bunting_admission_sequencer::DelayEstimator;
use bunting_api_contract::{
    ActorIdentity, ActorRole, FIX_COMPETITION_PROFILE_VERSION, UnsignedDecimalString,
};
use bunting_application::{
    ApplicationService, FixApplicationRequest, FixApplicationSnapshot, FixApplicationState,
    FixCommandContext, VerifiedActor,
    competition::{account, discovery, news_tenders, risk_score},
    listing_for_command, project_market,
};
use bunting_engine::RunState;
use bunting_market_events::{SimulationCommand, SimulationCommandRequest, TenderDecision};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, LogicalTimeNs, ParticipantId, PriceTicks,
    QuantityLots, RunId, TenderId, VenueId,
};
use bunting_origin_store::OriginStore;
use quarcc_execution_engine::ExecutionConfig;
use serde::{Deserialize, Serialize};
use simfix_mapping::{
    ApplyFinePayload, CompetitionRequest, PublishNewsPayload, RunAdvancePayload, RunReasonPayload,
    TenderAction, business_reject, competition_report, market_snapshot,
};
use simfix_session::{FixSession, SessionAction, SessionConfig, SessionSnapshot};
use simfix_wire::{Decoder, FixMessage, WireLimits};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct NativeFixSnapshot {
    version: u16,
    session: SessionSnapshot,
    application: FixApplicationSnapshot,
}

#[expect(
    clippy::too_many_lines,
    reason = "the socket/session/application loop keeps commit-before-response ordering visible"
)]
pub(crate) fn handle_fix_connection(
    mut stream: TcpStream,
    config: &FixConfig,
    origin: &PublishingOrigin,
    admission: &AdmissionService,
    session_path: Option<&Path>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(60)))
        .map_err(|error| format!("cannot configure FIX read timeout: {error}"))?;
    let wire_limits = WireLimits {
        max_message_bytes: config.max_message_bytes,
        max_buffer_bytes: config.max_message_bytes.saturating_mul(2),
        ..WireLimits::default()
    };
    let (logon_bytes, logon) = read_first_message(&mut stream, wire_limits)?;
    let credential = authenticate_logon(&logon, config)?;
    let session_path = session_path
        .map(|base| base.with_extension(format!("fix-session-{}.json", credential.participant_id)));
    let session_path = session_path.as_deref();
    let session_config = SessionConfig {
        sender_comp_id: config.sender_comp_id.clone(),
        target_comp_id: credential.target_comp_id.clone(),
        heartbeat_seconds: config.heartbeat_seconds,
        max_journal_messages: config.max_journal_messages,
        max_pending_inbound: config.max_pending_inbound,
        wire_limits,
        logon_fields: Vec::new(),
    };
    let persisted = session_path.map(load_session).transpose()?.flatten();
    let (mut session, mut application) = persisted.map_or_else(
        || {
            Ok::<_, String>((
                FixSession::try_new(session_config.clone())
                    .map_err(|error| format!("invalid FIX session config: {error:?}"))?,
                FixApplicationState::new(ExecutionConfig::default())
                    .with_identity_epoch(new_identity_epoch()),
            ))
        },
        |snapshot| {
            if snapshot.version != 1 {
                return Err("unsupported native FIX snapshot version".to_owned());
            }
            Ok((
                FixSession::restore(session_config.clone(), snapshot.session)
                    .map_err(|error| format!("cannot restore FIX session: {error:?}"))?,
                FixApplicationState::restore(snapshot.application)
                    .map_err(|error| format!("cannot restore FIX application: {error}"))?,
            ))
        },
    )?;
    let timestamp = fix_timestamp();
    let millis = epoch_millis();
    let actions = session
        .receive_bytes_at(&logon_bytes, &timestamp, millis)
        .map_err(|error| format!("FIX Logon sequencing failed: {error:?}"))?;
    process_session_actions(actions, &mut stream, session_path, &session, &application)?;
    let mut response = FixMessage::new("A");
    response.push(98, "0");
    response.push(108, config.heartbeat_seconds.to_string());
    response.push(1137, simfix_wire::FIX_50_SP2_APPL_VER_ID);
    response.push(10000, FIX_COMPETITION_PROFILE_VERSION);
    response.push(10004, actor_role_name(credential.role));
    send_messages(
        &mut session,
        &mut stream,
        [response],
        session_path,
        &application,
    )?;
    let actor = VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(credential.participant_id),
        role: credential.role,
        participant_id: matches!(
            credential.role,
            ActorRole::Participant | ActorRole::BuiltInAgent
        )
        .then(|| UnsignedDecimalString::new(credential.participant_id)),
        team_id: None,
    })
    .map_err(|error| format!("invalid configured actor: {error}"))?;
    let participant = ParticipantId::new(credential.participant_id);
    let run_id = RunId::new(config.run_id);
    // Subscribe before handling any message so no committed batch for this
    // participant can fall between the subscription and the first command.
    let subscription = origin.distributor().subscribe()?;
    let mut latency = ConnectionLatency::new(admission, participant);
    latency.refresh_kernel(&stream);
    // ADR 0034 §2: measure the path before the first order matters.
    for _ in 0..LOGON_PROBES {
        latency.probe(&mut session, &mut stream, session_path, &application)?;
    }
    let (reply_sender, replies) = sync_channel(admission.config().max_admission_queue);
    let mut outbound = OutboundHold::new(admission.config().max_outbound_hold);
    let mut buffer = vec![0; config.max_message_bytes.min(16_384)];
    let mut rate_window_started = Instant::now();
    let rate_window = Duration::from_millis(config.rate_limit_window_ms);
    let mut rate_messages = 0_usize;
    let mut release_floor_us = 0_u64;
    loop {
        let now_us = admission.clock().now_us();
        let wait = [outbound.next_due_us(), Some(latency.next_probe_us())]
            .into_iter()
            .flatten()
            .min()
            .map_or(DELIVERY_POLL, |due| {
                Duration::from_micros(due.saturating_sub(now_us))
                    .clamp(Duration::from_micros(200), DELIVERY_POLL)
            });
        stream
            .set_read_timeout(Some(wait))
            .map_err(|error| format!("cannot configure FIX delivery poll: {error}"))?;
        match stream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                // ADR 0034 §4: stamp on read, before parsing; messages that
                // arrived together share the stamp.
                let received_us = admission.clock().now_us();
                let actions = session
                    .receive_bytes_at(&buffer[..count], &fix_timestamp(), epoch_millis())
                    .map_err(|error| format!("FIX session rejected bytes: {error:?}"))?;
                for action in actions {
                    match action {
                        SessionAction::Application(message) => {
                            if rate_window_started.elapsed() >= rate_window {
                                rate_window_started = Instant::now();
                                rate_messages = 0;
                            }
                            let rejection = if rate_messages >= config.max_messages_per_interval {
                                Some(format!(
                                    "max_messages_per_interval limit {}",
                                    config.max_messages_per_interval
                                ))
                            } else {
                                rate_messages = rate_messages.saturating_add(1);
                                admit_message(
                                    &message,
                                    &AdmitContext {
                                        origin,
                                        admission,
                                        actor: &actor,
                                        participant,
                                        run_id,
                                        received_us,
                                        request_id: u128::from(
                                            session.snapshot().incoming_sequence,
                                        ),
                                    },
                                    &mut application,
                                    &latency,
                                    &mut release_floor_us,
                                    &reply_sender,
                                )?
                            };
                            if let Some(reason) = rejection {
                                outbound.hold(
                                    business_reject(&message.msg_type, &reason),
                                    received_us.saturating_add(latency.hold_us(None)?),
                                )?;
                            }
                        }
                        SessionAction::TestResponse(id) => latency.on_response(&id),
                        SessionAction::PeerLogon(_) => {}
                        other => process_session_actions(
                            vec![other],
                            &mut stream,
                            session_path,
                            &session,
                            &application,
                        )?,
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(error) => return Err(format!("FIX socket read failed: {error}")),
        }
        let actions = session
            .poll(epoch_millis(), &fix_timestamp())
            .map_err(|value| format!("FIX heartbeat failure: {value:?}"))?;
        process_session_actions(actions, &mut stream, session_path, &session, &application)?;
        if admission.clock().now_us() >= latency.next_probe_us() {
            latency.refresh_kernel(&stream);
            latency.probe(&mut session, &mut stream, session_path, &application)?;
        }
        for reply in replies.try_iter() {
            let messages = reply.result?;
            let send_at = reply.completed_us.saturating_add(latency.hold_us(None)?);
            for message in messages {
                outbound.hold(message, send_at)?;
            }
        }
        // Every committed batch, whoever caused it, maps to this
        // participant's own reports (slice 12); ADR 0034 holds them so
        // every participant receives them at the same venue time.
        for batch in subscription.drain()? {
            let messages = application
                .committed_messages(participant, &batch.events)
                .map_err(|error| format!("FIX report mapping failed: {error}"))?;
            if messages.is_empty() {
                continue;
            }
            let send_at = batch
                .committed_us
                .saturating_add(latency.hold_us(batch_venue(&batch.events))?);
            for message in messages {
                outbound.hold(message, send_at)?;
            }
        }
        let due = outbound.take_due(admission.clock().now_us());
        if !due.is_empty() {
            send_messages(&mut session, &mut stream, due, session_path, &application)?;
        }
    }
}

/// Probes sent right after logon, before the participant's first order.
const LOGON_PROBES: usize = 3;
/// Outstanding probes per connection; older unanswered ones are dropped.
const MAX_OUTSTANDING_PROBES: usize = 16;

/// What [`admit_message`] needs from the connection, besides its state.
struct AdmitContext<'a> {
    origin: &'a PublishingOrigin,
    admission: &'a AdmissionService,
    actor: &'a VerifiedActor,
    participant: ParticipantId,
    run_id: RunId,
    received_us: u64,
    request_id: u128,
}

/// Maps one application message and admits it without waiting for its
/// release. Returns a rejection reason instead of an error when the message
/// is refused but the session stays up.
fn admit_message(
    message: &FixMessage,
    context: &AdmitContext<'_>,
    application: &mut FixApplicationState,
    latency: &ConnectionLatency<'_>,
    release_floor_us: &mut u64,
    replies: &SyncSender<Reply>,
) -> Result<Option<String>, String> {
    // Sequence and logical time are stamped at release (venue time); the
    // values here are placeholders the job overwrites.
    let request = application.map_message(
        message,
        &FixCommandContext {
            actor: context.participant,
            run_id: context.run_id,
            expected_sequence: EventSequence::new(0),
            logical_time: LogicalTimeNs::new(0),
            correlation_id: CorrelationId::new(context.request_id),
        },
    );
    let request = match request {
        Ok(request) => request,
        Err(error) => return Ok(Some(error.to_string())),
    };
    let venue = match &request {
        FixApplicationRequest::Command(command) => context
            .origin
            .read_run(context.run_id, |state| listing_for_command(state, command))
            .map_err(|error| format!("run read failed: {error}"))?
            .map(|listing| listing.venue_id),
        FixApplicationRequest::MarketData { listing_key, .. } => Some(listing_key.venue_id),
        FixApplicationRequest::Competition(_) => None,
    };
    let work = job_for(
        request,
        context.actor.clone(),
        context.run_id,
        context.request_id,
    );
    let inbound = Inbound {
        received_us: context.received_us,
        estimator: latency.estimator(),
        participant: context.participant,
        venue,
        floor_us: *release_floor_us,
    };
    match context.admission.admit(&inbound, work, replies.clone()) {
        Ok(record) => {
            *release_floor_us = record.release_us;
            Ok(None)
        }
        Err(error) => Ok(Some(error.to_string())),
    }
}

/// The work a released message performs on the sequencer thread.
fn job_for(
    request: FixApplicationRequest,
    actor: VerifiedActor,
    run_id: RunId,
    request_id: u128,
) -> JobWork {
    match request {
        // Execution reports for every affected participant arrive through
        // the committed-event distributor; per-participant limits are
        // engine risk.
        FixApplicationRequest::Command(mut command) => Box::new(move |context| {
            let service = ApplicationService::new(context.origin);
            command.expected_sequence = service
                .read(run_id, RunState::sequence)
                .map_err(|error| format!("run read failed: {error}"))?;
            command.logical_time = context.logical_time;
            service
                .execute_admitted(&actor, &command, context.admission)
                .map_err(|error| format!("application command failed: {error}"))?;
            Ok(Vec::new())
        }),
        FixApplicationRequest::MarketData {
            request_id,
            listing_key,
            market_depth,
            ..
        } => Box::new(move |context| {
            let projection = ApplicationService::new(context.origin)
                .read(run_id, |state| project_market(state, listing_key))
                .map_err(|error| format!("run read failed: {error}"))?
                .map_err(|error| format!("market projection failed: {error}"))?;
            let bids = typed_levels(&projection.bids, market_depth);
            let asks = typed_levels(&projection.asks, market_depth);
            Ok(vec![market_snapshot(
                &request_id,
                listing_key,
                &bids,
                &asks,
            )])
        }),
        FixApplicationRequest::Competition(request) => Box::new(move |context| {
            competition_messages(
                &ApplicationService::new(context.origin),
                &actor,
                run_id,
                &request,
                request_id,
            )
        }),
    }
}

/// The venue of the first event in a batch that names a listing.
fn batch_venue(events: &[bunting_market_events::EventEnvelope]) -> Option<VenueId> {
    use bunting_market_events::EventPayload;
    events.iter().find_map(|event| match &event.payload {
        EventPayload::OrderReceived { listing_key, .. }
        | EventPayload::OrderRested { listing_key, .. }
        | EventPayload::OrderCanceled { listing_key, .. }
        | EventPayload::TradeExecuted { listing_key, .. } => {
            listing_key.map(|listing| listing.venue_id)
        }
        _ => None,
    })
}

/// One connection's delay estimate and probes (ADR 0034 §2).
struct ConnectionLatency<'a> {
    admission: &'a AdmissionService,
    connection: u64,
    participant: ParticipantId,
    estimator: DelayEstimator,
    probes: BTreeMap<u64, u64>,
    next_probe_id: u64,
    next_probe_us: u64,
}

impl<'a> ConnectionLatency<'a> {
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

    fn hold_us(&self, venue: Option<VenueId>) -> Result<u64, String> {
        self.admission
            .outbound_hold_us(&self.estimator, self.participant, venue)
    }

    fn refresh_kernel(&mut self, stream: &TcpStream) {
        if self.admission.uses_kernel_rtt()
            && let Some(reading) = crate::tcp_rtt::kernel_rtt(stream)
        {
            self.estimator.record_kernel_min(reading.min_rtt_us);
            self.report();
        }
    }

    fn probe(
        &mut self,
        session: &mut FixSession,
        stream: &mut TcpStream,
        session_path: Option<&Path>,
        application: &FixApplicationState,
    ) -> Result<(), String> {
        let id = self.next_probe_id;
        self.next_probe_id = self.next_probe_id.wrapping_add(1);
        let actions = session
            .probe(&format!("rtt-{id}"), &fix_timestamp(), epoch_millis())
            .map_err(|error| format!("FIX latency probe failed: {error:?}"))?;
        // Stamp as late as possible: just before the frame is written.
        let sent_us = self.admission.clock().now_us();
        process_session_actions(actions, stream, session_path, session, application)?;
        self.probes.insert(id, sent_us);
        while self.probes.len() > MAX_OUTSTANDING_PROBES {
            self.probes.pop_first();
        }
        let interval_us = self
            .admission
            .config()
            .probe_interval_ms
            .saturating_mul(1_000);
        self.next_probe_us = sent_us.saturating_add(interval_us);
        Ok(())
    }

    fn on_response(&mut self, id: &str) {
        let Some(sent_us) = id
            .strip_prefix("rtt-")
            .and_then(|value| value.parse::<u64>().ok())
            .and_then(|key| self.probes.remove(&key))
        else {
            return;
        };
        self.estimator
            .record_probe(self.admission.clock().now_us().saturating_sub(sent_us));
        self.report();
    }

    fn report(&self) {
        let d_max = self.admission.config().policy.max_one_way_delay_us;
        let one_way = self.estimator.one_way_delay_us(d_max);
        let kernel = self.estimator.kernel_min_rtt_us();
        let probe = self.estimator.probe_min_rtt_us();
        self.admission.report_connection(
            self.connection,
            ConnectionHealth {
                participant_id: self.participant.get().to_string(),
                rtt_source: self.estimator.source(),
                kernel_min_rtt_us: kernel,
                probe_min_rtt_us: probe,
                probe_samples: self.estimator.probe_samples(),
                one_way_delay_us: one_way,
                beyond_max_one_way_delay: one_way >= d_max,
                probe_far_above_kernel: matches!(
                    (kernel, probe),
                    (Some(kernel), Some(probe)) if probe > kernel.saturating_mul(4).max(kernel + 5_000)
                ),
            },
        );
    }
}

impl Drop for ConnectionLatency<'_> {
    fn drop(&mut self) {
        self.admission.remove_connection(self.connection);
    }
}

/// Outbound messages waiting for their equalized send time (ADR 0034 §3).
struct OutboundHold {
    capacity: usize,
    next: u64,
    queue: BTreeMap<(u64, u64), FixMessage>,
}

impl OutboundHold {
    const fn new(capacity: usize) -> Self {
        Self {
            capacity,
            next: 0,
            queue: BTreeMap::new(),
        }
    }

    fn hold(&mut self, message: FixMessage, send_at_us: u64) -> Result<(), String> {
        if self.queue.len() >= self.capacity {
            return Err(format!(
                "max_outbound_hold limit {}: reconnect to recover",
                self.capacity
            ));
        }
        self.queue.insert((send_at_us, self.next), message);
        self.next = self.next.wrapping_add(1);
        Ok(())
    }

    fn next_due_us(&self) -> Option<u64> {
        self.queue.keys().next().map(|(due, _)| *due)
    }

    fn take_due(&mut self, now_us: u64) -> Vec<FixMessage> {
        let mut due = Vec::new();
        while let Some(entry) = self.queue.first_entry() {
            if entry.key().0 > now_us {
                break;
            }
            due.push(entry.remove());
        }
        due
    }
}

/// A value unique to each newly created FIX application state in this
/// process lifetime and across restarts: wall-clock nanoseconds at creation
/// plus a process-wide counter. It only namespaces identifiers; it never
/// orders or times market events.
fn new_identity_epoch() -> u128 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    (nanos << 64) | u128::from(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
}

/// Longest wait for inbound bytes before checking held messages, probes and
/// heartbeats.
const DELIVERY_POLL: Duration = Duration::from_millis(20);

/// Commits a competition mutation, if the request carries one, and then
/// builds the report from one borrowed read of the committed run.
fn competition_messages<O: OriginStore>(
    service: &ApplicationService<'_, O>,
    actor: &VerifiedActor,
    run_id: RunId,
    request: &CompetitionRequest,
    request_id: u128,
) -> Result<Vec<FixMessage>, String> {
    let mutation = match request {
        CompetitionRequest::Tender { action, tender_id } if *action != TenderAction::List => {
            let decision = if *action == TenderAction::Accept {
                TenderDecision::Accept
            } else {
                TenderDecision::Decline
            };
            Some((
                SimulationCommand::DecideTender {
                    tender_id: TenderId::new(
                        tender_id.ok_or_else(|| "missing tender identity".to_owned())?,
                    ),
                    decision,
                },
                "tender decision failed",
            ))
        }
        CompetitionRequest::RunControl {
            action,
            payload_json,
        }
        | CompetitionRequest::RiskAdmin {
            action,
            payload_json,
        } => Some((
            operator_command(action, payload_json.as_deref())?,
            "operator command failed",
        )),
        _ => None,
    };
    if let Some((payload, context)) = mutation {
        let (now, expected_sequence) = service
            .read(run_id, |state| {
                (state.simulation().clock.now, state.sequence())
            })
            .map_err(|error| error.to_string())?;
        let participant = match request {
            CompetitionRequest::Tender { .. } => actor
                .participant_id()
                .ok_or_else(|| "competition participant identity is unavailable".to_owned())?,
            _ => ParticipantId::new(actor.identity().actor_id.get()),
        };
        service
            .execute_simulation(
                actor,
                &SimulationCommandRequest {
                    run_id,
                    command_id: competition_command_id(request_id),
                    correlation_id: CorrelationId::new(request_id),
                    logical_time: now,
                    expected_sequence,
                    actor: participant,
                    payload,
                },
            )
            .map_err(|error| format!("{context}: {error}"))?;
    }
    let report = service
        .read(run_id, |state| {
            competition_report_for(state, actor, request)
        })
        .map_err(|error| error.to_string())??;
    Ok(vec![report])
}

/// Read-only report for one competition request over the committed run.
fn competition_report_for(
    state: &RunState,
    actor: &VerifiedActor,
    request: &CompetitionRequest,
) -> Result<FixMessage, String> {
    match request {
        CompetitionRequest::Discovery => competition_report(
            "y",
            "public",
            "discovery",
            "snapshot",
            "ok",
            state.sequence().get(),
            &discovery(state),
        ),
        CompetitionRequest::Account => competition_report(
            "AP",
            "private",
            "account",
            "snapshot",
            "ok",
            state.sequence().get(),
            &account(state, actor).map_err(|error| error.to_string())?,
        ),
        CompetitionRequest::News => competition_report(
            "B",
            "private",
            "news",
            "list",
            "ok",
            state.sequence().get(),
            &news_tenders(state, actor)
                .map_err(|error| error.to_string())?
                .news,
        ),
        CompetitionRequest::Tender { .. } => {
            actor
                .participant_id()
                .ok_or_else(|| "competition participant identity is unavailable".to_owned())?;
            competition_report(
                "U6",
                "private",
                "tender",
                "list",
                "ok",
                state.sequence().get(),
                &news_tenders(state, actor)
                    .map_err(|error| error.to_string())?
                    .tenders,
            )
        }
        CompetitionRequest::Score => competition_report(
            "U9",
            "private",
            "score",
            "snapshot",
            "ok",
            state.sequence().get(),
            &risk_score(state, actor)
                .map_err(|error| error.to_string())?
                .latest_score,
        ),
        CompetitionRequest::Risk => competition_report(
            "UB",
            "private",
            "risk",
            "snapshot",
            "ok",
            state.sequence().get(),
            &risk_score(state, actor).map_err(|error| error.to_string())?,
        ),
        CompetitionRequest::RunControl { action, .. }
        | CompetitionRequest::RiskAdmin { action, .. } => competition_report(
            "UA",
            "admin",
            "run_control",
            action,
            "committed",
            state.sequence().get(),
            &discovery(state),
        ),
    }
    .map_err(|error| format!("competition report mapping failed: {error:?}"))
}

const fn competition_command_id(sequence: u128) -> CommandId {
    CommandId::new((1_u128 << 127) | sequence)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenTenderPayload {
    tender_id: TenderId,
    participant_id: ParticipantId,
    instrument_id: bunting_market_types::InstrumentId,
    side: bunting_market_events::Side,
    quantity: QuantityLots,
    price: PriceTicks,
    expires_at: LogicalTimeNs,
}

fn operator_command(action: &str, payload_json: Option<&str>) -> Result<SimulationCommand, String> {
    let payload = payload_json.unwrap_or("{}");
    match action {
        "start" => Ok(SimulationCommand::StartRun),
        "pause" => Ok(SimulationCommand::PauseRun),
        "resume" => Ok(SimulationCommand::ResumeRun),
        "advance" => serde_json::from_str::<RunAdvancePayload>(payload)
            .map(|value| SimulationCommand::Advance { steps: value.steps })
            .map_err(|error| format!("invalid advance payload: {error}")),
        "terminate" => serde_json::from_str::<RunReasonPayload>(payload)
            .map(|value| SimulationCommand::Terminate {
                reason: value.reason,
            })
            .map_err(|error| format!("invalid terminate payload: {error}")),
        "publish_news" => serde_json::from_str::<PublishNewsPayload>(payload)
            .map(|value| SimulationCommand::PublishNews {
                news_id: value.news_id,
                audience: value.audience,
                headline: value.headline,
                body: value.body,
            })
            .map_err(|error| format!("invalid news payload: {error}")),
        "open_tender" => serde_json::from_str::<OpenTenderPayload>(payload)
            .map(|value| SimulationCommand::OpenTender {
                tender_id: value.tender_id,
                participant_id: value.participant_id,
                instrument_id: value.instrument_id,
                side: value.side,
                quantity: value.quantity,
                price: value.price,
                expires_at: value.expires_at,
            })
            .map_err(|error| format!("invalid tender payload: {error}")),
        "score" => Ok(SimulationCommand::ScoreIteration),
        "fine" => serde_json::from_str::<ApplyFinePayload>(payload)
            .map(|value| SimulationCommand::ApplyFine {
                participant_id: value.participant_id,
                currency_id: value.currency_id,
                amount: value.amount,
                reason: value.reason,
            })
            .map_err(|error| format!("invalid fine payload: {error}")),
        _ => Err(format!("unsupported operator action {action}")),
    }
}

fn typed_levels(levels: &[(i64, i64)], depth: usize) -> Vec<(PriceTicks, QuantityLots)> {
    levels
        .iter()
        .take(depth)
        .map(|(price, quantity)| (PriceTicks::new(*price), QuantityLots::new(*quantity)))
        .collect()
}

fn read_first_message(
    stream: &mut TcpStream,
    limits: WireLimits,
) -> Result<(Vec<u8>, FixMessage), String> {
    let mut decoder = Decoder::try_new(limits)
        .map_err(|error| format!("cannot load FIX dictionaries: {error:?}"))?;
    let mut collected = Vec::new();
    let mut buffer = vec![0; limits.max_message_bytes.min(8_192)];
    loop {
        let count = stream
            .read(&mut buffer)
            .map_err(|error| format!("cannot read FIX Logon: {error}"))?;
        if count == 0 {
            return Err("peer disconnected before FIX Logon".to_owned());
        }
        collected.extend_from_slice(&buffer[..count]);
        if collected.len() > limits.max_message_bytes {
            return Err("FIX Logon exceeds max_message_bytes".to_owned());
        }
        let messages = decoder
            .push(&buffer[..count])
            .map_err(|error| format!("invalid FIX Logon framing: {error:?}"))?;
        if let Some(message) = messages.into_iter().next() {
            return Ok((collected, message));
        }
    }
}

fn authenticate_logon<'a>(
    message: &FixMessage,
    config: &'a FixConfig,
) -> Result<&'a RosterEntry, String> {
    let credential = config
        .roster
        .iter()
        .find(|entry| {
            message.value(49) == Some(entry.target_comp_id.as_str())
                && constant_time_eq(message.value(553).unwrap_or_default(), &entry.username)
        })
        .ok_or_else(|| "FIX Logon credentials rejected".to_owned())?;
    if message.msg_type != "A"
        || message.value(56) != Some(config.sender_comp_id.as_str())
        || message.value(1137) != Some(simfix_wire::FIX_50_SP2_APPL_VER_ID)
        || message.value(10000) != Some(FIX_COMPETITION_PROFILE_VERSION)
        || message.value(10004) != Some(actor_role_name(credential.role))
    {
        return Err("FIX Logon identity or Bunting profile is invalid".to_owned());
    }
    if !constant_time_eq(message.value(554).unwrap_or_default(), &credential.password) {
        return Err("FIX Logon credentials rejected".to_owned());
    }
    Ok(credential)
}

const fn actor_role_name(role: ActorRole) -> &'static str {
    match role {
        ActorRole::Participant => "participant",
        ActorRole::Team => "team",
        ActorRole::Instructor => "instructor",
        ActorRole::Administrator => "administrator",
        ActorRole::BuiltInAgent => "built_in_agent",
    }
}

pub(crate) fn constant_time_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..256 {
        difference |= usize::from(
            left.as_bytes().get(index).copied().unwrap_or_default()
                ^ right.as_bytes().get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0 && left.len() <= 256 && right.len() <= 256
}

fn send_messages(
    session: &mut FixSession,
    stream: &mut TcpStream,
    messages: impl IntoIterator<Item = FixMessage>,
    session_path: Option<&Path>,
    application: &FixApplicationState,
) -> Result<(), String> {
    for message in messages {
        let actions = session
            .send_application(message, &fix_timestamp())
            .map_err(|error| format!("cannot sequence FIX response: {error:?}"))?;
        process_session_actions(actions, stream, session_path, session, application)?;
    }
    persist_session(session_path, session, application)
}

fn process_session_actions(
    actions: Vec<SessionAction>,
    stream: &mut TcpStream,
    session_path: Option<&Path>,
    session: &FixSession,
    application: &FixApplicationState,
) -> Result<(), String> {
    for action in actions {
        match action {
            SessionAction::Send(frame) => stream
                .write_all(&frame)
                .map_err(|error| format!("FIX socket write failed: {error}"))?,
            SessionAction::Persist(_) => persist_session(session_path, session, application)?,
            SessionAction::Disconnect => return Err("FIX session requested disconnect".to_owned()),
            SessionAction::Application(_) => {
                return Err("application action must be handled by caller".to_owned());
            }
            SessionAction::PeerLogon(_) | SessionAction::TestResponse(_) => {}
        }
    }
    Ok(())
}

fn persist_session(
    path: Option<&Path>,
    session: &FixSession,
    application: &FixApplicationState,
) -> Result<(), String> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = path.parent().filter(|value| !value.as_os_str().is_empty()) {
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create FIX snapshot directory: {error}"))?;
    }
    let snapshot = NativeFixSnapshot {
        version: 1,
        session: session.snapshot(),
        application: application.snapshot(),
    };
    let bytes = serde_json::to_vec(&snapshot)
        .map_err(|error| format!("cannot encode FIX snapshot: {error}"))?;
    let temporary = path.with_extension("tmp");
    let mut file =
        File::create(&temporary).map_err(|error| format!("cannot create FIX snapshot: {error}"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("cannot persist FIX snapshot: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("cannot install FIX snapshot: {error}"))
}

fn load_session(path: &Path) -> Result<Option<NativeFixSnapshot>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|error| format!("cannot read FIX snapshot: {error}"))?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("invalid FIX snapshot: {error}"))
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn fix_timestamp() -> String {
    let format = time::macros::format_description!(
        "[year][month][day]-[hour]:[minute]:[second].[subsecond digits:3]"
    );
    match time::OffsetDateTime::now_utc().format(format) {
        Ok(value) => value,
        Err(_) => "19700101-00:00:00.000".to_owned(),
    }
}
