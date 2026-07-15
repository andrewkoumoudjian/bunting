use crate::config::{AdminConfig, FixConfig, ServerConfig, StorageKind, TlsConfig};
use crate::storage::NativeOrigin;
use bunting_api_contract::{
    ActorIdentity, ActorRole, FIX_COMPETITION_PROFILE_VERSION, UnsignedDecimalString,
};
use bunting_application::{
    ApplicationService, FixApplicationRequest, FixApplicationSnapshot, FixApplicationState,
    FixCommandContext, VerifiedActor, project_market,
};
use bunting_command_transaction::InMemorySnapshotCache;
use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_types::{
    CorrelationId, IterationId, LogicalTimeNs, ParticipantId, PriceTicks, QuantityLots, RunId,
};
use bunting_origin_store::{OriginError, OriginStore};
use quarcc_execution_engine::ExecutionConfig;
use serde::{Deserialize, Serialize};
use simfix_mapping::{business_reject, market_snapshot};
use simfix_session::{FixSession, SessionAction, SessionConfig, SessionSnapshot};
use simfix_wire::{Decoder, FixMessage, WireLimits};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct NativeFixSnapshot {
    version: u16,
    session: SessionSnapshot,
    application: FixApplicationSnapshot,
}

pub fn run(config: &ServerConfig) -> Result<(), String> {
    config.validate().map_err(|error| error.to_string())?;
    let origin =
        NativeOrigin::from_config(&config.storage).map_err(|error| origin_error(&error))?;
    if let Some(scenario) = &config.scenario {
        let bytes = fs::read(&scenario.path).map_err(|error| {
            format!("cannot read immutable scenario {}: {error}", scenario.path)
        })?;
        if bytes.len() > 4 * 1_024 * 1_024 {
            return Err("scenario exceeds 4194304 bytes".to_owned());
        }
        let definition: ScenarioDefinition = serde_json::from_slice(&bytes)
            .map_err(|error| format!("invalid scenario JSON: {error}"))?;
        definition
            .validate()
            .map_err(|error| format!("scenario validation failed: {error:?}"))?;
        let run = RunState::from_scenario(
            RunId::new(scenario.run_id),
            IterationId::new(scenario.iteration_id),
            &definition,
        )
        .map_err(|error| format!("cannot create run from scenario: {error}"))?;
        match origin.load_run(run.run_id()) {
            Ok(existing) if existing.scenario_hash() == run.scenario_hash() => {}
            Ok(_) => {
                return Err(
                    "configured immutable scenario does not match the restored run hash".to_owned(),
                );
            }
            Err(OriginError::UnknownRun) => {
                origin
                    .insert_run(run)
                    .map_err(|error| origin_error(&error))?;
            }
            Err(error) => return Err(origin_error(&error)),
        }
    }
    let origin = Arc::new(origin);
    let cache = Arc::new(InMemorySnapshotCache::new());
    let mut threads = Vec::new();
    if let Some(admin) = config.admin.clone() {
        let origin = origin.clone();
        threads.push(thread::spawn(move || run_admin(&admin, &origin)));
    }
    if let Some(fix) = config.fix.clone() {
        let origin = origin.clone();
        let cache = cache.clone();
        let session_path = match config.storage.kind {
            StorageKind::File => config
                .storage
                .path
                .as_deref()
                .map(|path| PathBuf::from(path).with_extension("fix-session.json")),
            StorageKind::Memory => None,
        };
        threads.push(thread::spawn(move || {
            run_fix_acceptor(&fix, &origin, &cache, session_path.as_deref())
        }));
    }
    if threads.is_empty() {
        return Err("native profile requires at least one FIX or admin listener".to_owned());
    }
    for handle in threads {
        handle
            .join()
            .map_err(|_| "server listener thread panicked".to_owned())??;
    }
    Ok(())
}

fn run_fix_acceptor(
    config: &FixConfig,
    origin: &Arc<NativeOrigin>,
    cache: &Arc<InMemorySnapshotCache>,
    session_path: Option<&Path>,
) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind FIX listener {}: {error}", config.bind))?;
    let active = Arc::new(AtomicUsize::new(0));
    for accepted in listener.incoming() {
        let stream = accepted.map_err(|error| format!("FIX accept failed: {error}"))?;
        verify_terminated_peer(&stream, &config.tls, "FIX")?;
        let prior = active.fetch_add(1, Ordering::AcqRel);
        if prior >= config.max_connections {
            active.fetch_sub(1, Ordering::AcqRel);
            drop(stream);
            continue;
        }
        let config = config.clone();
        let origin = origin.clone();
        let cache = cache.clone();
        let active = active.clone();
        let session_path = session_path.map(Path::to_path_buf);
        thread::spawn(move || {
            if let Err(error) =
                handle_fix_connection(stream, &config, &origin, &cache, session_path.as_deref())
            {
                eprintln!("bunting-server: FIX connection closed: {error}");
            }
            active.fetch_sub(1, Ordering::AcqRel);
        });
    }
    Ok(())
}

