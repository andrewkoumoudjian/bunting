#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native Rust Worker entrypoint for browser and outbound FIX clients.
//!
//! Browser requests use bounded `/api/<procedure>` dispatch.

mod d1_origin;
mod fix_session_object;
mod subscriptions;

use bunting_api_contract::{
    API_VERSION, AccountsSubscribeInput, ActorIdentity, ActorRole, BuntingErrorCode,
    CancelOrderInput, CommandOutput, HealthOutput, MarketSnapshotInput, MarketSnapshotOutput,
    MarketSubscribeInput, PriceLevel, SequenceDecimalString, Side as ContractSide,
    SignedDecimalString, SubmitOrderInput, UnsignedDecimalString,
};
use bunting_application::{
    VerifiedActor, derive_session_id, namespace_command_id, namespace_order_id,
    prepare_authenticated, prepare_authenticated_simulation, project_market,
};
use bunting_browser_wire::{
    Call, ErrorCode, Method, ParsedRequest, Request as WireRequest, Response as WireResponse,
};
use bunting_command_transaction::{
    CachedSnapshot, TransactionError, command_fingerprint, simulation_command_fingerprint,
};
use bunting_engine::{ORDERBOOK_RS_VERSION, RunState};
use bunting_market_events::{
    CancelOrder, Command, CommandPayload, EventEnvelope, OrderKind, Side, SimulationCommandRequest,
    SubmitOrder,
};
use bunting_market_types::{
    CorrelationId, EventSequence, InstrumentId, LogicalTimeNs, ParticipantId, PriceTicks,
    QuantityLots, RunId, SessionId,
};
use bunting_origin_store::{ClientCommandKey, CommandResult, CommitOutcome, OriginError};
use bunting_worker_cache::{CachePolicy, SnapshotCacheKey, cloudflare};
use serde::de::DeserializeOwned;
use worker::{Context, Env, Error, Request, Response, ResponseBuilder, Result, event};

const AUTH_TOKEN_BYTES: usize = 256;
const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy)]
struct VerifiedClaims {
    participant_id: ParticipantId,
    session_id: SessionId,
}

#[derive(Clone, Copy, Debug)]
enum ProcedureError {
    Unauthorized,
    BadRequest,
    NotFound,
    DuplicateCommandConflict,
    VersionConflict,
    OriginUnavailable,
    InternalContractMismatch,
}

impl ProcedureError {
    const fn code(self) -> ErrorCode {
        match self {
            Self::Unauthorized => ErrorCode::Unauthorized,
            Self::BadRequest => ErrorCode::BadRequest,
            Self::NotFound => ErrorCode::NotFound,
            Self::DuplicateCommandConflict | Self::VersionConflict => ErrorCode::Conflict,
            Self::OriginUnavailable | Self::InternalContractMismatch => {
                ErrorCode::InternalServerError
            }
        }
    }

    const fn bunting_code(self) -> BuntingErrorCode {
        match self {
            Self::Unauthorized => BuntingErrorCode::Unauthenticated,
            Self::BadRequest => BuntingErrorCode::InvalidInput,
            Self::NotFound => BuntingErrorCode::NotFound,
            Self::DuplicateCommandConflict => BuntingErrorCode::DuplicateCommandConflict,
            Self::VersionConflict => BuntingErrorCode::VersionConflict,
            Self::OriginUnavailable => BuntingErrorCode::OriginUnavailable,
            Self::InternalContractMismatch => BuntingErrorCode::InternalContractMismatch,
        }
    }
}

fn constant_time_token_eq(provided: &[u8], expected: &[u8]) -> bool {
    let mut difference = provided.len() ^ expected.len();
    for index in 0..AUTH_TOKEN_BYTES {
        let left = provided.get(index).copied().unwrap_or_default();
        let right = expected.get(index).copied().unwrap_or_default();
        difference |= usize::from(left ^ right);
    }
    difference == 0 && provided.len() <= AUTH_TOKEN_BYTES
}

