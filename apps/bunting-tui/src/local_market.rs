use crate::protocol::{now_millis, session_config, session_error, timestamp};
use bunting_engine::{ListingDefinition, ParticipantDefinition, RunState, ScenarioDefinition};
use bunting_market_events::{CancelOrder, Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, InstrumentId, IterationId, ListingKey, LogicalTimeNs, MoneyMinor,
    OrderId, ParticipantId, PriceBounds, PriceTicks, QuantityLots, RunId, ScenarioId,
    ScenarioVersion, VenueId,
};
use bunting_risk_engine::RiskLimits;
use quarcc_execution_engine::{ExecutionIntent, ids::IntentId};
use simfix_mapping::{
    InboundApplication, MappingContext, business_reject, map_inbound, market_snapshot,
};
use simfix_session::{FixSession, SessionAction};
use simfix_wire::FixMessage;
use std::{collections::BTreeMap, io};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
};

const RUN_ID: RunId = RunId::new(1);
const INSTRUMENT_ID: InstrumentId = InstrumentId::new(1);
const HUMAN_ID: ParticipantId = ParticipantId::new(1);
const MAKER_ID: ParticipantId = ParticipantId::new(2);

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
    orders: BTreeMap<u128, OrderView>,
}

pub async fn spawn(address: &str) -> io::Result<JoinHandle<()>> {
    let listener = TcpListener::bind(address).await?;
    Ok(tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let _ = Box::pin(serve(stream)).await;
        }
    }))
}

async fn serve(mut stream: TcpStream) -> io::Result<()> {
    let mut session = FixSession::try_new(session_config("BUNTING", "HUMAN"))
        .map_err(|error| session_error(&error))?;
    let actions = session
        .connected_at(&timestamp(), now_millis())
        .map_err(|error| session_error(&error))?;
    if write_actions(&mut stream, actions).await? {
        return Ok(());
    }
    let mut market = Market::new()?;
    let mut bytes = [0_u8; 16_384];
    loop {
        let read = stream.read(&mut bytes).await?;
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
                other => {
                    if write_actions(&mut stream, vec![other]).await? {
                        return Ok(());
                    }
                }
            }
        }
        for message in applications {
            for response in market.handle(&message) {
                let actions = session
                    .send_application(response, &timestamp())
                    .map_err(|error| session_error(&error))?;
                if write_actions(&mut stream, actions).await? {
                    return Ok(());
                }
            }
        }
    }
}

async fn write_actions(stream: &mut TcpStream, actions: Vec<SessionAction>) -> io::Result<bool> {
    let mut disconnect = false;
    for action in actions {
        match action {
            SessionAction::Send(frame) => stream.write_all(&frame).await?,
            SessionAction::Disconnect => disconnect = true,
            SessionAction::Application(_) | SessionAction::Persist(_) => {}
        }
    }
    Ok(disconnect)
}

