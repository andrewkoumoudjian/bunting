use crate::{execute_command_detailed, load_run, snapshot_output};
use bunting_market_events::{CancelOrder, Command, CommandPayload, EventPayload, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, LogicalTimeNs, OrderId, ParticipantId,
    RunId,
};
use quarcc_execution_engine::event::ExecutionAction;
use quarcc_execution_engine::ids::{IntentId, LocalOrderId, ReportId, VenueOrderId};
use quarcc_execution_engine::{
    ExecutionActionBuffer, ExecutionConfig, ExecutionEngine, ExecutionIntent, ExecutionSnapshot,
    NormalizedVenueReport, QuarccExecutionEngine, VenueReportKind,
};
use serde::{Deserialize, Serialize};
use simfix_mapping::{
    ExecutionMode, InboundApplication, MappingContext, business_reject, map_execution_report,
    map_inbound, market_snapshot,
};
use simfix_session::{FixSession, SessionAction, SessionConfig, SessionSnapshot};
use simfix_wire::{FixMessage, WireLimits};
use std::cell::RefCell;
use std::collections::BTreeMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use worker::{DurableObject, Env, Error, Request, Response, Result, Socket, State, durable_object};

const MAX_SOCKET_READ: usize = 65_536;
const MAX_APPLICATION_MESSAGES_PER_READ: usize = 64;
const MAX_FIX_RESPONSES_PER_READ: usize = 256;
const STORAGE_KEY: &str = "fix-session-v2";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConnectRequest {
    hostname: String,
    port: u16,
    sender_comp_id: String,
    target_comp_id: String,
    heartbeat_seconds: u32,
    timestamp: String,
    now_millis: u64,
    run_id: String,
    execution_mode: ExecutionMode,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SendRequest {
    timestamp: String,
    message: FixMessage,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PumpRequest {
    timestamp: String,
    now_millis: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct StoredFixSession {
    hostname: String,
    port: u16,
    config: SessionConfig,
    session: SessionSnapshot,
    execution_mode: ExecutionMode,
    execution: Option<ExecutionSnapshot>,
    run_id: RunId,
    participant_id: ParticipantId,
    next_intent_id: u128,
    logical_time_ns: u64,
    bunting_recovery_cursor: EventSequence,
    order_instruments: BTreeMap<OrderId, InstrumentId>,
}

#[durable_object]
pub struct FixSessionObject {
    state: State,
    environment: Env,
    socket: RefCell<Option<Socket>>,
}

impl DurableObject for FixSessionObject {
    fn new(state: State, environment: Env) -> Self {
        Self {
            state,
            environment,
            socket: RefCell::new(None),
        }
    }

    async fn fetch(&self, mut request: Request) -> Result<Response> {
        let path = request.url()?.path().to_owned();
        if path.ends_with("/connect") {
            let input: ConnectRequest = request.json().await?;
            return self.connect(input).await;
        }
        if path.ends_with("/send") {
            let input: SendRequest = request.json().await?;
            return self.send(input).await;
        }
        if path.ends_with("/pump") {
            let input: PumpRequest = request.json().await?;
            return self.pump(input).await;
        }
        if path.ends_with("/snapshot") {
            let stored: Option<StoredFixSession> = self.state.storage().get(STORAGE_KEY).await?;
            return Response::from_json(&stored);
        }
        Response::error("FIX session operation not found", 404)
    }
}

impl FixSessionObject {
    async fn connect(&self, input: ConnectRequest) -> Result<Response> {
        if input.hostname.is_empty()
            || input.hostname.len() > 253
            || input.heartbeat_seconds == 0
            || !self.destination_allowed(&input.hostname, input.port)?
        {
            return Response::error("invalid or disallowed FIX connection configuration", 400);
        }
        let run_id = input
            .run_id
            .parse::<u128>()
            .map(RunId::new)
            .map_err(|_| Error::RustError("invalid FIX run identity".to_owned()))?;
        let participant_id = self
            .environment
            .secret("BUNTING_API_PARTICIPANT_ID")?
            .to_string()
            .parse::<u128>()
            .map(ParticipantId::new)
            .map_err(|_| Error::RustError("invalid configured participant claim".to_owned()))?;
        let database = self.environment.d1("ORIGIN_DB")?;
        let authoritative_run = crate::d1_origin::load_run(&database, &run_id.to_string())
            .await
            .map_err(|error| Error::RustError(format!("FIX run recovery failed: {error:?}")))?;
        let config = SessionConfig {
            sender_comp_id: input.sender_comp_id,
            target_comp_id: input.target_comp_id,
            heartbeat_seconds: input.heartbeat_seconds,
            max_journal_messages: 4_096,
            max_pending_inbound: 256,
            wire_limits: WireLimits::default(),
        };
        let previous: Option<StoredFixSession> = self.state.storage().get(STORAGE_KEY).await?;
        if previous.as_ref().is_some_and(|stored| {
            stored.run_id != run_id
                || stored.participant_id != participant_id
                || stored.hostname != input.hostname
                || stored.port != input.port
                || stored.execution_mode != input.execution_mode
                || stored.config.sender_comp_id != config.sender_comp_id
                || stored.config.target_comp_id != config.target_comp_id
                || stored.bunting_recovery_cursor != authoritative_run.sequence()
        }) {
            return Response::error(
                "FIX session identity/configuration changed or its Bunting recovery cursor requires reconciliation",
                409,
            );
        }
        let mut session = if let Some(stored) = &previous {
            FixSession::restore(config.clone(), stored.session.clone())
                .map_err(|error| session_error(&error))?
        } else {
            FixSession::try_new(config.clone()).map_err(|error| session_error(&error))?
        };
        let mut socket = Socket::builder()
            .allow_half_open(false)
            .connect(input.hostname.clone(), input.port)?;
        socket.opened().await?;
        let logon_actions = session
            .connected_at(&input.timestamp, input.now_millis)
            .map_err(|error| session_error(&error))?;
        let execution = match (input.execution_mode, previous.as_ref()) {
            (ExecutionMode::QuarccManaged, Some(stored)) => stored.execution.clone(),
            (ExecutionMode::QuarccManaged, None) => {
                Some(QuarccExecutionEngine::new(ExecutionConfig::default()).snapshot())
            }
            (ExecutionMode::Direct, _) => None,
        };
        let stored = StoredFixSession {
            hostname: input.hostname,
            port: input.port,
            config,
            session: session.snapshot(),
            execution_mode: input.execution_mode,
            execution,
            run_id,
            participant_id,
            next_intent_id: previous.as_ref().map_or(1, |value| value.next_intent_id),
            logical_time_ns: previous.as_ref().map_or(0, |value| value.logical_time_ns),
            bunting_recovery_cursor: previous
                .as_ref()
                .map_or(authoritative_run.sequence(), |value| {
                    value.bunting_recovery_cursor
                }),
            order_instruments: previous
                .as_ref()
                .map_or_else(BTreeMap::new, |value| value.order_instruments.clone()),
        };
        self.state.storage().put(STORAGE_KEY, &stored).await?;
        write_actions(&mut socket, &logon_actions).await?;
        self.socket.replace(Some(socket));
        Response::from_json(&stored)
    }

    fn destination_allowed(&self, hostname: &str, port: u16) -> Result<bool> {
        let configured = self
            .environment
            .var("BUNTING_FIX_DESTINATIONS")?
            .to_string();
        let candidate = format!("{hostname}:{port}");
        Ok(configured
            .split(',')
            .map(str::trim)
            .any(|allowed| allowed == candidate))
    }

    async fn send(&self, input: SendRequest) -> Result<Response> {
        let mut stored = self.load_stored().await?;
        let mut session = FixSession::restore(stored.config.clone(), stored.session.clone())
            .map_err(|error| session_error(&error))?;
        let actions = session
            .send_application(input.message, &input.timestamp)
            .map_err(|error| session_error(&error))?;
        stored.session = session.snapshot();
        self.state.storage().put(STORAGE_KEY, &stored).await?;
        self.write_to_socket(&actions).await?;
        Response::from_json(&stored)
    }

    async fn pump(&self, input: PumpRequest) -> Result<Response> {
        let mut stored = self.load_stored().await?;
        let mut session = FixSession::restore(stored.config.clone(), stored.session.clone())
            .map_err(|error| session_error(&error))?;
        let bytes = self.read_from_socket().await?;
        let actions = session
            .receive_bytes_at(&bytes, &input.timestamp, input.now_millis)
            .map_err(|error| session_error(&error))?;
        let applications: Vec<_> = actions
            .iter()
            .filter_map(|action| match action {
                SessionAction::Application(message) => Some(message.clone()),
                _ => None,
            })
            .collect();
        if applications.len() > MAX_APPLICATION_MESSAGES_PER_READ {
            return Err(Error::RustError(
                "FIX application batch exceeds configured bound".to_owned(),
            ));
        }
        stored.session = session.snapshot();
        self.state.storage().put(STORAGE_KEY, &stored).await?;
        self.write_to_socket(&actions).await?;

        let mut responses = Vec::new();
        for message in applications {
            responses.extend(self.execute_application(&mut stored, &message).await);
            if responses.len() > MAX_FIX_RESPONSES_PER_READ {
                return Err(Error::RustError(
                    "FIX response batch exceeds configured bound".to_owned(),
                ));
            }
        }
        for response in &responses {
            let outbound = session
                .send_application(response.clone(), &input.timestamp)
                .map_err(|error| session_error(&error))?;
            stored.session = session.snapshot();
            self.state.storage().put(STORAGE_KEY, &stored).await?;
            self.write_to_socket(&outbound).await?;
        }
        stored.session = session.snapshot();
        self.state.storage().put(STORAGE_KEY, &stored).await?;
        Response::from_json(&responses)
    }

    async fn execute_application(
        &self,
        stored: &mut StoredFixSession,
        message: &FixMessage,
    ) -> Vec<FixMessage> {
        let context = MappingContext {
            participant_id: stored.participant_id,
            next_intent_id: IntentId::new(stored.next_intent_id),
        };
        stored.next_intent_id = stored.next_intent_id.saturating_add(1);
        let application = match map_inbound(message, context) {
            Ok(application) => application,
            Err(error) => {
                return vec![business_reject(
                    &message.msg_type,
                    &format!("invalid supported FIX 4.4 message: {error:?}"),
                )];
            }
        };
        match application {
            InboundApplication::MarketDataRequest {
                request_id,
                instrument_id,
                ..
            } => match load_run(&self.environment, stored.run_id, instrument_id)
                .await
                .and_then(|state| snapshot_output(&state, instrument_id))
            {
                Ok(snapshot) => vec![market_snapshot(
                    &request_id,
                    instrument_id,
                    &snapshot
                        .bids
                        .iter()
                        .map(|level| {
                            (
                                bunting_market_types::PriceTicks::new(level.price_ticks.get()),
                                bunting_market_types::QuantityLots::new(level.quantity_lots.get()),
                            )
                        })
                        .collect::<Vec<_>>(),
                    &snapshot
                        .asks
                        .iter()
                        .map(|level| {
                            (
                                bunting_market_types::PriceTicks::new(level.price_ticks.get()),
                                bunting_market_types::QuantityLots::new(level.quantity_lots.get()),
                            )
                        })
                        .collect::<Vec<_>>(),
                )],
                Err(error) => vec![business_reject(
                    &message.msg_type,
                    &format!("market snapshot unavailable: {error:?}"),
                )],
            },
            InboundApplication::Intent(intent) => match stored.execution_mode {
                ExecutionMode::Direct => self.execute_direct(stored, intent, message).await,
                ExecutionMode::QuarccManaged => self.execute_managed(stored, intent, message).await,
            },
        }
    }

    async fn execute_direct(
        &self,
        stored: &mut StoredFixSession,
        intent: ExecutionIntent,
        original: &FixMessage,
    ) -> Vec<FixMessage> {
        let command = match command_for_intent(stored, &intent) {
            Ok(command) => command,
            Err(reason) => return vec![business_reject(&original.msg_type, reason)],
        };
        let Some(instrument_id) = command_instrument(&command, stored) else {
            return vec![business_reject(
                &original.msg_type,
                "instrument identity is unavailable for this order",
            )];
        };
        match execute_command_detailed(command.clone(), instrument_id, &self.environment).await {
            Ok(executed) => {
                stored.bunting_recovery_cursor = executed.result.committed_sequence;
                reports_for_command(&command, &executed)
                    .iter()
                    .filter_map(|report| map_execution_report(report).ok())
                    .collect()
            }
            Err(error) => vec![business_reject(
                &original.msg_type,
                &format!("Bunting command failed: {error:?}"),
            )],
        }
    }

    async fn execute_managed(
        &self,
        stored: &mut StoredFixSession,
        intent: ExecutionIntent,
        original: &FixMessage,
    ) -> Vec<FixMessage> {
        let Some(snapshot) = stored.execution.take() else {
            return vec![business_reject(
                &original.msg_type,
                "managed execution snapshot is unavailable",
            )];
        };
        let mut engine = match QuarccExecutionEngine::restore(snapshot) {
            Ok(engine) => engine,
            Err(error) => {
                return vec![business_reject(
                    &original.msg_type,
                    &format!("managed execution restore failed: {error:?}"),
                )];
            }
        };
        let limit = engine.snapshot().config.max_actions_per_call;
        let mut actions = ExecutionActionBuffer::with_limit(limit);
        if let Err(error) = engine.submit_intent(intent, &mut actions) {
            stored.execution = Some(engine.snapshot());
            return vec![business_reject(
                &original.msg_type,
                &format!("managed intent rejected: {error:?}"),
            )];
        }
        let mut responses = Vec::new();
        for action in actions.into_vec() {
            let command = match command_for_action(stored, &action) {
                Ok(command) => command,
                Err(reason) => {
                    responses.push(business_reject(&original.msg_type, reason));
                    continue;
                }
            };
            let Some(instrument_id) = command_instrument(&command, stored) else {
                responses.push(business_reject(
                    &original.msg_type,
                    "instrument identity is unavailable for this order",
                ));
                continue;
            };
            match execute_command_detailed(command.clone(), instrument_id, &self.environment).await
            {
                Ok(executed) => {
                    stored.bunting_recovery_cursor = executed.result.committed_sequence;
                    for report in reports_for_command(&command, &executed) {
                        let mut followups = ExecutionActionBuffer::with_limit(limit);
                        if let Err(error) = engine.apply_venue_report(&report, &mut followups) {
                            responses.push(business_reject(
                                &original.msg_type,
                                &format!("managed report rejected: {error:?}"),
                            ));
                        } else if let Ok(response) = map_execution_report(&report) {
                            responses.push(response);
                        }
                    }
                }
                Err(error) => responses.push(business_reject(
                    &original.msg_type,
                    &format!("Bunting command failed: {error:?}"),
                )),
            }
        }
        stored.execution = Some(engine.snapshot());
        responses
    }

    async fn load_stored(&self) -> Result<StoredFixSession> {
        self.state
            .storage()
            .get(STORAGE_KEY)
            .await?
            .ok_or_else(|| Error::RustError("FIX session is not configured".to_owned()))
    }

    async fn read_from_socket(&self) -> Result<Vec<u8>> {
        let mut socket = self
            .socket
            .replace(None)
            .ok_or_else(|| Error::RustError("FIX socket reconnect required".to_owned()))?;
        let mut bytes = vec![0_u8; MAX_SOCKET_READ];
        let result = socket
            .read(&mut bytes)
            .await
            .map_err(|error| Error::RustError(error.to_string()));
        self.socket.replace(Some(socket));
        let read = result?;
        bytes.truncate(read);
        Ok(bytes)
    }

    async fn write_to_socket(&self, actions: &[SessionAction]) -> Result<()> {
        let mut socket = self
            .socket
            .replace(None)
            .ok_or_else(|| Error::RustError("FIX socket reconnect required".to_owned()))?;
        let result = write_actions(&mut socket, actions).await;
        self.socket.replace(Some(socket));
        result
    }
}

fn command_for_intent(
    stored: &mut StoredFixSession,
    intent: &ExecutionIntent,
) -> std::result::Result<Command, &'static str> {
    let (intent_id, payload) = match intent {
        ExecutionIntent::Submit { intent_id, order } => (
            *intent_id,
            CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(order.client_order_id.get()),
                instrument_id: order.instrument_id,
                participant_id: stored.participant_id,
                side: order.side,
                quantity: order.quantity,
                kind: order.kind,
            }),
        ),
        ExecutionIntent::Cancel {
            intent_id,
            client_order_id,
        } => (
            *intent_id,
            CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(client_order_id.get()),
                participant_id: stored.participant_id,
            }),
        ),
        ExecutionIntent::Replace { .. } => {
            return Err("replace is not enabled by the venue command surface");
        }
        ExecutionIntent::Query { .. } => {
            return Err("order status recovery is not available in this slice");
        }
        ExecutionIntent::ActivateKillSwitch { .. } => {
            return Err("kill switch is not a supported FIX application message");
        }
    };
    Ok(command_envelope(stored, intent_id, payload))
}

