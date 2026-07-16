use crate::protocol::{now_millis, session_config, session_error, timestamp};
use bunting_agents::PolicyKind;
use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_application::{
    VerifiedActor,
    competition::{account, discovery, news_tenders, risk_score},
};
use bunting_engine::{
    ListingDefinition, OwnedOrderState, ParticipantDefinition, RunState, ScenarioDefinition,
};
use bunting_market_events::{
    CancelOrder, Command, CommandPayload, EventEnvelope, OrderKind, Side, SubmitOrder,
};
use bunting_market_types::{
    CommandId, CorrelationId, InstrumentId, IterationId, ListingKey, LogicalTimeNs, MoneyMinor,
    OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId,
    ScenarioVersion, VenueId,
};
use bunting_risk_engine::RiskLimits;
use bunting_runtime::{
    DeterministicRuntime, RuntimeAgentConfig, RuntimeConfig, RuntimeError, RuntimeHost,
};
use quarcc_bunting_adapter::BuntingExecutionAdapter;
use quarcc_execution_engine::{
    ExecutionIntent,
    ids::{ClientOrderId, IntentId},
};
use simfix_mapping::{
    CompetitionRequest, InboundApplication, MappingContext, business_reject, competition_report,
    map_execution_report, map_inbound, market_snapshot,
};
use simfix_session::{ConnectionState, FixSession, SessionAction};
use simfix_wire::FixMessage;
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

const RUN_ID: RunId = RunId::new(1);
const INSTRUMENT_ID: InstrumentId = InstrumentId::new(1);
const HUMAN_ID: ParticipantId = ParticipantId::new(1);
const MAKER_ID: ParticipantId = ParticipantId::new(2);
const FIRST_AGENT_ID: u128 = 10;
const AGENT_WAKE_INTERVAL_NS: u64 = 1_000_000_000;
const MAX_AGENT_ACTIONS_PER_TICK: usize = 256;
const MAX_PENDING_HUMAN_REPORTS: usize = 256;
const MANUAL_REPORT_ID_START: u128 = 1_000_000_000;

#[derive(Clone, Debug)]
pub struct LocalScenarioConfig {
    policies: Vec<PolicyKind>,
    wall_tick: Duration,
}

impl LocalScenarioConfig {
    pub fn from_names(names: &[String], wall_tick_ms: u64) -> io::Result<Self> {
        if names.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the local scenario requires at least one --agent policy",
            ));
        }
        if wall_tick_ms == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "--agent-tick-ms must be greater than zero",
            ));
        }
        let policies = names
            .iter()
            .map(|name| {
                serde_json::from_str::<PolicyKind>(&format!("{name:?}")).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown built-in policy: {name}"),
                    )
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self {
            policies,
            wall_tick: Duration::from_millis(wall_tick_ms),
        })
    }
}

#[derive(Clone, Copy)]
struct OrderView {
    participant: ParticipantId,
    instrument: InstrumentId,
    side: Side,
    quantity: QuantityLots,
    kind: OrderKind,
    active: bool,
}

struct Market {
    state: RunState,
    next_command_id: u128,
    next_report_id: u128,
    logical_time: LogicalTimeNs,
    orders: BTreeMap<u128, OrderView>,
    human_adapter: BuntingExecutionAdapter,
    pending_human_reports: VecDeque<FixMessage>,
    runner: Option<DeterministicRuntime>,
}

pub async fn spawn(address: &str, config: LocalScenarioConfig) -> io::Result<JoinHandle<()>> {
    let listener = TcpListener::bind(address).await?;
    Ok(tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            if let Err(error) = Box::pin(serve(stream, config)).await {
                eprintln!("bunting-tui: embedded market connection closed: {error}");
            }
        }
    }))
}

