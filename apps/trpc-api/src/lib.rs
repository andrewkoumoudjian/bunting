#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native Rust tRPC Worker entrypoint for Bunting.
//!
//! The Worker exposes only the bounded `/trpc/<procedure-or-batch>` surface.
//! There is deliberately no REST router or Durable Object binding.

mod d1_origin;

use bunting_api_contract::{
    API_VERSION, CancelOrderInput, CommandOutput, HealthOutput, MarketSnapshotInput,
    MarketSnapshotOutput, PriceLevel, SequenceDecimalString, Side as ContractSide,
    SignedDecimalString, SubmitOrderInput, UnsignedDecimalString,
};
use bunting_command_transaction::{
    CachedSnapshot, TransactionError, command_fingerprint, prepare_command,
};
use bunting_market_events::{CancelOrder, Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, LogicalTimeNs, OrderId, ParticipantId,
    PriceTicks, QuantityLots, RunId,
};
use bunting_orderbook::{ORDERBOOK_RS_VERSION, visible_levels_from_snapshot_json};
use bunting_origin_store::{CommandResult, CommitOutcome, OriginError, RunState};
use bunting_trpc_wire::{
    Call, ErrorCode, Method, ParsedRequest, Request as WireRequest, Response as WireResponse,
};
use bunting_worker_cache::{CachePolicy, SnapshotCacheKey, cloudflare};
use serde::de::DeserializeOwned;
use worker::{Context, Env, Error, Request, Response, ResponseBuilder, Result, event};

const AUTH_TOKEN_BYTES: usize = 256;
const SERVICE_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy)]
struct VerifiedClaims {
    participant_id: ParticipantId,
}

#[derive(Clone, Copy)]
enum ProcedureError {
    Unauthorized,
    BadRequest,
    NotFound,
    Conflict,
    Internal,
}

impl ProcedureError {
    const fn code(self) -> ErrorCode {
        match self {
            Self::Unauthorized => ErrorCode::Unauthorized,
            Self::BadRequest => ErrorCode::BadRequest,
            Self::NotFound => ErrorCode::NotFound,
            Self::Conflict => ErrorCode::Conflict,
            Self::Internal => ErrorCode::InternalServerError,
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
    })
}

fn decode_input<T: DeserializeOwned>(call: &Call) -> std::result::Result<T, ProcedureError> {
    serde_json::from_value(call.input.clone().unwrap_or(serde_json::Value::Null))
        .map_err(|_| ProcedureError::BadRequest)
}

fn wire_error(error: ProcedureError, path: &str, message: &str) -> WireResponse {
    bunting_trpc_wire::procedure_error(error.code(), message, path)
}

fn worker_response(response: WireResponse) -> Result<Response> {
    Ok(ResponseBuilder::new()
        .with_status(response.status)
        .with_header("content-type", response.content_type)?
        .with_header("vary", response.vary)?
        .fixed(response.body))
}

fn health() -> HealthOutput {
    HealthOutput {
        api_version: API_VERSION.to_string(),
        service_version: SERVICE_VERSION.to_string(),
        orderbook_version: ORDERBOOK_RS_VERSION.to_string(),
        contract_compatible: true,
    }
}

fn build_submit(input: &SubmitOrderInput, actor: ParticipantId) -> Command {
    Command {
        run_id: RunId::new(input.run_id.get()),
        command_id: CommandId::new(input.command_id.get()),
        correlation_id: CorrelationId::new(input.correlation_id.get()),
        logical_time: LogicalTimeNs::new(input.logical_time_ns.get()),
        expected_sequence: EventSequence::new(input.expected_sequence.get()),
        actor,
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(input.order_id.get()),
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
    }
}

fn build_cancel(input: &CancelOrderInput, actor: ParticipantId) -> Command {
    Command {
        run_id: RunId::new(input.run_id.get()),
        command_id: CommandId::new(input.command_id.get()),
        correlation_id: CorrelationId::new(input.correlation_id.get()),
        logical_time: LogicalTimeNs::new(input.logical_time_ns.get()),
        expected_sequence: EventSequence::new(input.expected_sequence.get()),
        actor,
        payload: CommandPayload::CancelOrder(CancelOrder {
            order_id: OrderId::new(input.order_id.get()),
            participant_id: actor,
        }),
    }
}

async fn load_run(
    environment: &Env,
    run_id: RunId,
    instrument_id: InstrumentId,
) -> std::result::Result<RunState, ProcedureError> {
    let database = environment
        .d1("ORIGIN_DB")
        .map_err(|_| ProcedureError::Internal)?;
    let state = d1_origin::load_run(&database, &run_id.to_string())
        .await
        .map_err(|error| match error {
            OriginError::UnknownRun => ProcedureError::NotFound,
            _ => ProcedureError::Internal,
        })?;
    if state.instrument_id != instrument_id {
        return Err(ProcedureError::NotFound);
    }
    Ok(state)
}

fn snapshot_output(state: &RunState) -> std::result::Result<MarketSnapshotOutput, ProcedureError> {
    let (upstream_bids, upstream_asks) =
        visible_levels_from_snapshot_json(&state.snapshot.package_json)
            .map_err(|_| ProcedureError::Internal)?;
    let levels = |items: Vec<(u128, u64)>| {
        items
            .into_iter()
            .map(|(price, quantity)| {
                let price = i64::try_from(price).map_err(|_| ProcedureError::Internal)?;
                let quantity = i64::try_from(quantity).map_err(|_| ProcedureError::Internal)?;
                Ok(PriceLevel {
                    price_ticks: SignedDecimalString::new(price),
                    quantity_lots: SignedDecimalString::new(quantity),
                })
            })
            .collect::<std::result::Result<Vec<_>, ProcedureError>>()
    };
    Ok(MarketSnapshotOutput {
        run_id: UnsignedDecimalString::new(state.run_id.get()),
        instrument_id: UnsignedDecimalString::new(state.instrument_id.get()),
        sequence: SequenceDecimalString::new(state.version.get()),
        bids: levels(upstream_bids)?,
        asks: levels(upstream_asks)?,
    })
}

