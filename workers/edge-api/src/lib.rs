#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Plain Cloudflare Worker entrypoint for Bunting.
//!
//! There is deliberately no Durable Object binding. The Worker uses the Workers
//! Cache API for immutable, checksum-addressed `OrderBook-rs` snapshot packages.

mod d1_origin;

use bunting_command_transaction::{
    CachedSnapshot, TransactionError, command_fingerprint, prepare_command,
};
use bunting_market_events::{CancelOrder, Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, LogicalTimeNs, OrderId, ParticipantId,
    PriceTicks, QuantityLots, RunId,
};
use bunting_orderbook::{ORDERBOOK_RS_AUDIT_COMMIT, ORDERBOOK_RS_VERSION};
use bunting_origin_store::{CommandResult, CommitOutcome, OriginError};
use bunting_worker_cache::{CachePolicy, SnapshotCacheKey, cloudflare};
use serde::{Deserialize, Serialize};
use worker::{
    Context, Env, Error, Request, Response, ResponseBuilder, Result, RouteContext, Router, event,
};

const MAX_COMMAND_BYTES: usize = 16 * 1024;
const AUTH_TOKEN_BYTES: usize = 256;

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum JsonSide {
    Buy,
    Sell,
}

#[derive(Deserialize)]
struct OrderRequest {
    command_id: String,
    correlation_id: String,
    expected_sequence: String,
    logical_time_ns: String,
    order_id: String,
    side: JsonSide,
    price_ticks: i64,
    quantity_lots: i64,
}

#[derive(Deserialize)]
struct CancelRequest {
    command_id: String,
    correlation_id: String,
    expected_sequence: String,
    logical_time_ns: String,
}

#[derive(Serialize)]
struct CommandResponse {
    accepted: bool,
    reject_code: Option<String>,
    committed_sequence: String,
    order_id: Option<String>,
    snapshot_checksum: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse<'a> {
    code: &'a str,
    message: &'a str,
}

#[derive(Serialize)]
struct HealthResponse<'a> {
    status: &'a str,
    runtime: &'a str,
    orderbook_rs_version: &'a str,
    orderbook_rs_audit_commit: &'a str,
    snapshot_cache: &'a str,
}

async fn health(_: Request, _: RouteContext<()>) -> Result<Response> {
    Response::from_json(&HealthResponse {
        status: "ok",
        runtime: "cloudflare-worker",
        orderbook_rs_version: ORDERBOOK_RS_VERSION,
        orderbook_rs_audit_commit: ORDERBOOK_RS_AUDIT_COMMIT,
        snapshot_cache: "workers-cache",
    })
}

fn required_param<'a>(context: &'a RouteContext<()>, name: &str) -> Result<&'a str> {
    context
        .param(name)
        .map(String::as_str)
        .ok_or_else(|| Error::RustError(format!("missing route parameter: {name}")))
}

fn parse_u128(context: &RouteContext<()>, name: &str) -> Result<u128> {
    required_param(context, name)?
        .parse()
        .map_err(|_| Error::RustError(format!("invalid {name}")))
}

fn parse_u64(context: &RouteContext<()>, name: &str) -> Result<u64> {
    required_param(context, name)?
        .parse()
        .map_err(|_| Error::RustError(format!("invalid {name}")))
}

async fn cached_snapshot(_: Request, context: RouteContext<()>) -> Result<Response> {
    let key = SnapshotCacheKey::new(
        RunId::new(parse_u128(&context, "run_id")?),
        InstrumentId::new(parse_u128(&context, "instrument_id")?),
        EventSequence::new(parse_u64(&context, "sequence")?),
        required_param(&context, "checksum")?,
    )
    .map_err(|error| Error::RustError(error.to_string()))?;

    match cloudflare::get_json(&key).await? {
        Some(snapshot) => Ok(ResponseBuilder::new()
            .with_header("content-type", "application/json")?
            .with_header("x-bunting-cache", "HIT")?
            .with_header("etag", &key.etag())?
            .fixed(snapshot.into_bytes())),
        None => Response::error("snapshot cache miss", 404),
    }
}