fn verify_terminated_peer(
    stream: &TcpStream,
    tls: &TlsConfig,
    listener: &str,
) -> Result<(), String> {
    let TlsConfig::Terminated { trusted_proxy, .. } = tls else {
        return Ok(());
    };
    let expected = trusted_proxy
        .parse::<std::net::IpAddr>()
        .map_err(|_| format!("invalid trusted proxy for {listener}"))?;
    let actual = stream
        .peer_addr()
        .map_err(|error| format!("cannot inspect {listener} peer: {error}"))?
        .ip();
    if actual != expected {
        return Err(format!(
            "{listener} peer {actual} is not configured TLS terminator {expected}"
        ));
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "the socket/session/application loop keeps commit-before-response ordering visible"
)]
fn handle_fix_connection(
    mut stream: TcpStream,
    config: &FixConfig,
    origin: &NativeOrigin,
    cache: &InMemorySnapshotCache,
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
    let session_config = SessionConfig {
        sender_comp_id: config.sender_comp_id.clone(),
        target_comp_id: config.target_comp_id.clone(),
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
                FixApplicationState::new(ExecutionConfig::default()),
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
    let (logon_bytes, logon) = read_first_message(&mut stream, wire_limits)?;
    authenticate_logon(&logon, config)?;
    let timestamp = fix_timestamp();
    let millis = epoch_millis();
    let actions = session
        .receive_bytes_at(&logon_bytes, &timestamp, millis)
        .map_err(|error| format!("FIX Logon sequencing failed: {error:?}"))?;
    process_session_actions(actions, &mut stream, session_path, &session, &application)?;
    let mut response = FixMessage::new("A");
    response.push(98, "0");
    response.push(108, config.heartbeat_seconds.to_string());
    response.push(10000, FIX_COMPETITION_PROFILE_VERSION);
    send_messages(
        &mut session,
        &mut stream,
        [response],
        session_path,
        &application,
    )?;
    let actor = VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(config.participant_id),
        role: ActorRole::Participant,
        participant_id: Some(UnsignedDecimalString::new(config.participant_id)),
        team_id: None,
    })
    .map_err(|error| format!("invalid configured actor: {error}"))?;
    let service = ApplicationService::new(origin, cache);
    let mut buffer = vec![0; config.max_message_bytes.min(16_384)];
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                let actions = session
                    .poll(epoch_millis(), &fix_timestamp())
                    .map_err(|value| format!("FIX heartbeat failure: {value:?}"))?;
                process_session_actions(
                    actions,
                    &mut stream,
                    session_path,
                    &session,
                    &application,
                )?;
                continue;
            }
            Err(error) => return Err(format!("FIX socket read failed: {error}")),
        };
        let actions = session
            .receive_bytes_at(&buffer[..count], &fix_timestamp(), epoch_millis())
            .map_err(|error| format!("FIX session rejected bytes: {error:?}"))?;
        let mut applications = Vec::new();
        for action in actions {
            match action {
                SessionAction::Application(message) => applications.push(message),
                other => process_session_actions(
                    vec![other],
                    &mut stream,
                    session_path,
                    &session,
                    &application,
                )?,
            }
        }
        for message in applications {
            let state = service
                .recover(RunId::new(config.run_id))
                .map_err(|error| format!("run recovery failed: {error}"))?;
            let request = application.map_message(
                &message,
                &FixCommandContext {
                    actor: ParticipantId::new(config.participant_id),
                    run_id: RunId::new(config.run_id),
                    expected_sequence: state.sequence(),
                    logical_time: LogicalTimeNs::new(epoch_millis().saturating_mul(1_000_000)),
                    correlation_id: CorrelationId::new(u128::from(
                        session.snapshot().incoming_sequence,
                    )),
                },
            );
            let outbound = match request {
                Ok(FixApplicationRequest::Command(command)) => {
                    let executed = service
                        .execute(&actor, &command)
                        .map_err(|error| format!("application command failed: {error}"))?;
                    application
                        .committed_messages(
                            ParticipantId::new(config.participant_id),
                            &executed.events,
                        )
                        .map_err(|error| format!("FIX report mapping failed: {error}"))?
                }
                Ok(FixApplicationRequest::MarketData {
                    request_id,
                    instrument_id,
                    market_depth,
                    ..
                }) => {
                    let projection = project_market(&state, instrument_id)
                        .map_err(|error| format!("market projection failed: {error}"))?;
                    let bids = typed_levels(&projection.bids, market_depth);
                    let asks = typed_levels(&projection.asks, market_depth);
                    vec![market_snapshot(&request_id, instrument_id, &bids, &asks)]
                }
                Err(error) => vec![business_reject(&message.msg_type, &error.to_string())],
            };
            send_messages(
                &mut session,
                &mut stream,
                outbound,
                session_path,
                &application,
            )?;
        }
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
    let mut decoder = Decoder::new(limits);
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