fn command_for_action(
    stored: &mut StoredFixSession,
    action: &ExecutionAction,
) -> std::result::Result<Command, &'static str> {
    let (action_id, payload) = match action {
        ExecutionAction::Submit {
            action_id,
            local_order_id,
            order,
        } => (
            action_id.get(),
            CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(local_order_id.get()),
                instrument_id: order.instrument_id,
                participant_id: stored.participant_id,
                side: order.side,
                quantity: order.quantity,
                kind: order.kind,
            }),
        ),
        ExecutionAction::Cancel {
            action_id,
            local_order_id,
            ..
        } => (
            action_id.get(),
            CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(local_order_id.get()),
                participant_id: stored.participant_id,
            }),
        ),
        ExecutionAction::Replace { .. } => {
            return Err("replace is not enabled by the venue command surface");
        }
        ExecutionAction::QueryOrder { .. } | ExecutionAction::QueryOpenOrders { .. } => {
            return Err("venue query action is not implemented by the origin adapter");
        }
    };
    Ok(command_envelope(stored, IntentId::new(action_id), payload))
}

fn command_envelope(
    stored: &mut StoredFixSession,
    id: IntentId,
    payload: CommandPayload,
) -> Command {
    if let CommandPayload::SubmitOrder(order) = &payload {
        stored
            .order_instruments
            .insert(order.order_id, order.instrument_id);
    }
    stored.logical_time_ns = stored.logical_time_ns.saturating_add(1);
    Command {
        run_id: stored.run_id,
        command_id: CommandId::new(id.get()),
        correlation_id: CorrelationId::new(id.get()),
        logical_time: LogicalTimeNs::new(stored.logical_time_ns),
        expected_sequence: stored.bunting_recovery_cursor,
        actor: stored.participant_id,
        payload,
    }
}

