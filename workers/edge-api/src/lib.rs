#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Plain Cloudflare Worker entrypoint for Bunting.
//!
//! There is deliberately no Durable Object binding. The Worker uses the Workers
//! Cache API for immutable, checksum-addressed `OrderBook-rs` snapshot packages.

use bunting_market_types::{EventSequence, InstrumentId, RunId};
use bunting_orderbook::{ORDERBOOK_RS_AUDIT_COMMIT, ORDERBOOK_RS_VERSION};
use bunting_worker_cache::{SnapshotCacheKey, cloudflare};
use serde::Serialize;
use worker::{
    event, Context, Env, Error, Request, Response, ResponseBuilder, Result, RouteContext, Router,
};

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

#[event(fetch)]
pub async fn main(request: Request, environment: Env, _context: Context) -> Result<Response> {
    Router::new()
        .get_async("/health", health)
        .get_async(
            "/v1/cache/:run_id/:instrument_id/:sequence/:checksum",
            cached_snapshot,
        )
        .run(request, environment)
        .await
}