async fn serve(mut stream: TcpStream, config: LocalScenarioConfig) -> io::Result<()> {
    let mut session_config = session_config("BUNTING", "HUMAN");
    session_config.logon_fields = vec![
        simfix_wire::Field::new(10000, crate::config::FIX_PROFILE_VERSION),
        simfix_wire::Field::new(10004, "participant"),
    ];
    let mut session = FixSession::try_new(session_config).map_err(|error| session_error(&error))?;
    let actions = session
        .connected_at(&timestamp(), now_millis())
        .map_err(|error| session_error(&error))?;
    if write_actions(&mut stream, actions).await? {
        return Ok(());
    }
    let mut timer = interval(config.wall_tick);
    timer.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut market = Market::new(&config.policies)?;
    let mut bytes = [0_u8; 16_384];
    loop {
        tokio::select! {
            read = stream.read(&mut bytes) => {
                let read = read?;
                if read == 0 {
                    return Ok(());
                }
                let actions = session
                    .receive_bytes_at(&bytes[..read], &timestamp(), now_millis())
                    .map_err(|error| session_error(&error))?;
                let mut applications = Vec::new();
                for action in actions {
                    match action {
                        SessionAction::Application(message) => applications.push(message),
                        SessionAction::PeerLogon(_) => {}
                        other => {
                            if write_actions(&mut stream, vec![other]).await? {
                                return Ok(());
                            }
                        }
                    }
                }
                for message in applications {
                    for response in market.handle(&message) {
                        if write_application(&mut stream, &mut session, response).await? {
                            return Ok(());
                        }
                    }
                }
            }
            _ = timer.tick() => {
                if session.snapshot().state != ConnectionState::Established {
                    continue;
                }
                let changed = market.advance_agents()?;
                for response in market.drain_human_reports() {
                    if write_application(&mut stream, &mut session, response).await? {
                        return Ok(());
                    }
                }
                if changed && write_application(
                    &mut stream,
                    &mut session,
                    market.book("book-agents"),
                ).await? {
                    return Ok(());
                }
            }
        }
    }
}

async fn write_application(
    stream: &mut TcpStream,
    session: &mut FixSession,
    message: FixMessage,
) -> io::Result<bool> {
    let actions = session
        .send_application(message, &timestamp())
        .map_err(|error| session_error(&error))?;
    write_actions(stream, actions).await
}

async fn write_actions(stream: &mut TcpStream, actions: Vec<SessionAction>) -> io::Result<bool> {
    let mut disconnect = false;
    for action in actions {
        match action {
            SessionAction::Send(frame) => stream.write_all(&frame).await?,
            SessionAction::Disconnect => disconnect = true,
            SessionAction::Application(_)
            | SessionAction::PeerLogon(_)
            | SessionAction::Persist(_) => {}
        }
    }
    Ok(disconnect)
}