impl Market {
    fn new() -> io::Result<Self> {
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
        let scenario = ScenarioDefinition::new(
            ScenarioId::new(1),
            ScenarioVersion::new(1),
            [listing],
            [participant(HUMAN_ID), participant(MAKER_ID)],
        )
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let state = RunState::from_scenario(RUN_ID, IterationId::new(1), &scenario)
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        let mut market = Self {
            state,
            next_command_id: 1,
            next_report_id: 1,
            orders: BTreeMap::new(),
        };
        market.seed(9_001, Side::Buy, 99, 50)?;
        market.seed(9_002, Side::Sell, 101, 50)?;
        Ok(market)
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
        let outcome = self.transition(CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(id),
            instrument_id: order.instrument,
            participant_id: order.participant,
            side: order.side,
            quantity: order.quantity,
            kind: order.kind,
        }))?;
        if outcome {
            self.orders.insert(id, order);
        }
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
                match self.transition(CommandPayload::SubmitOrder(SubmitOrder {
                    order_id: OrderId::new(id),
                    instrument_id: view.instrument,
                    participant_id: view.participant,
                    side: view.side,
                    quantity: view.quantity,
                    kind: view.kind,
                })) {
                    Ok(true) => {
                        self.orders.insert(id, view);
                        vec![self.report(id, id, "0", "0", None)]
                    }
                    Ok(false) => vec![self.report(id, id, "8", "8", Some("order rejected"))],
                    Err(error) => vec![business_reject("D", &error.to_string())],
                }
            }
            ExecutionIntent::Cancel {
                client_order_id, ..
            } => {
                let id = client_order_id.get();
                match self.transition(CommandPayload::CancelOrder(CancelOrder {
                    order_id: OrderId::new(id),
                    participant_id: HUMAN_ID,
                })) {
                    Ok(true) => {
                        if let Some(order) = self.orders.get_mut(&id) {
                            order.active = false;
                        }
                        vec![self.report(id, id, "4", "4", None)]
                    }
                    Ok(false) => vec![Self::cancel_reject(id, "cancel rejected")],
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
                let status = self
                    .orders
                    .get(&id)
                    .map_or("8", |order| if order.active { "0" } else { "4" });
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
        if !matches!(
            self.transition(CommandPayload::CancelOrder(CancelOrder {
                order_id: OrderId::new(old_id),
                participant_id: HUMAN_ID,
            })),
            Ok(true)
        ) {
            return vec![Self::cancel_reject(old_id, "replace cancel leg rejected")];
        }
        if let Some(order) = self.orders.get_mut(&old_id) {
            order.active = false;
        }
        previous.quantity = quantity;
        previous.kind = kind;
        let accepted = self.transition(CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(new_id),
            instrument_id: previous.instrument,
            participant_id: HUMAN_ID,
            side: previous.side,
            quantity,
            kind,
        }));
        if matches!(accepted, Ok(true)) {
            self.orders.insert(new_id, previous);
            vec![self.report(new_id, new_id, "5", "5", None)]
        } else {
            vec![self.report(
                new_id,
                new_id,
                "8",
                "8",
                Some("replacement submit leg rejected after cancel"),
            )]
        }
    }

    fn transition(&mut self, payload: CommandPayload) -> io::Result<bool> {
        let id = self.next_command_id;
        self.next_command_id = self.next_command_id.saturating_add(1);
        let command = Command {
            run_id: RUN_ID,
            command_id: CommandId::new(id),
            correlation_id: CorrelationId::new(id),
            logical_time: LogicalTimeNs::new(u64::try_from(id).unwrap_or(u64::MAX)),
            expected_sequence: self.state.sequence(),
            actor: match &payload {
                CommandPayload::SubmitOrder(order) => order.participant_id,
                CommandPayload::CancelOrder(_)
                | CommandPayload::ActivateKillSwitch
                | CommandPayload::NbcDone(_) => HUMAN_ID,
            },
            payload,
        };
        let outcome = self
            .state
            .transition(&command, None)
            .map_err(|error| io::Error::other(format!("engine transition: {error:?}")))?;
        let accepted = outcome.accepted;
        self.state = outcome.candidate;
        Ok(accepted)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{FixClient, book_request, new_order};
    use std::time::Duration;

    #[test]
    fn seeded_market_exposes_bid_and_ask_depth() -> io::Result<()> {
        let market = Market::new()?;
        let (bids, asks) = market
            .state
            .visible_levels(ListingKey::new(VenueId::new(1), INSTRUMENT_ID))
            .map_err(|error| io::Error::other(format!("{error:?}")))?;
        assert_eq!(bids, vec![(99, 50)]);
        assert_eq!(asks, vec![(101, 50)]);
        Ok(())
    }

    #[tokio::test]
    async fn fix_client_trades_and_refreshes_the_engine_book() -> io::Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await?;
            Box::pin(serve(stream)).await
        });
        let mut client = FixClient::connect(&address.to_string()).await?;
        for _ in 0..20 {
            Box::pin(client.poll()).await?;
            if client.connection_state() == simfix_session::ConnectionState::Established {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        client.send(book_request(1)).await?;
        for _ in 0..20 {
            Box::pin(client.poll()).await?;
            if !client.book.bids.is_empty() && !client.book.asks.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(client.book.bids, vec![(99, 50)]);
        assert_eq!(client.book.asks, vec![(101, 50)]);
        client.send(new_order(2, "buy", 5, Some(100))).await?;
        for _ in 0..20 {
            Box::pin(client.poll()).await?;
            if client.book.bids.first() == Some(&(100, 5)) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert_eq!(client.book.bids.first(), Some(&(100, 5)));
        server.abort();
        Ok(())
    }
}