fn authenticate(request: &Request, environment: &Env) -> Result<VerifiedClaims> {
    let authorization = request
        .headers()
        .get("authorization")?
        .ok_or_else(|| Error::RustError("missing authorization".to_string()))?;
    let provided = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| Error::RustError("invalid authorization scheme".to_string()))?;
    let expected = environment.secret("BUNTING_API_TOKEN")?.to_string();
    if !constant_time_token_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(Error::RustError("invalid bearer token".to_string()));
    }
    let participant_id = environment
        .secret("BUNTING_API_PARTICIPANT_ID")?
        .to_string()
        .parse::<u128>()
        .map_err(|_| Error::RustError("invalid configured participant claim".to_string()))?;
    Ok(VerifiedClaims {
        participant_id: ParticipantId::new(participant_id),
        session_id: derive_session_id(provided.as_bytes()),
    })
}

pub(crate) fn verified_participant(
    participant_id: ParticipantId,
) -> std::result::Result<VerifiedActor, ProcedureError> {
    VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(participant_id.get()),
        role: ActorRole::Participant,
        participant_id: Some(UnsignedDecimalString::new(participant_id.get())),
        team_id: None,
    })
    .map_err(|_| ProcedureError::InternalContractMismatch)
}

fn decode_input<T: DeserializeOwned>(call: &Call) -> std::result::Result<T, ProcedureError> {
    serde_json::from_value(call.input.clone().unwrap_or(serde_json::Value::Null))
        .map_err(|_| ProcedureError::BadRequest)
}

fn wire_error(error: ProcedureError, path: &str, message: &str) -> WireResponse {
    bunting_browser_wire::procedure_error(error.code(), error.bunting_code(), message, path)
}

fn command_output(result: &CommandResult) -> CommandOutput {
    CommandOutput {
        accepted: result.accepted,
        reject_code: result.reject_code.clone(),
        committed_sequence: SequenceDecimalString::new(result.committed_sequence.get()),
        order_id: result
            .order_id
            .map(|order_id| UnsignedDecimalString::new(order_id.get())),
        snapshot_checksum: result.snapshot_checksum.clone(),
    }
}

fn command_response(result: &CommandResult, path: &str) -> WireResponse {
    let output = command_output(result);
    if result.accepted {
        bunting_browser_wire::success(200, &output)
    } else {
        bunting_browser_wire::procedure_error_with_data(
            ErrorCode::UnprocessableContent,
            BuntingErrorCode::RiskRejected,
            "command rejected by market or participant controls",
            path,
            Some(&output),
        )
    }
}

const fn map_origin_error(error: &OriginError) -> ProcedureError {
    match error {
        OriginError::UnknownRun => ProcedureError::NotFound,
        OriginError::IdempotencyConflict => ProcedureError::DuplicateCommandConflict,
        OriginError::VersionConflict { .. } => ProcedureError::VersionConflict,
        OriginError::Unavailable => ProcedureError::OriginUnavailable,
        OriginError::InvalidCommit => ProcedureError::InternalContractMismatch,
    }
}

const fn map_transaction_error(error: TransactionError) -> ProcedureError {
    match error {
        TransactionError::Origin(origin) => map_origin_error(&origin),
        TransactionError::IdempotencyConflict => ProcedureError::DuplicateCommandConflict,
        TransactionError::Engine(_) | TransactionError::Serialization => {
            ProcedureError::InternalContractMismatch
        }
    }
}

fn worker_response(response: WireResponse) -> Result<Response> {
    Ok(ResponseBuilder::new()
        .with_status(response.status)
        .with_header("content-type", response.content_type)?
        .with_header("vary", response.vary)?
        .fixed(response.body))
}

fn subscription_response(frames: &[Vec<u8>]) -> Result<Response> {
    Ok(ResponseBuilder::new()
        .with_status(200)
        .with_header("content-type", "text/event-stream")?
        .with_header("cache-control", "no-cache, no-transform")?
        .with_header("vary", "authorization")?
        .with_header("x-accel-buffering", "no")?
        .fixed(frames.concat()))
}

fn resume_cursor(request: &Request, input_cursor: u64) -> Result<u64> {
    request
        .headers()
        .get("last-event-id")?
        .map_or(Ok(input_cursor), |value| {
            value
                .parse::<u64>()
                .map_err(|_| Error::RustError("invalid Last-Event-ID".to_owned()))
        })
}