fn authenticate_logon(message: &FixMessage, config: &FixConfig) -> Result<(), String> {
    if message.msg_type != "A"
        || message.value(49) != Some(config.target_comp_id.as_str())
        || message.value(56) != Some(config.sender_comp_id.as_str())
        || message.value(10000) != Some(FIX_COMPETITION_PROFILE_VERSION)
    {
        return Err("FIX Logon identity or Bunting profile is invalid".to_owned());
    }
    if !constant_time_eq(message.value(553).unwrap_or_default(), &config.username)
        || !constant_time_eq(message.value(554).unwrap_or_default(), &config.password)
    {
        return Err("FIX Logon credentials rejected".to_owned());
    }
    Ok(())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
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

fn run_admin(config: &AdminConfig, origin: &NativeOrigin) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind admin listener {}: {error}", config.bind))?;
    for accepted in listener.incoming() {
        let mut stream = accepted.map_err(|error| format!("admin accept failed: {error}"))?;
        handle_admin(&mut stream, config, origin)?;
    }
    Ok(())
}

fn handle_admin(
    stream: &mut TcpStream,
    config: &AdminConfig,
    origin: &NativeOrigin,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("cannot set admin timeout: {error}"))?;
    let mut bytes = vec![0; config.max_request_bytes];
    let count = stream
        .read(&mut bytes)
        .map_err(|error| format!("cannot read admin request: {error}"))?;
    let request = std::str::from_utf8(&bytes[..count]).unwrap_or_default();
    let first = request.lines().next().unwrap_or_default();
    if first == "GET /health HTTP/1.1" {
        return write_http(
            stream,
            200,
            &serde_json::json!({"status":"ok","service":crate::SERVICE_NAME}),
        );
    }
    if let Some(run) = first
        .strip_prefix("GET /admin/runs/")
        .and_then(|value| value.strip_suffix(" HTTP/1.1"))
    {
        let authorized = request.lines().any(|line| {
            line.strip_prefix("Authorization: Bearer ")
                .is_some_and(|value| constant_time_eq(value, &config.bearer_token))
        });
        if !authorized {
            return write_http(stream, 401, &serde_json::json!({"error":"unauthorized"}));
        }
        let run_id = run
            .parse::<u128>()
            .map_err(|_| "invalid admin run ID".to_owned())?;
        return match origin.load_run(RunId::new(run_id)) {
            Ok(state) => write_http(
                stream,
                200,
                &serde_json::json!({
                    "runId": state.run_id().to_string(),
                    "committedSequence": state.sequence().to_string(),
                    "eventSequence": state.event_sequence().to_string()
                }),
            ),
            Err(OriginError::UnknownRun) => {
                write_http(stream, 404, &serde_json::json!({"error":"unknown_run"}))
            }
            Err(error) => Err(origin_error(&error)),
        };
    }
    write_http(stream, 404, &serde_json::json!({"error":"not_found"}))
}

fn write_http(stream: &mut TcpStream, status: u16, body: &serde_json::Value) -> Result<(), String> {
    let body =
        serde_json::to_vec(body).map_err(|error| format!("cannot encode response: {error}"))?;
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|()| stream.write_all(&body))
        .map_err(|error| format!("cannot write admin response: {error}"))
}

fn epoch_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn fix_timestamp() -> String {
    let millis = epoch_millis();
    let seconds = millis / 1_000;
    let days = i64::try_from(seconds / 86_400).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(days);
    let day_seconds = seconds % 86_400;
    format!(
        "{year:04}{month:02}{day:02}-{:02}:{:02}:{:02}.{:03}",
        day_seconds / 3_600,
        (day_seconds % 3_600) / 60,
        day_seconds % 60,
        millis % 1_000
    )
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

fn origin_error(error: &OriginError) -> String {
    format!("origin store error: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_calendar_conversion_is_stable() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }
}