fn command_instrument(command: &Command, stored: &StoredFixSession) -> Option<InstrumentId> {
    match &command.payload {
        CommandPayload::SubmitOrder(order) => Some(order.instrument_id),
        CommandPayload::CancelOrder(order) => {
            stored.order_instruments.get(&order.order_id).copied()
        }
        CommandPayload::ActivateKillSwitch | CommandPayload::NbcDone(_) => None,
    }
}

fn reports_for_command(
    command: &Command,
    executed: &crate::ExecutedCommand,
) -> Vec<NormalizedVenueReport> {
    let order_id = match &command.payload {
        CommandPayload::SubmitOrder(order) => order.order_id,
        CommandPayload::CancelOrder(order) => order.order_id,
        CommandPayload::ActivateKillSwitch | CommandPayload::NbcDone(_) => OrderId::new(0),
    };
    let local = LocalOrderId::new(order_id.get());
    let client = quarcc_execution_engine::ids::ClientOrderId::new(order_id.get());
    let total_quantity = match &command.payload {
        CommandPayload::SubmitOrder(order) => Some(order.quantity.get()),
        _ => None,
    };
    let mut cumulative_quantity = 0_i64;
    let mut reports = Vec::new();
    for event in &executed.events {
        let mapped = match &event.payload {
            EventPayload::OrderAccepted { order_id: accepted } if *accepted == order_id => Some((
                VenueReportKind::Accepted,
                total_quantity.map(bunting_market_types::QuantityLots::new),
            )),
            EventPayload::OrderRejected {
                order_id: Some(rejected),
                code,
            } if *rejected == order_id => Some((
                VenueReportKind::Rejected {
                    reason: format!("{code:?}"),
                },
                total_quantity.map(bunting_market_types::QuantityLots::new),
            )),
            EventPayload::OrderCanceled {
                order_id: cancelled,
                ..
            } if *cancelled == order_id => Some((
                VenueReportKind::Cancelled,
                Some(bunting_market_types::QuantityLots::new(0)),
            )),
            EventPayload::TradeExecuted {
                maker_order_id,
                taker_order_id,
                price,
                quantity,
                ..
            } if *maker_order_id == order_id || *taker_order_id == order_id => {
                cumulative_quantity = cumulative_quantity.saturating_add(quantity.get());
                let leaves = total_quantity.map(|total| {
                    bunting_market_types::QuantityLots::new(
                        total.saturating_sub(cumulative_quantity).max(0),
                    )
                });
                Some((
                    VenueReportKind::Fill {
                        last_quantity: *quantity,
                        cumulative_quantity: bunting_market_types::QuantityLots::new(
                            cumulative_quantity,
                        ),
                        price: *price,
                    },
                    leaves,
                ))
            }
            _ => None,
        };
        if let Some((kind, leaves_quantity)) = mapped {
            reports.push(NormalizedVenueReport {
                report_id: ReportId::new(event.event_id.get()),
                source_sequence: Some(event.sequence.get()),
                client_order_id: Some(client),
                local_order_id: Some(local),
                venue_order_id: Some(VenueOrderId::new(order_id.to_string())),
                leaves_quantity,
                kind,
            });
        }
    }
    if reports.is_empty() {
        reports.push(NormalizedVenueReport {
            report_id: ReportId::new(command.command_id.get()),
            source_sequence: Some(executed.result.committed_sequence.get()),
            client_order_id: Some(client),
            local_order_id: Some(local),
            venue_order_id: Some(VenueOrderId::new(order_id.to_string())),
            leaves_quantity: None,
            kind: if executed.result.accepted {
                VenueReportKind::Accepted
            } else {
                VenueReportKind::Rejected {
                    reason: executed
                        .result
                        .reject_code
                        .clone()
                        .unwrap_or_else(|| "command rejected".to_owned()),
                }
            },
        });
    }
    reports
}

async fn write_actions(socket: &mut Socket, actions: &[SessionAction]) -> Result<()> {
    for action in actions {
        if let SessionAction::Send(frame) = action {
            socket
                .write_all(frame)
                .await
                .map_err(|error| Error::RustError(error.to_string()))?;
        }
    }
    socket
        .flush()
        .await
        .map_err(|error| Error::RustError(error.to_string()))
}

fn session_error(error: &simfix_session::SessionError) -> Error {
    Error::RustError(format!("FIX session error: {error:?}"))
}