async fn subscribe(call: &Call, request: &Request, environment: &Env) -> Result<Response> {
    let (run_id, instrument_id, after, class) = match call.path.as_str() {
        "market.subscribe" => {
            let input: MarketSubscribeInput = decode_input(call)
                .map_err(|_| Error::RustError("invalid market subscription input".to_owned()))?;
            let instrument_id = InstrumentId::new(input.instrument_id.get());
            (
                RunId::new(input.run_id.get()),
                Some(instrument_id),
                resume_cursor(request, input.after_sequence.get())?,
                subscriptions::StreamClass::Public { instrument_id },
            )
        }
        "accounts.subscribe" => {
            let claims = authenticate(request, environment)?;
            let input: AccountsSubscribeInput = decode_input(call)
                .map_err(|_| Error::RustError("invalid account subscription input".to_owned()))?;
            (
                RunId::new(input.run_id.get()),
                None,
                resume_cursor(request, input.after_sequence.get())?,
                subscriptions::StreamClass::Private {
                    participant_id: claims.participant_id,
                },
            )
        }
        _ => return Err(Error::RustError("unknown subscription".to_owned())),
    };
    let database = environment.d1("ORIGIN_DB")?;
    let state = d1_origin::load_run(&database, &run_id.to_string())
        .await
        .map_err(|_| Error::RustError("subscription origin unavailable".to_owned()))?;
    if let Some(instrument_id) = instrument_id {
        state
            .listing_key_for_instrument(instrument_id)
            .map_err(|_| Error::RustError("subscription instrument not found".to_owned()))?;
    }
    if after > state.event_sequence().get() {
        return Err(Error::RustError(
            "resume cursor exceeds committed event sequence".to_owned(),
        ));
    }
    let events = d1_origin::load_event_tail(
        &database,
        &run_id.to_string(),
        after,
        subscriptions::ORIGIN_READ_LIMIT,
    )
    .await
    .map_err(|_| Error::RustError("subscription tail unavailable".to_owned()))?;
    let cursor = state.event_sequence();
    let plan = subscriptions::plan(events, cursor, class);
    let snapshot = if matches!(&plan, subscriptions::Plan::Reset { .. }) {
        instrument_id
            .map(|id| snapshot_output(&state, id))
            .transpose()
            .map_err(|_| Error::RustError("snapshot unavailable".to_owned()))?
    } else {
        None
    };
    subscription_response(&subscriptions::encode(plan, snapshot.as_ref(), cursor))
}

fn health() -> HealthOutput {
    HealthOutput {
        api_version: API_VERSION.to_string(),
        service_version: SERVICE_VERSION.to_string(),
        orderbook_version: ORDERBOOK_RS_VERSION.to_string(),
        contract_compatible: true,
    }
}

fn build_submit(input: &SubmitOrderInput, claims: VerifiedClaims) -> (Command, ClientCommandKey) {
    let run_id = RunId::new(input.run_id.get());
    let actor = claims.participant_id;
    let local_command_id = input.command_id.get();
    let command_id = namespace_command_id(run_id, actor, claims.session_id, local_command_id);
    let command = Command {
        run_id,
        command_id,
        correlation_id: CorrelationId::new(command_id.get()),
        logical_time: LogicalTimeNs::new(input.logical_time_ns.get()),
        expected_sequence: EventSequence::new(input.expected_sequence.get()),
        actor,
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: namespace_order_id(run_id, actor, claims.session_id, input.order_id.get()),
            instrument_id: InstrumentId::new(input.instrument_id.get()),
            participant_id: actor,
            side: match input.side {
                ContractSide::Buy => Side::Buy,
                ContractSide::Sell => Side::Sell,
            },
            quantity: QuantityLots(input.quantity_lots.get()),
            kind: OrderKind::Limit {
                price: PriceTicks(input.price_ticks.get()),
            },
        }),
    };
    (
        command,
        ClientCommandKey {
            actor,
            session_id: claims.session_id,
            local_command_id,
            local_order_id: Some(input.order_id.get()),
        },
    )
}

fn build_cancel(input: &CancelOrderInput, claims: VerifiedClaims) -> (Command, ClientCommandKey) {
    let run_id = RunId::new(input.run_id.get());
    let actor = claims.participant_id;
    let local_command_id = input.command_id.get();
    let command_id = namespace_command_id(run_id, actor, claims.session_id, local_command_id);
    let command = Command {
        run_id,
        command_id,
        correlation_id: CorrelationId::new(command_id.get()),
        logical_time: LogicalTimeNs::new(input.logical_time_ns.get()),
        expected_sequence: EventSequence::new(input.expected_sequence.get()),
        actor,
        payload: CommandPayload::CancelOrder(CancelOrder {
            order_id: namespace_order_id(run_id, actor, claims.session_id, input.order_id.get()),
            participant_id: actor,
        }),
    };
    (
        command,
        ClientCommandKey {
            actor,
            session_id: claims.session_id,
            local_command_id,
            local_order_id: Some(input.order_id.get()),
        },
    )
}