impl Market {
    fn new(policies: &[PolicyKind]) -> io::Result<Self> {
        let bounds = PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000))
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let listing = ListingDefinition::new(
            ListingKey::new(VenueId::new(1), INSTRUMENT_ID),
            "BUNT".to_owned(),
            bounds,
        )
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let participant = |id| {
            ParticipantDefinition::new(
                id,
                true,
                RiskLimits {
                    max_order_quantity: QuantityLots::new(10_000),
                    max_open_order_quantity: QuantityLots::new(100_000),
                    max_absolute_position: QuantityLots::new(1_000_000),
                },
                MoneyMinor::new(10_000_000),
                BTreeMap::from([(INSTRUMENT_ID, QuantityLots::new(100_000))]),
            )
        };
        let agent_participants = policies
            .iter()
            .enumerate()
            .map(|(index, _)| {
                let offset =
                    u128::try_from(index).map_err(|_| io::Error::other("too many local agents"))?;
                Ok(participant(ParticipantId::new(FIRST_AGENT_ID + offset)))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [listing],
            [participant(HUMAN_ID), participant(MAKER_ID)]
                .into_iter()
                .chain(agent_participants),
        )
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let state = RunState::from_scenario(RUN_ID, IterationId::new(1), &scenario)
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let mut market = Self {
            state,
            next_command_id: 1,
            next_report_id: MANUAL_REPORT_ID_START,
            logical_time: LogicalTimeNs::new(0),
            orders: BTreeMap::new(),
            human_adapter: BuntingExecutionAdapter::default(),
            pending_human_reports: VecDeque::new(),
            runner: None,
        };
        market.seed(9_001, Side::Buy, 99, 50)?;
        market.seed(9_002, Side::Sell, 101, 50)?;
        let agents = policies
            .iter()
            .copied()
            .enumerate()
            .map(|(index, kind)| {
                let offset =
                    u128::try_from(index).map_err(|_| io::Error::other("too many local agents"))?;
                Ok(RuntimeAgentConfig {
                    kind,
                    participant_id: ParticipantId::new(FIRST_AGENT_ID + offset),
                    base_quantity: QuantityLots::new(5),
                    spread_ticks: 2,
                    inventory_target: QuantityLots::new(0),
                    wake_interval_ns: AGENT_WAKE_INTERVAL_NS,
                    seed: 42_u64.saturating_add(u64::try_from(index).unwrap_or(u64::MAX)),
                    max_intents_per_wake: 4,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        market.runner = Some(
            DeterministicRuntime::new(RuntimeConfig {
                run_id: RUN_ID,
                instrument_id: INSTRUMENT_ID,
                fundamental_price: PriceTicks::new(100),
                remaining_parent_quantity: QuantityLots::new(1_000),
                max_actions_per_tick: MAX_AGENT_ACTIONS_PER_TICK,
                agents,
            })
            .map_err(runtime_error)?,
        );
        Ok(market)
    }

    fn advance_agents(&mut self) -> io::Result<bool> {
        let Some(mut runner) = self.runner.take() else {
            return Ok(false);
        };
        let result = runner
            .advance(self)
            .map(|processed| processed > 0)
            .map_err(runtime_error);
        self.runner = Some(runner);
        result
    }

    fn seed(&mut self, id: u128, side: Side, price: i64, quantity: i64) -> io::Result<()> {
        let order = OrderView {
            participant: MAKER_ID,
            instrument: INSTRUMENT_ID,
            side,
            quantity: QuantityLots::new(quantity),
            kind: OrderKind::Limit {
                price: PriceTicks::new(price),
            },
            active: true,
        };
        let outcome = self.transition(
            CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(id),
                instrument_id: order.instrument,
                participant_id: order.participant,
                side: order.side,
                quantity: order.quantity,
                kind: order.kind,
            }),
            MAKER_ID,
        )?;
        if outcome.accepted {
            self.orders.insert(id, order);
        }
        self.pending_human_reports.clear();
        Ok(())
    }

    fn handle(&mut self, message: &FixMessage) -> Vec<FixMessage> {
        let context = MappingContext {
            participant_id: HUMAN_ID,
            next_intent_id: IntentId::new(self.next_command_id),
        };
        let application = match map_inbound(message, context) {
            Ok(value) => value,
            Err(error) => {
                return vec![business_reject(
                    &message.msg_type,
                    &format!("invalid FIX application message: {error:?}"),
                )];
            }
        };
        let mut responses = match application {
            InboundApplication::Competition(request) => self.competition(&request),
            InboundApplication::MarketDataRequest { request_id, .. } => {
                vec![self.book(&request_id)]
            }
            InboundApplication::Intent(intent) => self.execute(intent, message),
        };
        if message.msg_type != "V" {
            responses.push(self.book("book-live"));
        }
        responses
    }

    #[expect(
        clippy::too_many_lines,
        reason = "the deterministic fixture keeps every competition projection in one exhaustive match"
    )]
    fn competition(&self, request: &CompetitionRequest) -> Vec<FixMessage> {
        let actor = match VerifiedActor::try_from_identity(ActorIdentity {
            actor_id: UnsignedDecimalString::new(HUMAN_ID.get()),
            role: ActorRole::Participant,
            participant_id: Some(UnsignedDecimalString::new(HUMAN_ID.get())),
            team_id: None,
        }) {
            Ok(actor) => actor,
            Err(error) => {
                return vec![business_reject(
                    "competition",
                    &format!("fixture actor is invalid: {error:?}"),
                )];
            }
        };
        let result = match request {
            CompetitionRequest::Discovery => competition_report(
                "y",
                "public",
                "discovery",
                "snapshot",
                "ok",
                self.state.sequence().get(),
                &discovery(&self.state),
            ),
            CompetitionRequest::Account => account(&self.state, &actor).map_or_else(
                |_| Err(simfix_mapping::MappingError::Serialization),
                |view| {
                    competition_report(
                        "AP",
                        "private",
                        "account",
                        "snapshot",
                        "ok",
                        self.state.sequence().get(),
                        &view,
                    )
                },
            ),
            CompetitionRequest::News => news_tenders(&self.state, &actor).map_or_else(
                |_| Err(simfix_mapping::MappingError::Serialization),
                |view| {
                    competition_report(
                        "B",
                        "private",
                        "news",
                        "list",
                        "ok",
                        self.state.sequence().get(),
                        &view.news,
                    )
                },
            ),
            CompetitionRequest::Tender { .. } => news_tenders(&self.state, &actor).map_or_else(
                |_| Err(simfix_mapping::MappingError::Serialization),
                |view| {
                    competition_report(
                        "U6",
                        "private",
                        "tender",
                        "list",
                        "ok",
                        self.state.sequence().get(),
                        &view.tenders,
                    )
                },
            ),
            CompetitionRequest::Score => risk_score(&self.state, &actor).map_or_else(
                |_| Err(simfix_mapping::MappingError::Serialization),
                |view| {
                    competition_report(
                        "U9",
                        "private",
                        "score",
                        "snapshot",
                        "ok",
                        self.state.sequence().get(),
                        &view.latest_score,
                    )
                },
            ),
            CompetitionRequest::Risk => risk_score(&self.state, &actor).map_or_else(
                |_| Err(simfix_mapping::MappingError::Serialization),
                |view| {
                    competition_report(
                        "UB",
                        "private",
                        "risk",
                        "snapshot",
                        "ok",
                        self.state.sequence().get(),
                        &view,
                    )
                },
            ),
            CompetitionRequest::RunControl { .. } | CompetitionRequest::RiskAdmin { .. } => {
                return vec![business_reject("UA", "fixture operator role required")];
            }
        };
        match result {
            Ok(report) => vec![report],
            Err(error) => vec![business_reject(
                "U1",
                &format!("fixture competition projection failed: {error:?}"),
            )],
        }
    }

    fn execute(&mut self, intent: ExecutionIntent, original: &FixMessage) -> Vec<FixMessage> {
        match intent {
            ExecutionIntent::Submit { order, .. } => {
                let id = order.client_order_id.get();
                let view = OrderView {
                    participant: HUMAN_ID,
                    instrument: order.instrument_id,
                    side: order.side,
                    quantity: order.quantity,
                    kind: order.kind,
                    active: true,
                };
                match self.transition(
                    CommandPayload::SubmitOrder(SubmitOrder {
                        order_id: OrderId::new(id),
                        instrument_id: view.instrument,
                        participant_id: view.participant,
                        side: view.side,
                        quantity: view.quantity,
                        kind: view.kind,
                    }),
                    HUMAN_ID,
                ) {
                    Ok(outcome) if outcome.accepted => {
                        let active = self.order_is_active(id);
                        let mut view = view;
                        view.active = active;
                        self.orders.insert(id, view);
                        self.drain_human_reports()
                    }
                    Ok(_) => self.drain_human_reports(),
                    Err(error) => vec![business_reject("D", &error.to_string())],
                }
            }
            ExecutionIntent::Cancel {
                client_order_id, ..
            } => {
                let id = client_order_id.get();
                match self.transition(
                    CommandPayload::CancelOrder(CancelOrder {
                        order_id: OrderId::new(id),
                        participant_id: HUMAN_ID,
                    }),
                    HUMAN_ID,
                ) {
                    Ok(outcome) if outcome.accepted => {
                        if let Some(order) = self.orders.get_mut(&id) {
                            order.active = false;
                        }
                        self.drain_human_reports()
                    }
                    Ok(_) => {
                        self.pending_human_reports.clear();
                        vec![Self::cancel_reject(id, "cancel rejected")]
                    }
                    Err(error) => vec![Self::cancel_reject(id, &error.to_string())],
                }
            }
            ExecutionIntent::Replace {
                client_order_id,
                quantity,
                kind,
                ..
            } => self.replace(client_order_id.get(), quantity, kind, original),
            ExecutionIntent::Query { local_order_id, .. } => {
                let id = local_order_id.get();
                let status = self.order_status(id);
                vec![self.report(id, id, "I", status, None)]
            }
            ExecutionIntent::ActivateKillSwitch { .. } => {
                vec![business_reject("q", "kill switch has no FIX human message")]
            }
        }
    }

    fn replace(
        &mut self,
        old_id: u128,
        quantity: QuantityLots,
        kind: OrderKind,
        original: &FixMessage,
    ) -> Vec<FixMessage> {
        let Some(mut previous) = self.orders.get(&old_id).copied() else {
            return vec![Self::cancel_reject(old_id, "unknown original order")];
        };
        let Some(new_id) = original.value(11).and_then(|value| value.parse().ok()) else {
            return vec![business_reject("G", "missing replacement ClOrdID")];
        };
        if !previous.active {
            return vec![Self::cancel_reject(old_id, "original order is not active")];
        }
        let cancel = self.transition(
            CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(old_id),
                participant_id: HUMAN_ID,
            }),
            HUMAN_ID,
        );
        if !matches!(cancel, Ok(outcome) if outcome.accepted) {
            self.pending_human_reports.clear();
            return vec![Self::cancel_reject(old_id, "replace cancel leg rejected")];
        }
        if let Some(order) = self.orders.get_mut(&old_id) {
            order.active = false;
        }
        previous.quantity = quantity;
        previous.kind = kind;
        let submitted = self.transition(
            CommandPayload::SubmitOrder(SubmitOrder {
                order_id: OrderId::new(new_id),
                instrument_id: previous.instrument,
                participant_id: HUMAN_ID,
                side: previous.side,
                quantity,
                kind,
            }),
            HUMAN_ID,
        );
        if matches!(submitted, Ok(ref outcome) if outcome.accepted) {
            previous.active = self.order_is_active(new_id);
            self.orders.insert(new_id, previous);
            self.drain_human_reports()
        } else {
            let reports = self.drain_human_reports();
            if reports.is_empty() {
                vec![business_reject(
                    "G",
                    "replacement submit leg rejected after cancel",
                )]
            } else {
                reports
            }
        }
    }

    fn transition(
        &mut self,
        payload: CommandPayload,
        actor: ParticipantId,
    ) -> io::Result<bunting_engine::TransitionOutcome> {
        let id = self.next_command_id;
        self.next_command_id = self.next_command_id.saturating_add(1);
        let logical_time = self.next_logical_time(self.logical_time);
        let command = Command {
            run_id: RUN_ID,
            command_id: CommandId::new(id),
            correlation_id: CorrelationId::new(id),
            logical_time,
            expected_sequence: self.state.sequence(),
            actor,
            payload,
        };
        self.apply_command(&command)
    }

    fn apply_command(
        &mut self,
        command: &Command,
    ) -> io::Result<bunting_engine::TransitionOutcome> {
        let outcome = self
            .state
            .transition(command, None)
            .map_err(|error| io::Error::other(format!("engine transition: {error:?}")))?;
        let mut human_adapter = self.human_adapter.clone();
        let mut reports = human_adapter
            .normalize_committed_events(HUMAN_ID, &outcome.events)
            .map_err(|error| io::Error::other(format!("human venue reports: {error:?}")))?;
        let messages = reports
            .iter_mut()
            .map(|report| {
                if let Some(local) = report.local_order_id {
                    report.client_order_id = Some(ClientOrderId::new(local.get()));
                }
                map_execution_report(report)
                    .map_err(|error| io::Error::other(format!("FIX execution report: {error:?}")))
            })
            .collect::<io::Result<Vec<_>>>()?;
        if self
            .pending_human_reports
            .len()
            .saturating_add(messages.len())
            > MAX_PENDING_HUMAN_REPORTS
        {
            return Err(io::Error::other("human execution-report queue is full"));
        }
        self.logical_time = command.logical_time;
        self.state = outcome.candidate.clone();
        self.human_adapter = human_adapter;
        self.pending_human_reports.extend(messages);
        for (order_id, order) in &mut self.orders {
            order.active = self
                .state
                .ownership()
                .get(&OrderId::new(*order_id))
                .is_some_and(|owned| owned.state == OwnedOrderState::Active);
        }
        Ok(outcome)
    }

    fn drain_human_reports(&mut self) -> Vec<FixMessage> {
        self.pending_human_reports.drain(..).collect()
    }

    fn order_is_active(&self, order_id: u128) -> bool {
        self.state
            .ownership()
            .get(&OrderId::new(order_id))
            .is_some_and(|owned| owned.state == OwnedOrderState::Active)
    }

    fn order_status(&self, order_id: u128) -> &'static str {
        self.state
            .ownership()
            .get(&OrderId::new(order_id))
            .map_or("8", |owned| match owned.state {
                OwnedOrderState::Active => "0",
                OwnedOrderState::Filled => "2",
                OwnedOrderState::Canceled => "4",
            })
    }

    fn next_logical_time(&self, requested: LogicalTimeNs) -> LogicalTimeNs {
        LogicalTimeNs::new(
            requested
                .get()
                .max(self.logical_time.get().saturating_add(1)),
        )
    }

    fn report(
        &mut self,
        order_id: u128,
        client_id: u128,
        exec_type: &str,
        order_status: &str,
        reason: Option<&str>,
    ) -> FixMessage {
        let mut message = FixMessage::new("8");
        message.push(37, order_id.to_string());
        message.push(11, client_id.to_string());
        message.push(17, self.next_report_id.to_string());
        self.next_report_id = self.next_report_id.saturating_add(1);
        message.push(150, exec_type);
        message.push(39, order_status);
        if let Some(reason) = reason {
            message.push(58, reason);
        }
        message
    }

    fn cancel_reject(order_id: u128, reason: &str) -> FixMessage {
        let mut message = FixMessage::new("9");
        message.push(37, order_id.to_string());
        message.push(39, "0");
        message.push(58, reason);
        message
    }

    fn book(&self, request_id: &str) -> FixMessage {
        let key = ListingKey::new(VenueId::new(1), INSTRUMENT_ID);
        let (bids, asks) = self.state.visible_levels(key).unwrap_or_default();
        let convert = |levels: Vec<(u128, u64)>| {
            levels
                .into_iter()
                .filter_map(|(price, quantity)| {
                    Some((
                        PriceTicks::new(i64::try_from(price).ok()?),
                        QuantityLots::new(i64::try_from(quantity).ok()?),
                    ))
                })
                .collect::<Vec<_>>()
        };
        market_snapshot(request_id, INSTRUMENT_ID, &convert(bids), &convert(asks))
    }
}

