use crate::config::AdminConfig;
use crate::session_host::constant_time_eq;
use crate::storage::NativeOrigin;
use bunting_market_types::RunId;
use bunting_origin_store::{OriginError, OriginStore};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const HEALTH_CONTRACT_VERSION: u16 = 1;

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: serde_json::Value,
}

#[derive(Debug)]
enum AdminRequest {
    Health,
    Run(RunId),
    Immediate(HttpResponse),
}

pub(crate) fn run(config: &AdminConfig, origin: &NativeOrigin) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind admin listener {}: {error}", config.bind))?;
    for accepted in listener.incoming() {
        let mut stream = match accepted {
            Ok(stream) => stream,
            Err(error) => {
                eprintln!("bunting-server: admin accept failed: {error}");
                continue;
            }
        };
        if let Err(error) = handle(&mut stream, config, origin) {
            eprintln!("bunting-server: admin connection closed: {error}");
        }
    }
    Ok(())
}

fn health_body() -> serde_json::Value {
    serde_json::json!({
        "status": "ok",
        "service": crate::SERVICE_NAME,
        "healthContractVersion": HEALTH_CONTRACT_VERSION,
        "fixCompetitionProfileVersion": bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION,
    })
}

fn classify_request(request: &str, bearer_token: &str) -> AdminRequest {
    let first = request.lines().next().unwrap_or_default();
    if first == "GET /health HTTP/1.1" {
        return AdminRequest::Health;
    }
    if let Some(run) = first
        .strip_prefix("GET /admin/runs/")
        .and_then(|value| value.strip_suffix(" HTTP/1.1"))
    {
        let authorized = request.lines().any(|line| {
            line.strip_prefix("Authorization: Bearer ")
                .is_some_and(|value| constant_time_eq(value, bearer_token))
        });
        if !authorized {
            return AdminRequest::Immediate(HttpResponse {
                status: 401,
                body: serde_json::json!({"error":"unauthorized"}),
            });
        }
        let Ok(run_id) = run.parse::<u128>() else {
            return AdminRequest::Immediate(HttpResponse {
                status: 400,
                body: serde_json::json!({"error":"invalid_run_id"}),
            });
        };
        return AdminRequest::Run(RunId::new(run_id));
    }
    AdminRequest::Immediate(HttpResponse {
        status: 404,
        body: serde_json::json!({"error":"not_found"}),
    })
}

fn handle(
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
    match classify_request(request, &config.bearer_token) {
        AdminRequest::Health => write_http(stream, 200, &health_body()),
        AdminRequest::Immediate(response) => write_http(stream, response.status, &response.body),
        AdminRequest::Run(run_id) => match origin.load_run(run_id) {
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
            Err(error) => Err(format!("origin store error: {error}")),
        },
    }
}

fn write_http(stream: &mut TcpStream, status: u16, body: &serde_json::Value) -> Result<(), String> {
    let body =
        serde_json::to_vec(body).map_err(|error| format!("cannot encode response: {error}"))?;
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_contract_identifies_bunting_and_protocol_version() {
        let body = health_body();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["service"], crate::SERVICE_NAME);
        assert_eq!(body["healthContractVersion"], 1);
        assert_eq!(
            body["fixCompetitionProfileVersion"],
            bunting_api_contract::FIX_COMPETITION_PROFILE_VERSION
        );
    }

    #[test]
    fn malformed_admin_run_id_is_a_client_error() -> Result<(), String> {
        let request = "GET /admin/runs/not-a-number HTTP/1.1\r\nAuthorization: Bearer x\r\n\r\n";
        let AdminRequest::Immediate(response) = classify_request(request, "x") else {
            return Err("malformed run ID did not produce an immediate client response".to_owned());
        };
        assert_eq!(response.status, 400);
        assert_eq!(response.body["error"], "invalid_run_id");
        Ok(())
    }
}