async fn load_run(
    environment: &Env,
    run_id: RunId,
    instrument_id: InstrumentId,
) -> std::result::Result<RunState, ProcedureError> {
    let database = environment
        .d1("ORIGIN_DB")
        .map_err(|_| ProcedureError::OriginUnavailable)?;
    let state = d1_origin::load_run(&database, &run_id.to_string())
        .await
        .map_err(|error| map_origin_error(&error))?;
    state
        .listing_key_for_instrument(instrument_id)
        .map_err(|_| ProcedureError::NotFound)?;
    Ok(state)
}

fn snapshot_output(
    state: &RunState,
    instrument_id: InstrumentId,
) -> std::result::Result<MarketSnapshotOutput, ProcedureError> {
    let projection = project_market(state, instrument_id).map_err(|error| match error {
        bunting_application::ApplicationError::UnknownInstrument => ProcedureError::NotFound,
        _ => ProcedureError::InternalContractMismatch,
    })?;
    let levels = |items: Vec<(i64, i64)>| {
        items
            .into_iter()
            .map(|(price, quantity)| {
                Ok(PriceLevel {
                    price_ticks: SignedDecimalString::new(price),
                    quantity_lots: SignedDecimalString::new(quantity),
                })
            })
            .collect::<std::result::Result<Vec<_>, ProcedureError>>()
    };
    Ok(MarketSnapshotOutput {
        run_id: UnsignedDecimalString::new(projection.run_id.get()),
        instrument_id: UnsignedDecimalString::new(instrument_id.get()),
        sequence: SequenceDecimalString::new(projection.sequence.get()),
        bids: levels(projection.bids)?,
        asks: levels(projection.asks)?,
    })
}

async fn execute_command(
    client_command: (Command, ClientCommandKey),
    instrument_id: InstrumentId,
    environment: &Env,
) -> std::result::Result<CommandResult, ProcedureError> {
    execute_command_detailed(
        client_command.0,
        instrument_id,
        client_command.1,
        environment,
    )
    .await
    .map(|executed| executed.result)
}

pub(crate) struct ExecutedCommand {
    pub result: CommandResult,
    pub events: Vec<EventEnvelope>,
}