async fn execute_command(
    command: Command,
    instrument_id: InstrumentId,
    environment: &Env,
) -> std::result::Result<CommandResult, ProcedureError> {
    let database = environment
        .d1("ORIGIN_DB")
        .map_err(|_| ProcedureError::Internal)?;
    let fingerprint = command_fingerprint(&command).map_err(|_| ProcedureError::Internal)?;
    if let Some((stored_fingerprint, result)) = d1_origin::find_command(
        &database,
        &command.run_id.to_string(),
        &command.command_id.to_string(),
    )
    .await
    .map_err(|_| ProcedureError::Internal)?
    {
        return if stored_fingerprint == fingerprint {
            Ok(result)
        } else {
            Err(ProcedureError::Conflict)
        };
    }
    let state = load_run(environment, command.run_id, instrument_id).await?;
    let cache_key = SnapshotCacheKey::new(
        state.run_id,
        state.instrument_id,
        state.snapshot.represented_sequence,
        state.snapshot.checksum.clone(),
    )
    .map_err(|_| ProcedureError::Internal)?;
    let cached = match cloudflare::get_json(&cache_key).await {
        Ok(Some(package_json)) => Some(CachedSnapshot {
            represented_sequence: state.snapshot.represented_sequence,
            checksum: state.snapshot.checksum.clone(),
            package_json,
        }),
        Ok(None) | Err(_) => None,
    };
    let prepared = prepare_command(&command, state, cached).map_err(|error| match error {
        TransactionError::Origin(OriginError::VersionConflict { .. }) => ProcedureError::Conflict,
        _ => ProcedureError::Internal,
    })?;
    let command_json = serde_json::to_string(&command).map_err(|_| ProcedureError::Internal)?;
    let outcome = d1_origin::commit(&database, &prepared.commit, &command_json)
        .await
        .map_err(|error| match error {
            OriginError::VersionConflict { .. } | OriginError::IdempotencyConflict => {
                ProcedureError::Conflict
            }
            OriginError::UnknownRun => ProcedureError::NotFound,
            _ => ProcedureError::Internal,
        })?;
    let result = match outcome {
        CommitOutcome::Committed(result) | CommitOutcome::Duplicate(result) => result,
    };

    // Origin commit above must succeed before this best-effort cache publication.
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
    Ok(result)
}

async fn dispatch_call(call: &Call, request: &Request, environment: &Env) -> WireResponse {
    match call.path.as_str() {
        "system.health" => bunting_trpc_wire::success(200, &health()),
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
            .and_then(|state| snapshot_output(&state))
            {
                Ok(output) => bunting_trpc_wire::success(200, &output),
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
            match execute_command(
                build_submit(&input, claims.participant_id),
                instrument_id,
                environment,
            )
            .await
            {
                Ok(result) => bunting_trpc_wire::success(
                    200,
                    &CommandOutput {
                        accepted: result.accepted,
                        sequence: SequenceDecimalString::new(result.committed_sequence.get()),
                    },
                ),
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
            match execute_command(
                build_cancel(&input, claims.participant_id),
                instrument_id,
                environment,
            )
            .await
            {
                Ok(result) => bunting_trpc_wire::success(
                    200,
                    &CommandOutput {
                        accepted: result.accepted,
                        sequence: SequenceDecimalString::new(result.committed_sequence.get()),
                    },
                ),
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
    let query = url.query().map(str::to_string);
    let request_method = method(&request);
    let content_type = request.headers().get("content-type")?;
    let body = if request_method == Method::Post {
        request.bytes().await?
    } else {
        Vec::new()
    };
    let parsed = match bunting_trpc_wire::parse(&WireRequest {
        method: request_method,
        path: &path,
        query: query.as_deref(),
        content_type: content_type.as_deref(),
        body: &body,
    }) {
        Ok(parsed) => parsed,
        Err(error) => return worker_response(bunting_trpc_wire::error(&error)),
    };
    let response = match parsed {
        ParsedRequest::Query(call) | ParsedRequest::Mutation(call) => {
            dispatch_call(&call, &request, &environment).await
        }
        ParsedRequest::QueryBatch(calls) => {
            let mut results = Vec::with_capacity(calls.len());
            for call in &calls {
                results.push(dispatch_call(call, &request, &environment).await);
            }
            bunting_trpc_wire::batch_responses(&results)
        }
    };
    worker_response(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bunting_trpc_wire::WireError;

    #[test]
    fn route_inventory_is_trpc_only() {
        for path in [
            "/health",
            "/v1/cache/1/1/1/checksum",
            "/v1/runs/1/instruments/1/orders",
            "/internal/migrate",
        ] {
            let error = bunting_trpc_wire::parse(&WireRequest {
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
        let call = bunting_trpc_wire::parse(&WireRequest {
            method: Method::Post,
            path: "/trpc/orders.cancel",
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
        let command = build_cancel(&input, ParticipantId::new(91));
        assert_eq!(command.actor, ParticipantId::new(91));
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
}
