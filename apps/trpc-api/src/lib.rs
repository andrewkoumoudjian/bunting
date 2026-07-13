#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Native Rust tRPC Worker entrypoint for Bunting.
//!
//! The Worker exposes only the bounded `/trpc/<procedure-or-batch>` surface.
//! There is deliberately no REST router or Durable Object binding.

mod d1_origin;

use bunting_api_contract::{
    API_VERSION, BuntingErrorCode, CancelOrderInput, CommandOutput, HealthOutput,
    MarketSnapshotInput, MarketSnapshotOutput, PriceLevel, SequenceDecimalString,
    Side as ContractSide, SignedDecimalString, SubmitOrderInput, UnsignedDecimalString,
};
use bunting_command_transaction::{
    CachedSnapshot, TransactionError, command_fingerprint, prepare_command,
};
use bunting_engine::{ORDERBOOK_RS_VERSION, RunState};
use bunting_market_events::{CancelOrder, Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, LogicalTimeNs, OrderId, ParticipantId,
    PriceTicks, QuantityLots, RunId,
};
use bunting_origin_store::{CommandResult, CommitOutcome, OriginError};
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
    })
}

fn decode_input<T: DeserializeOwned>(call: &Call) -> std::result::Result<T, ProcedureError> {
    serde_json::from_value(call.input.clone().unwrap_or(serde_json::Value::Null))
        .map_err(|_| ProcedureError::BadRequest)
}

fn wire_error(error: ProcedureError, path: &str, message: &str) -> WireResponse {
    bunting_trpc_wire::procedure_error(error.code(), error.bunting_code(), message, path)
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
        bunting_trpc_wire::success(200, &output)
    } else {
        bunting_trpc_wire::procedure_error_with_data(
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
    let listing_key = state
        .listing_key_for_instrument(instrument_id)
        .map_err(|_| ProcedureError::NotFound)?;
    let (upstream_bids, upstream_asks) = state
        .visible_levels(listing_key)
        .map_err(|_| ProcedureError::InternalContractMismatch)?;
    let levels = |items: Vec<(u128, u64)>| {
        items
            .into_iter()
            .map(|(price, quantity)| {
                let price =
                    i64::try_from(price).map_err(|_| ProcedureError::InternalContractMismatch)?;
                let quantity = i64::try_from(quantity)
                    .map_err(|_| ProcedureError::InternalContractMismatch)?;
                Ok(PriceLevel {
                    price_ticks: SignedDecimalString::new(price),
                    quantity_lots: SignedDecimalString::new(quantity),
                })
            })
            .collect::<std::result::Result<Vec<_>, ProcedureError>>()
    };
    Ok(MarketSnapshotOutput {
        run_id: UnsignedDecimalString::new(state.run_id().get()),
        instrument_id: UnsignedDecimalString::new(instrument_id.get()),
        sequence: SequenceDecimalString::new(state.sequence().get()),
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
            Ok(result)
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
    let prepared =
        prepare_command(&command, &state, cached.as_ref()).map_err(map_transaction_error)?;
    let command_json =
        serde_json::to_string(&command).map_err(|_| ProcedureError::InternalContractMismatch)?;
    let outcome = d1_origin::commit(&database, &prepared.commit, &command_json)
        .await
        .map_err(|error| map_origin_error(&error))?;
    let result = match outcome {
        CommitOutcome::Committed(result) | CommitOutcome::Duplicate(result) => result,
    };

    // Origin commit above must succeed before this best-effort cache publication.
    if let Ok(snapshot) = prepared.commit.candidate.listing_snapshot(listing_key)
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
            .and_then(|state| snapshot_output(&state, InstrumentId::new(input.instrument_id.get())))
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
            match execute_command(
                build_cancel(&input, claims.participant_id),
                instrument_id,
                environment,
            )
            .await
            {
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
            serde_json::json!({"result":{"data":{
                "accepted":true,
                "rejectCode":null,
                "committedSequence":"18446744073709551615",
                "orderId":"340282366920938463463374607431768211455",
                "snapshotChecksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
            }}})
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
            assert_eq!(body["error"]["data"]["code"], code.name());
            assert_eq!(body["error"]["data"]["buntingCode"], bunting_code);
        }
        Ok(())
    }

    #[test]
    fn rejected_result_is_a_typed_risk_error_with_complete_durable_outcome()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let response = command_response(&result(false, Some("InsufficientCash")), "orders.submit");
        let body: serde_json::Value = serde_json::from_slice(&response.body)?;
        assert_eq!(response.status, 422);
        assert_eq!(body["error"]["data"]["code"], "UNPROCESSABLE_CONTENT");
        assert_eq!(body["error"]["data"]["buntingCode"], "RISK_REJECTED");
        assert_eq!(body["error"]["data"]["buntingData"]["accepted"], false);
        assert_eq!(
            body["error"]["data"]["buntingData"]["rejectCode"],
            "InsufficientCash"
        );
        assert_eq!(
            body["error"]["data"]["buntingData"]["committedSequence"],
            "18446744073709551615"
        );
        Ok(())
    }
}