pub(crate) async fn execute_command_detailed(
    command: Command,
    instrument_id: InstrumentId,
    client_key: ClientCommandKey,
    environment: &Env,
) -> std::result::Result<ExecutedCommand, ProcedureError> {
    let database = environment
        .d1("ORIGIN_DB")
        .map_err(|_| ProcedureError::OriginUnavailable)?;
    let fingerprint =
        command_fingerprint(&command).map_err(|_| ProcedureError::InternalContractMismatch)?;
    if let Some((stored_fingerprint, result)) = d1_origin::find_command(
        &database,
        &command.run_id.to_string(),
        &command.command_id.to_string(),
    )
    .await
    .map_err(|error| map_origin_error(&error))?
    {
        return if stored_fingerprint == fingerprint {
            Ok(ExecutedCommand {
                result,
                events: Vec::new(),
            })
        } else {
            Err(ProcedureError::DuplicateCommandConflict)
        };
    }
    let state = load_run(environment, command.run_id, instrument_id).await?;
    let listing_key = state
        .listing_key_for_instrument(instrument_id)
        .map_err(|_| ProcedureError::NotFound)?;
    let snapshot = state
        .listing_snapshot(listing_key)
        .map_err(|_| ProcedureError::InternalContractMismatch)?;
    let cache_key = SnapshotCacheKey::new(
        state.run_id(),
        instrument_id,
        snapshot.represented_sequence,
        snapshot.checksum.clone(),
    )
    .map_err(|_| ProcedureError::InternalContractMismatch)?;
    let cached = match cloudflare::get_json(&cache_key).await {
        Ok(Some(package_json)) => Some(CachedSnapshot {
            listing_key,
            represented_sequence: snapshot.represented_sequence,
            checksum: snapshot.checksum.clone(),
            package_json,
        }),
        Ok(None) | Err(_) => None,
    };
    let actor = verified_participant(command.actor)?;
    let mut prepared =
        prepare_authenticated(&actor, &command, &state, cached.as_ref()).map_err(|error| {
            match error {
                bunting_application::ApplicationError::Transaction(transaction) => {
                    map_transaction_error(transaction)
                }
                bunting_application::ApplicationError::Unauthenticated
                | bunting_application::ApplicationError::Unauthorized
                | bunting_application::ApplicationError::ActorMismatch
                | bunting_application::ApplicationError::InvalidIdentity => {
                    ProcedureError::Unauthorized
                }
                _ => ProcedureError::InternalContractMismatch,
            }
        })?;
    prepared.commit.client_key = Some(client_key);
    let events = prepared.commit.events.clone();
    let command_json =
        serde_json::to_string(&command).map_err(|_| ProcedureError::InternalContractMismatch)?;
    let outcome = d1_origin::commit(&database, &prepared.commit, &command_json)
        .await
        .map_err(|error| map_origin_error(&error))?;
    let (result, events, publish_snapshot) = match outcome {
        CommitOutcome::Committed(result) => (result, events, true),
        CommitOutcome::Duplicate(result) => (result, Vec::new(), false),
    };

    // Origin commit above must succeed before this best-effort cache publication.
    if publish_snapshot
        && let Ok(snapshot) = prepared.commit.candidate.listing_snapshot(listing_key)
        && snapshot.represented_sequence == result.committed_sequence
        && let Ok(key) = SnapshotCacheKey::new(
            prepared.commit.run_id,
            listing_key.instrument_id,
            snapshot.represented_sequence,
            snapshot.checksum.clone(),
        )
    {
        let _cache_result =
            cloudflare::put_json(&key, snapshot.package_json.clone(), CachePolicy::default()).await;
    }
    Ok(ExecutedCommand { result, events })
}

pub(crate) async fn execute_simulation_detailed(
    request: SimulationCommandRequest,
    actor: &VerifiedActor,
    client_key: ClientCommandKey,
    environment: &Env,
) -> std::result::Result<ExecutedCommand, ProcedureError> {
    let database = environment
        .d1("ORIGIN_DB")
        .map_err(|_| ProcedureError::OriginUnavailable)?;
    let fingerprint = simulation_command_fingerprint(&request)
        .map_err(|_| ProcedureError::InternalContractMismatch)?;
    if let Some((stored_fingerprint, result)) = d1_origin::find_command(
        &database,
        &request.run_id.to_string(),
        &request.command_id.to_string(),
    )
    .await
    .map_err(|error| map_origin_error(&error))?
    {
        return if stored_fingerprint == fingerprint {
            Ok(ExecutedCommand {
                result,
                events: Vec::new(),
            })
        } else {
            Err(ProcedureError::DuplicateCommandConflict)
        };
    }
    let state = d1_origin::load_run(&database, &request.run_id.to_string())
        .await
        .map_err(|error| map_origin_error(&error))?;
    let mut prepared =
        prepare_authenticated_simulation(actor, &request, &state).map_err(|error| match error {
            bunting_application::ApplicationError::Transaction(transaction) => {
                map_transaction_error(transaction)
            }
            bunting_application::ApplicationError::Unauthenticated
            | bunting_application::ApplicationError::Unauthorized
            | bunting_application::ApplicationError::ActorMismatch
            | bunting_application::ApplicationError::InvalidIdentity => {
                ProcedureError::Unauthorized
            }
            _ => ProcedureError::InternalContractMismatch,
        })?;
    prepared.commit.client_key = Some(client_key);
    let events = prepared.commit.events.clone();
    let request_json =
        serde_json::to_string(&request).map_err(|_| ProcedureError::InternalContractMismatch)?;
    let outcome = d1_origin::commit(&database, &prepared.commit, &request_json)
        .await
        .map_err(|error| map_origin_error(&error))?;
    let (result, events) = match outcome {
        CommitOutcome::Committed(result) => (result, events),
        CommitOutcome::Duplicate(result) => (result, Vec::new()),
    };
    Ok(ExecutedCommand { result, events })
}

