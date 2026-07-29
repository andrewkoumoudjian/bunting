use crate::config::AdminConfig;
use crate::session_host::constant_time_eq;
use crate::storage::NativeOrigin;
use bunting_market_types::RunId;
use bunting_origin_store::{OriginError, OriginStore};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub(crate) fn run(config: &AdminConfig, origin: &NativeOrigin) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind admin listener {}: {error}", config.bind))?;
    for accepted in listener.incoming() {
        let mut stream = accepted.map_err(|error| format!("admin accept failed: {error}"))?;
        handle(&mut stream, config, origin)?;
    }
    Ok(())
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
            Err(error) => Err(format!("origin store error: {error}")),
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