impl RuntimeHost for Market {
    fn state(&self, run_id: RunId) -> Result<RunState, RuntimeError> {
        if run_id != self.state.run_id() {
            return Err(RuntimeError::Host(
                "runtime requested an unknown run".to_owned(),
            ));
        }
        Ok(self.state.clone())
    }

    fn commit(
        &mut self,
        actor: &VerifiedActor,
        command: &Command,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        if actor.participant_id() != Some(command.actor) {
            return Err(RuntimeError::Host("runtime actor mismatch".to_owned()));
        }
        self.apply_command(command)
            .map(|outcome| outcome.events)
            .map_err(|error| RuntimeError::Host(error.to_string()))
    }
}

fn runtime_error(error: RuntimeError) -> io::Error {
    io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TerminalConfig;
    use crate::protocol::{FixClient, book_request, new_order};
    use std::time::Duration;

    #[test]
    fn seeded_market_exposes_bid_and_ask_depth() -> io::Result<()> {
        let market = Market::new(&[])?;
        let (bids, asks) = market
            .state
            .visible_levels(ListingKey::new(VenueId::new(1), INSTRUMENT_ID))
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        assert_eq!(bids, vec![(99, 50)]);
        assert_eq!(asks, vec![(101, 50)]);
        Ok(())
    }

    #[test]
    fn agent_wake_routes_quarcc_actions_into_the_engine_book() -> io::Result<()> {
        let mut market = Market::new(&[PolicyKind::StaticLiquidityProvider])?;
        assert!(market.advance_agents()?);
        let (bids, asks) = market
            .state
            .visible_levels(ListingKey::new(VenueId::new(1), INSTRUMENT_ID))
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        assert_eq!(bids, vec![(99, 50), (98, 5)]);
        assert_eq!(asks, vec![(101, 50), (102, 5)]);
        let runner = market
            .runner
            .as_ref()
            .ok_or_else(|| io::Error::other("missing scenario runner"))?;
        assert_eq!(
            runner.snapshot().agents[0].managed.execution.orders.len(),
            2
        );
        Ok(())
    }

    #[test]
    fn crossing_limit_order_returns_acceptance_and_fill_reports() -> io::Result<()> {
        let mut market = Market::new(&[])?;
        let messages = market.handle(&new_order(3, "buy", 5, Some(101)));
        let reports = messages
            .iter()
            .filter(|message| message.msg_type == "8")
            .collect::<Vec<_>>();

        assert_eq!(reports.len(), 2);
        assert_eq!(field(reports[0], 150), Some("0"));
        assert_eq!(field(reports[1], 150), Some("F"));
        assert_eq!(field(reports[1], 39), Some("2"));
        assert_eq!(field(reports[1], 32), Some("5"));
        assert_eq!(field(reports[1], 31), Some("101"));
        assert_eq!(field(reports[1], 151), Some("0"));
        assert_eq!(market.order_status(3), "2");
        assert_eq!(
            market
                .state
                .visible_levels(ListingKey::new(VenueId::new(1), INSTRUMENT_ID))
                .map_err(|error| io::Error::other(format!("{error:?}")))?
                .1
                .first(),
            Some(&(101, 45))
        );
        Ok(())
    }

    #[test]
    fn market_order_executes_against_seeded_orderbook_rs_liquidity() -> io::Result<()> {
        let mut market = Market::new(&[])?;
        let messages = market.handle(&new_order(4, "buy", 5, None));
        let fill = messages
            .iter()
            .find(|message| field(message, 150) == Some("F"))
            .ok_or_else(|| io::Error::other("missing FIX fill report"))?;

        assert_eq!(field(fill, 39), Some("2"));
        assert_eq!(field(fill, 14), Some("5"));
        assert_eq!(field(fill, 151), Some("0"));
        assert_eq!(market.order_status(4), "2");
        assert_eq!(
            market
                .state
                .visible_levels(ListingKey::new(VenueId::new(1), INSTRUMENT_ID))
                .map_err(|error| io::Error::other(format!("{error:?}")))?
                .1
                .first(),
            Some(&(101, 45))
        );
        Ok(())
    }

    fn field(message: &FixMessage, tag: u32) -> Option<&str> {
        message
            .fields
            .iter()
            .find(|field| field.tag == tag)
            .map(|field| field.value.as_str())
    }

    #[tokio::test]
    async fn embedded_market_waits_for_slow_fix_logon() -> io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            Box::pin(serve(
                stream,
                LocalScenarioConfig::from_names(&["static_liquidity_provider".to_owned()], 10)?,
            ))
            .await
        });
        let mut stream = TcpStream::connect(address).await?;
        let mut bytes = [0_u8; 16_384];
        assert!(stream.read(&mut bytes).await? > 0);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(20), stream.read(&mut bytes))
                .await
                .is_err(),
            "embedded market closed before the client sent FIX Logon"
        );
        server.abort();
        Ok(())
    }

    #[tokio::test]
    async fn fix_client_trades_and_refreshes_the_engine_book() -> io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            Box::pin(serve(
                stream,
                LocalScenarioConfig::from_names(&["static_liquidity_provider".to_owned()], 10)?,
            ))
            .await
        });
        let mut profile = TerminalConfig::default()
            .profile("local")
            .map_err(io::Error::other)?;
        profile.endpoint = address.to_string();
        let mut client = FixClient::new(
            "local-fixture".to_owned(),
            profile,
            Some("fixture-only".to_owned()),
        )?;
        client.reconnect().await?;
        for _ in 0..20 {
            Box::pin(client.poll_once()).await?;
            if client.connection_state() == simfix_session::ConnectionState::Established {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        client.send(book_request(1)).await?;
        for _ in 0..20 {
            Box::pin(client.poll_once()).await?;
            if !client.book.bids.is_empty() && !client.book.asks.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(client.book.bids.first(), Some(&(99, 50)));
        assert_eq!(client.book.asks.first(), Some(&(101, 50)));
        client.send(new_order(2, "buy", 5, Some(100))).await?;
        for _ in 0..20 {
            Box::pin(client.poll_once()).await?;
            if client.book.bids.first() == Some(&(100, 5)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(client.book.bids.first(), Some(&(100, 5)));
        client.send(new_order(3, "buy", 5, None)).await?;
        for _ in 0..20 {
            Box::pin(client.poll_once()).await?;
            if client
                .executions
                .iter()
                .any(|report| report.order_id == "3" && report.kind == "F")
                && client.book.asks.first() == Some(&(101, 45))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(client.executions.iter().any(|report| {
            report.order_id == "3" && report.kind == "F" && report.order_status == "2"
        }));
        assert_eq!(client.book.asks.first(), Some(&(101, 45)));
        server.abort();
        Ok(())
    }
}