async fn dispatch_call(call: &Call, request: &Request, environment: &Env) -> WireResponse {
    match call.path.as_str() {
        "system.health" => bunting_browser_wire::success(200, &health()),
        "market.snapshot" => {
            let input = match decode_input::<MarketSnapshotInput>(call) {
                Ok(input) => input,
                Err(error) => return wire_error(error, &call.path, "invalid procedure input"),
            };
            match load_run(
                environment,
                RunId::new(input.run_id.get()),
                InstrumentId::new(input.instrument_id.get()),
            )
            .await
            .and_then(|state| snapshot_output(&state, InstrumentId::new(input.instrument_id.get())))
            {
                Ok(output) => bunting_browser_wire::success(200, &output),
                Err(error) => wire_error(error, &call.path, "snapshot unavailable"),
            }
        }
        "orders.submit" => {
            let Ok(claims) = authenticate(request, environment) else {
                return wire_error(
                    ProcedureError::Unauthorized,
                    &call.path,
                    "valid bearer claims required",
                );
            };
            let input = match decode_input::<SubmitOrderInput>(call) {
                Ok(input) => input,
                Err(error) => return wire_error(error, &call.path, "invalid procedure input"),
            };
            let instrument_id = InstrumentId::new(input.instrument_id.get());
            match execute_command(build_submit(&input, claims), instrument_id, environment).await {
                Ok(result) => command_response(&result, &call.path),
                Err(error) => wire_error(error, &call.path, "command rejected"),
            }
        }
        "orders.cancel" => {
            let Ok(claims) = authenticate(request, environment) else {
                return wire_error(
                    ProcedureError::Unauthorized,
                    &call.path,
                    "valid bearer claims required",
                );
            };
            let input = match decode_input::<CancelOrderInput>(call) {
                Ok(input) => input,
                Err(error) => return wire_error(error, &call.path, "invalid procedure input"),
            };
            let instrument_id = InstrumentId::new(input.instrument_id.get());
            match execute_command(build_cancel(&input, claims), instrument_id, environment).await {
                Ok(result) => command_response(&result, &call.path),
                Err(error) => wire_error(error, &call.path, "command rejected"),
            }
        }
        _ => wire_error(ProcedureError::NotFound, &call.path, "procedure not found"),
    }
}

fn method(request: &Request) -> Method {
    match request.method() {
        worker::Method::Get => Method::Get,
        worker::Method::Post => Method::Post,
        _ => Method::Other,
    }
}