fn error_response(code: &str, message: &str, status: u16) -> Result<Response> {
    Response::from_json(&ErrorResponse { code, message })
        .map(|response| response.with_status(status))
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

fn authenticate(request: &Request, context: &RouteContext<()>) -> Result<ParticipantId> {
    let authorization = request
        .headers()
        .get("authorization")?
        .ok_or_else(|| Error::RustError("missing authorization".to_string()))?;
    let provided = authorization
        .strip_prefix("Bearer ")
        .ok_or_else(|| Error::RustError("invalid authorization scheme".to_string()))?;
    let expected = context.secret("BUNTING_API_TOKEN")?.to_string();
    if !constant_time_token_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(Error::RustError("invalid bearer token".to_string()));
    }
    let actor = request
        .headers()
        .get("x-bunting-participant-id")?
        .ok_or_else(|| Error::RustError("missing participant identity".to_string()))?
        .parse::<u128>()
        .map_err(|_| Error::RustError("invalid participant identity".to_string()))?;
    Ok(ParticipantId::new(actor))
}

async fn bounded_json<T: for<'de> Deserialize<'de>>(request: &mut Request) -> Result<T> {
    let bytes = request.bytes().await?;
    if bytes.len() > MAX_COMMAND_BYTES {
        return Err(Error::RustError(
            "command payload exceeds 16 KiB".to_string(),
        ));
    }
    serde_json::from_slice(&bytes).map_err(|error| Error::RustError(error.to_string()))
}

fn command_response(result: CommandResult) -> Result<Response> {
    let status = if result.accepted { 200 } else { 422 };
    Response::from_json(&CommandResponse {
        accepted: result.accepted,
        reject_code: result.reject_code,
        committed_sequence: result.committed_sequence.to_string(),
        order_id: result.order_id.map(|order_id| order_id.to_string()),
        snapshot_checksum: result.snapshot_checksum,
    })
    .map(|response| response.with_status(status))
}

async fn execute_command(command: Command, context: &RouteContext<()>) -> Result<Response> {
    let database = context.d1("ORIGIN_DB")?;
    let fingerprint =
        command_fingerprint(&command).map_err(|error| Error::RustError(error.to_string()))?;
    if let Some((stored_fingerprint, result)) = d1_origin::find_command(
        &database,
        &command.run_id.to_string(),
        &command.command_id.to_string(),
    )
    .await
    .map_err(|error| Error::RustError(error.to_string()))?
    {
        return if stored_fingerprint == fingerprint {
            command_response(result)
        } else {
            error_response(
                "command_id_conflict",
                "command_id is already bound to a different canonical payload",
                409,
            )
        };
    }
    let state = match d1_origin::load_run(&database, &command.run_id.to_string()).await {
        Ok(state) => state,
        Err(OriginError::UnknownRun) => {
            return error_response("unknown_run", "run does not exist", 404);
        }
        Err(error) => return error_response("origin_failure", &error.to_string(), 500),
    };
    let route_instrument = InstrumentId::new(parse_u128(context, "instrument_id")?);
    if state.instrument_id != route_instrument {
        return error_response(
            "unknown_instrument",
            "instrument does not exist in run",
            404,
        );
    }
    let cache_key = SnapshotCacheKey::new(
        state.run_id,
        state.instrument_id,
        state.snapshot.represented_sequence,
        state.snapshot.checksum.clone(),
    )
    .map_err(|error| Error::RustError(error.to_string()))?;
    let cached = match cloudflare::get_json(&cache_key).await {
        Ok(Some(package_json)) => Some(CachedSnapshot {
            represented_sequence: state.snapshot.represented_sequence,
            checksum: state.snapshot.checksum.clone(),
            package_json,
        }),
        Ok(None) | Err(_) => None,
    };
    let prepared = match prepare_command(&command, state, cached) {
        Ok(prepared) => prepared,
        Err(TransactionError::Origin(OriginError::VersionConflict { current })) => {
            return error_response(
                "expected_sequence_conflict",
                &format!("current sequence is {current}"),
                409,
            );
        }
        Err(error) => return error_response("command_failed", &error.to_string(), 500),
    };
    let command_json =
        serde_json::to_string(&command).map_err(|error| Error::RustError(error.to_string()))?;
    let outcome = match d1_origin::commit(&database, &prepared.commit, &command_json).await {
        Ok(outcome) => outcome,
        Err(OriginError::VersionConflict { current }) => {
            return error_response(
                "expected_sequence_conflict",
                &format!("current sequence is {current}"),
                409,
            );
        }
        Err(OriginError::IdempotencyConflict) => {
            return error_response(
                "command_id_conflict",
                "command_id is already bound to a different canonical payload",
                409,
            );
        }
        Err(error) => return error_response("origin_failure", &error.to_string(), 500),
    };
    let result = match outcome {
        CommitOutcome::Committed(result) | CommitOutcome::Duplicate(result) => result,
    };
    let snapshot = &prepared.commit.candidate.snapshot;
    if let Ok(key) = SnapshotCacheKey::new(
        prepared.commit.run_id,
        snapshot.instrument_id,
        snapshot.represented_sequence,
        snapshot.checksum.clone(),
    ) {
        let _cache_result =
            cloudflare::put_json(&key, snapshot.package_json.clone(), CachePolicy::default()).await;
    }
    command_response(result)
}