#[event(fetch)]
pub async fn main(mut request: Request, environment: Env, _context: Context) -> Result<Response> {
    let url = request.url()?;
    let path = url.path().to_string();
    if let Some(session_path) = path.strip_prefix("/fix-sessions/") {
        let _claims = authenticate(&request, &environment)?;
        let session_id = session_path
            .split('/')
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| Error::RustError("missing FIX session identity".to_owned()))?;
        let namespace = environment.durable_object("FIX_SESSIONS")?;
        let stub = namespace.id_from_name(session_id)?.get_stub()?;
        return stub.fetch_with_request(request).await;
    }
    let query = url.query().map(str::to_string);
    let request_method = method(&request);
    let content_type = request.headers().get("content-type")?;
    let body = if request_method == Method::Post {
        request.bytes().await?
    } else {
        Vec::new()
    };
    let parsed = match bunting_browser_wire::parse(&WireRequest {
        method: request_method,
        path: &path,
        query: query.as_deref(),
        content_type: content_type.as_deref(),
        body: &body,
    }) {
        Ok(parsed) => parsed,
        Err(error) => return worker_response(bunting_browser_wire::error(&error)),
    };
    let response = match parsed {
        ParsedRequest::Query(call) | ParsedRequest::Mutation(call) => {
            dispatch_call(&call, &request, &environment).await
        }
        ParsedRequest::Subscription(call) => return subscribe(&call, &request, &environment).await,
    };
    worker_response(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_browser_wire::WireError;
    use bunting_market_types::OrderId;

    #[test]
    fn route_inventory_is_browser_api_only() {
        for path in [
            "/health",
            "/v1/cache/1/1/1/checksum",
            "/v1/runs/1/instruments/1/orders",
            "/internal/migrate",
        ] {
            let error = bunting_browser_wire::parse(&WireRequest {
                method: Method::Get,
                path,
                query: None,
                content_type: None,
                body: &[],
            });
            assert!(matches!(
                error,
                Err(WireError {
                    code: ErrorCode::NotFound,
                    ..
                })
            ));
        }
    }

    #[test]
    fn command_actor_comes_only_from_verified_claims() -> std::result::Result<(), WireError> {
        let call = bunting_browser_wire::parse(&WireRequest {
            method: Method::Post,
            path: "/api/orders.cancel",
            query: None,
            content_type: Some("application/json"),
            body: br#"{"runId":"1","instrumentId":"2","commandId":"3","correlationId":"4","expectedSequence":"5","logicalTimeNs":"6","orderId":"7"}"#,
        })?;
        let ParsedRequest::Mutation(call) = call else {
            return Err(WireError {
                code: ErrorCode::BadRequest,
                message: "mutation expected".to_string(),
                path: None,
            });
        };
        let input = decode_input::<CancelOrderInput>(&call).map_err(|_| WireError {
            code: ErrorCode::BadRequest,
            message: "valid contract input expected".to_string(),
            path: Some(call.path.clone()),
        })?;
        let (command, client_key) = build_cancel(
            &input,
            VerifiedClaims {
                participant_id: ParticipantId::new(91),
                session_id: SessionId::new(12),
            },
        );
        assert_eq!(command.actor, ParticipantId::new(91));
        assert_eq!(client_key.local_command_id, 3);
        let CommandPayload::CancelOrder(cancel) = command.payload else {
            return Err(WireError {
                code: ErrorCode::BadRequest,
                message: "cancel expected".to_string(),
                path: Some(call.path),
            });
        };
        assert_eq!(cancel.participant_id, ParticipantId::new(91));
        Ok(())
    }

    fn result(accepted: bool, reject_code: Option<&str>) -> CommandResult {
        CommandResult {
            accepted,
            reject_code: reject_code.map(str::to_string),
            committed_sequence: EventSequence::new(18_446_744_073_709_551_615),
            order_id: Some(OrderId::new(
                340_282_366_920_938_463_463_374_607_431_768_211_455,
            )),
            snapshot_checksum: Some("a".repeat(64)),
        }
    }

    #[test]
    fn complete_command_output_is_exact_and_duplicate_stable()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let committed = result(true, None);
        let first = command_response(&committed, "orders.submit");
        let duplicate = command_response(&committed, "orders.submit");
        assert_eq!(first, duplicate);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&first.body)?,
            serde_json::json!({"data":{
                "accepted":true,
                "rejectCode":null,
                "committedSequence":"18446744073709551615",
                "orderId":"340282366920938463463374607431768211455",
                "snapshotChecksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }})
        );
        Ok(())
    }

    #[test]
    fn version_duplicate_origin_and_input_failures_keep_stable_codes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                ProcedureError::VersionConflict,
                ErrorCode::Conflict,
                "VERSION_CONFLICT",
            ),
            (
                ProcedureError::DuplicateCommandConflict,
                ErrorCode::Conflict,
                "DUPLICATE_COMMAND_CONFLICT",
            ),
            (
                ProcedureError::OriginUnavailable,
                ErrorCode::InternalServerError,
                "ORIGIN_UNAVAILABLE",
            ),
            (
                ProcedureError::BadRequest,
                ErrorCode::BadRequest,
                "INVALID_INPUT",
            ),
        ];
        for (error, code, bunting_code) in cases {
            let response = wire_error(error, "orders.submit", "stable message");
            let body: serde_json::Value = serde_json::from_slice(&response.body)?;
            assert_eq!(response.status, code.status());
            assert_eq!(body["error"]["code"], code.name());
            assert_eq!(body["error"]["buntingCode"], bunting_code);
        }
        Ok(())
    }

    #[test]
    fn rejected_result_is_a_typed_risk_error_with_complete_durable_outcome()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let response = command_response(&result(false, Some("InsufficientCash")), "orders.submit");
        let body: serde_json::Value = serde_json::from_slice(&response.body)?;
        assert_eq!(response.status, 422);
        assert_eq!(body["error"]["code"], "UNPROCESSABLE_CONTENT");
        assert_eq!(body["error"]["buntingCode"], "RISK_REJECTED");
        assert_eq!(body["error"]["data"]["accepted"], false);
        assert_eq!(body["error"]["data"]["rejectCode"], "InsufficientCash");
        assert_eq!(
            body["error"]["data"]["committedSequence"],
            "18446744073709551615"
        );
        Ok(())
    }
}