async fn submit_order(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let Ok(actor) = authenticate(&request, &context) else {
        return error_response("unauthorized", "valid bearer token required", 401);
    };
    let body: OrderRequest = match bounded_json(&mut request).await {
        Ok(body) => body,
        Err(_) => return error_response("malformed_request", "invalid bounded JSON body", 400),
    };
    let Ok(command) = build_submit_command(&body, actor, &context) else {
        return error_response("malformed_request", "invalid exact command fields", 400);
    };
    execute_command(command, &context).await
}

fn build_submit_command(
    body: &OrderRequest,
    actor: ParticipantId,
    context: &RouteContext<()>,
) -> Result<Command> {
    let run_id = parse_u128(context, "run_id")?;
    let instrument_id = parse_u128(context, "instrument_id")?;
    let parse_id = |value: &str| {
        value
            .parse::<u128>()
            .map_err(|_| Error::RustError("invalid exact identifier".to_string()))
    };
    let command = Command {
        run_id: RunId::new(run_id),
        command_id: CommandId::new(parse_id(&body.command_id)?),
        correlation_id: CorrelationId::new(parse_id(&body.correlation_id)?),
        logical_time: LogicalTimeNs::new(
            body.logical_time_ns
                .parse()
                .map_err(|_| Error::RustError("invalid logical_time_ns".to_string()))?,
        ),
        expected_sequence: EventSequence::new(
            body.expected_sequence
                .parse()
                .map_err(|_| Error::RustError("invalid expected_sequence".to_string()))?,
        ),
        actor,
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(parse_id(&body.order_id)?),
            instrument_id: InstrumentId::new(instrument_id),
            participant_id: actor,
            side: match body.side {
                JsonSide::Buy => Side::Buy,
                JsonSide::Sell => Side::Sell,
            },
            quantity: QuantityLots(body.quantity_lots),
            kind: OrderKind::Limit {
                price: PriceTicks(body.price_ticks),
            },
        }),
    };
    Ok(command)
}

async fn cancel_order(mut request: Request, context: RouteContext<()>) -> Result<Response> {
    let Ok(actor) = authenticate(&request, &context) else {
        return error_response("unauthorized", "valid bearer token required", 401);
    };
    let body: CancelRequest = match bounded_json(&mut request).await {
        Ok(body) => body,
        Err(_) => return error_response("malformed_request", "invalid bounded JSON body", 400),
    };
    let Ok(command) = build_cancel_command(&body, actor, &context) else {
        return error_response("malformed_request", "invalid exact command fields", 400);
    };
    execute_command(command, &context).await
}

fn build_cancel_command(
    body: &CancelRequest,
    actor: ParticipantId,
    context: &RouteContext<()>,
) -> Result<Command> {
    let _instrument_id = parse_u128(context, "instrument_id")?;
    Ok(Command {
        run_id: RunId::new(parse_u128(context, "run_id")?),
        command_id: CommandId::new(
            body.command_id
                .parse()
                .map_err(|_| Error::RustError("invalid command_id".to_string()))?,
        ),
        correlation_id: CorrelationId::new(
            body.correlation_id
                .parse()
                .map_err(|_| Error::RustError("invalid correlation_id".to_string()))?,
        ),
        logical_time: LogicalTimeNs::new(
            body.logical_time_ns
                .parse()
                .map_err(|_| Error::RustError("invalid logical_time_ns".to_string()))?,
        ),
        expected_sequence: EventSequence::new(
            body.expected_sequence
                .parse()
                .map_err(|_| Error::RustError("invalid expected_sequence".to_string()))?,
        ),
        actor,
        payload: CommandPayload::CancelOrder(CancelOrder {
            order_id: OrderId::new(parse_u128(context, "order_id")?),
            participant_id: actor,
        }),
    })
}

#[event(fetch)]
pub async fn main(request: Request, environment: Env, _context: Context) -> Result<Response> {
    Router::new()
        .get_async("/health", health)
        .get_async(
            "/v1/cache/:run_id/:instrument_id/:sequence/:checksum",
            cached_snapshot,
        )
        .post_async(
            "/v1/runs/:run_id/instruments/:instrument_id/orders",
            submit_order,
        )
        .post_async(
            "/v1/runs/:run_id/instruments/:instrument_id/orders/:order_id/cancel",
            cancel_order,
        )
        .run(request, environment)
        .await
}
