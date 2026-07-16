#![forbid(unsafe_code)]
//! Bounded sans-I/O parsing and encoding for Bunting's pinned tRPC HTTP subset.

use bunting_api_contract::{BuntingErrorCode, ProcedureKind, procedure_kind};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use serde_json::{Value, json};

pub const MAX_REQUEST_BYTES: usize = 16_384;
pub const MAX_QUERY_CALLS: usize = 16;
pub const MAX_PROCEDURE_PATH_BYTES: usize = 2_048;
pub const MAX_JSON_DEPTH: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Request<'a> {
    pub method: Method,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub content_type: Option<&'a str>,
    pub body: &'a [u8],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Call {
    pub path: String,
    pub input: Option<Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParsedRequest {
    Query(Call),
    Mutation(Call),
    Subscription(Call),
    QueryBatch(Vec<Call>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    BadRequest,
    InternalServerError,
    Unauthorized,
    NotFound,
    MethodNotSupported,
    Conflict,
    UnprocessableContent,
    UnsupportedMediaType,
    PayloadTooLarge,
}

impl ErrorCode {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::BadRequest => "BAD_REQUEST",
            Self::InternalServerError => "INTERNAL_SERVER_ERROR",
            Self::Unauthorized => "UNAUTHORIZED",
            Self::NotFound => "NOT_FOUND",
            Self::MethodNotSupported => "METHOD_NOT_SUPPORTED",
            Self::Conflict => "CONFLICT",
            Self::UnprocessableContent => "UNPROCESSABLE_CONTENT",
            Self::UnsupportedMediaType => "UNSUPPORTED_MEDIA_TYPE",
            Self::PayloadTooLarge => "PAYLOAD_TOO_LARGE",
        }
    }
    #[must_use]
    pub const fn numeric(self) -> i32 {
        match self {
            Self::BadRequest => -32600,
            Self::InternalServerError => -32603,
            Self::Unauthorized => -32001,
            Self::NotFound => -32004,
            Self::MethodNotSupported => -32005,
            Self::Conflict => -32009,
            Self::UnprocessableContent => -32022,
            Self::UnsupportedMediaType => -32015,
            Self::PayloadTooLarge => -32013,
        }
    }
    #[must_use]
    pub const fn status(self) -> u16 {
        match self {
            Self::BadRequest => 400,
            Self::InternalServerError => 500,
            Self::Unauthorized => 401,
            Self::NotFound => 404,
            Self::MethodNotSupported => 405,
            Self::Conflict => 409,
            Self::UnprocessableContent => 422,
            Self::UnsupportedMediaType => 415,
            Self::PayloadTooLarge => 413,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WireError {
    pub code: ErrorCode,
    pub message: String,
    pub path: Option<String>,
}

impl WireError {
    fn new(code: ErrorCode, message: impl Into<String>, path: Option<&str>) -> Self {
        Self {
            code,
            message: message.into(),
            path: path.map(str::to_owned),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Response {
    pub status: u16,
    pub content_type: &'static str,
    pub vary: &'static str,
    pub body: Vec<u8>,
}

/// Parses one bounded tRPC request without performing I/O.
///
/// # Errors
/// Returns a structured [`WireError`] when the request is malformed, unsupported, or out of bounds.
pub fn parse(request: &Request<'_>) -> Result<ParsedRequest, WireError> {
    if request.path.len() > MAX_PROCEDURE_PATH_BYTES || request.body.len() > MAX_REQUEST_BYTES {
        return Err(WireError::new(
            ErrorCode::PayloadTooLarge,
            "request exceeds Bunting bounds",
            None,
        ));
    }
    let encoded = request
        .path
        .strip_prefix("/trpc/")
        .ok_or_else(|| WireError::new(ErrorCode::NotFound, "No procedure found", None))?;
    validate_percent_encoding(encoded)?;
    let decoded = percent_decode_str(encoded).decode_utf8().map_err(|_| {
        WireError::new(
            ErrorCode::BadRequest,
            "malformed procedure path encoding",
            None,
        )
    })?;
    let query = Query::parse(request.query.unwrap_or_default())?;
    if query.unsupported_transformer {
        return Err(WireError::new(
            ErrorCode::BadRequest,
            "non-identity transformers are unsupported",
            None,
        ));
    }
    if query.batch {
        return parse_batch(request, &decoded, query.input.as_deref());
    }
    let kind = procedure_kind(&decoded).ok_or_else(|| {
        WireError::new(
            ErrorCode::NotFound,
            format!("No procedure found on path \"{decoded}\""),
            Some(&decoded),
        )
    })?;
    let input = match (request.method, kind) {
        (Method::Get, ProcedureKind::Query | ProcedureKind::Subscription) => {
            if !request.body.is_empty() {
                return Err(WireError::new(
                    ErrorCode::BadRequest,
                    "query request bodies are forbidden",
                    Some(&decoded),
                ));
            }
            parse_optional_json(query.input.as_deref())?
        }
        (Method::Post, ProcedureKind::Mutation) => {
            if query.input.is_some() {
                return Err(WireError::new(
                    ErrorCode::BadRequest,
                    "mutation query input is forbidden",
                    Some(&decoded),
                ));
            }
            if request.content_type != Some("application/json") {
                return Err(WireError::new(
                    ErrorCode::UnsupportedMediaType,
                    "mutation content type must be application/json",
                    Some(&decoded),
                ));
            }
            Some(parse_json(request.body)?)
        }
        _ => {
            return Err(WireError::new(
                ErrorCode::MethodNotSupported,
                format!("Unsupported request method for procedure at path \"{decoded}\""),
                Some(&decoded),
            ));
        }
    };
    let call = Call {
        path: decoded.into_owned(),
        input,
    };
    Ok(match kind {
        ProcedureKind::Query => ParsedRequest::Query(call),
        ProcedureKind::Mutation => ParsedRequest::Mutation(call),
        ProcedureKind::Subscription => ParsedRequest::Subscription(call),
    })
}

fn parse_batch(
    request: &Request<'_>,
    decoded: &str,
    input: Option<&str>,
) -> Result<ParsedRequest, WireError> {
    if request.method != Method::Get {
        return Err(WireError::new(
            ErrorCode::MethodNotSupported,
            "mutation batching is disabled",
            None,
        ));
    }
    let paths: Vec<_> = decoded.split(',').collect();
    if paths.is_empty() || paths.len() > MAX_QUERY_CALLS {
        return Err(WireError::new(
            ErrorCode::BadRequest,
            "query batch exceeds call bound",
            None,
        ));
    }
    if paths
        .iter()
        .any(|path| procedure_kind(path) != Some(ProcedureKind::Query))
    {
        return Err(WireError::new(
            ErrorCode::BadRequest,
            "batches may contain queries only",
            None,
        ));
    }
    let inputs = parse_optional_json(input)?.unwrap_or_else(|| json!({}));
    let map = inputs.as_object().ok_or_else(|| {
        WireError::new(ErrorCode::BadRequest, "batch input must be an object", None)
    })?;
    let calls = paths
        .iter()
        .enumerate()
        .map(|(index, path)| Call {
            path: (*path).to_owned(),
            input: map.get(&index.to_string()).cloned(),
        })
        .collect();
    Ok(ParsedRequest::QueryBatch(calls))
}

fn parse_optional_json(input: Option<&str>) -> Result<Option<Value>, WireError> {
    input.map(|value| parse_json(value.as_bytes())).transpose()
}
fn parse_json(bytes: &[u8]) -> Result<Value, WireError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| WireError::new(ErrorCode::BadRequest, "invalid JSON input", None))?;
    if json_depth(&value) > MAX_JSON_DEPTH {
        return Err(WireError::new(
            ErrorCode::BadRequest,
            "JSON input exceeds depth bound",
            None,
        ));
    }
    Ok(value)
}
fn json_depth(value: &Value) -> usize {
    match value {
        Value::Array(v) => 1 + v.iter().map(json_depth).max().unwrap_or(0),
        Value::Object(v) => 1 + v.values().map(json_depth).max().unwrap_or(0),
        _ => 1,
    }
}

struct Query {
    batch: bool,
    input: Option<String>,
    unsupported_transformer: bool,
}
impl Query {
    fn parse(raw: &str) -> Result<Self, WireError> {
        let mut result = Self {
            batch: false,
            input: None,
            unsupported_transformer: false,
        };
        for pair in raw
            .trim_start_matches('?')
            .split('&')
            .filter(|p| !p.is_empty())
        {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            validate_percent_encoding(value)?;
            let decoded = percent_decode_str(value).decode_utf8().map_err(|_| {
                WireError::new(ErrorCode::BadRequest, "malformed query encoding", None)
            })?;
            match key {
                "batch" => result.batch = decoded == "1",
                "input" => result.input = Some(decoded.into_owned()),
                "transformer" => result.unsupported_transformer = true,
                _ => {
                    return Err(WireError::new(
                        ErrorCode::BadRequest,
                        "unsupported tRPC query extension",
                        None,
                    ));
                }
            }
        }
        Ok(result)
    }
}

fn validate_percent_encoding(value: &str) -> Result<(), WireError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && (index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit())
        {
            return Err(WireError::new(
                ErrorCode::BadRequest,
                "malformed percent encoding",
                None,
            ));
        }
        index += if bytes[index] == b'%' { 3 } else { 1 };
    }
    Ok(())
}

#[must_use]
pub fn success<T: Serialize>(status: u16, data: &T) -> Response {
    json_response(status, &json!({"result":{"data":data}}))
}

#[must_use]
pub fn batch_success<T: Serialize>(items: &[T]) -> Response {
    let body: Vec<_> = items
        .iter()
        .map(|data| json!({"result":{"data":data}}))
        .collect();
    json_response(200, &body)
}

#[must_use]
pub fn error(error: &WireError) -> Response {
    json_response(
        error.code.status(),
        &json!({"error":{"message":error.message,"code":error.code.numeric(),"data":{"code":error.code.name(),"httpStatus":error.code.status(),"path":error.path}}}),
    )
}

#[must_use]
pub fn procedure_error(
    code: ErrorCode,
    bunting_code: BuntingErrorCode,
    message: &str,
    path: &str,
) -> Response {
    procedure_error_with_data::<Value>(code, bunting_code, message, path, None)
}

#[must_use]
pub fn procedure_error_with_data<T: Serialize>(
    code: ErrorCode,
    bunting_code: BuntingErrorCode,
    message: &str,
    path: &str,
    bunting_data: Option<&T>,
) -> Response {
    json_response(
        code.status(),
        &json!({"error":{"message":message,"code":code.numeric(),"data":{"code":code.name(),"httpStatus":code.status(),"path":path,"buntingCode":bunting_code.name(),"buntingData":bunting_data}}}),
    )
}

#[must_use]
pub fn batch_responses(items: &[Response]) -> Response {
    let values: Vec<Value> = items
        .iter()
        .map(|item| serde_json::from_slice(&item.body).unwrap_or_else(|_| json!({})))
        .collect();
    let status = items
        .first()
        .filter(|first| items.iter().all(|item| item.status == first.status))
        .map_or(207, |item| item.status);
    json_response(status, &values)
}

fn json_response<T: Serialize>(status: u16, value: &T) -> Response {
    Response {
        status,
        content_type: "application/json",
        vary: "trpc-accept, accept",
        body: serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        method: Method,
        path: &'a str,
        query: Option<&'a str>,
        content_type: Option<&'a str>,
        body: &'a [u8],
    ) -> Request<'a> {
        Request {
            method,
            path,
            query,
            content_type,
            body,
        }
    }

    #[test]
    fn parses_single_query_and_mutation() {
        assert!(matches!(
            parse(&request(
                Method::Get,
                "/trpc/system.health",
                None,
                None,
                b""
            )),
            Ok(ParsedRequest::Query(_))
        ));
        assert!(matches!(
            parse(&request(
                Method::Post,
                "/trpc/orders.submit",
                None,
                Some("application/json"),
                br#"{"orderId":"1"}"#
            )),
            Ok(ParsedRequest::Mutation(_))
        ));
    }

    #[test]
    fn bounded_query_batch_preserves_order_and_sparse_inputs() {
        let parsed = parse(&request(
            Method::Get,
            "/trpc/system.health,market.snapshot",
            Some("?batch=1&input=%7B%221%22%3A%7B%22runId%22%3A%227%22%7D%7D"),
            None,
            b"",
        ));
        assert!(matches!(&parsed, Ok(ParsedRequest::QueryBatch(_))));
        if let Ok(ParsedRequest::QueryBatch(calls)) = parsed {
            assert_eq!(calls[0].input, None);
            assert_eq!(calls[1].input, Some(json!({"runId":"7"})));
        }
    }

    #[test]
    fn rejects_mutation_batches_unknowns_extensions_and_bounds() {
        assert_eq!(
            parse(&request(
                Method::Post,
                "/trpc/orders.submit,orders.cancel",
                Some("?batch=1"),
                Some("application/json"),
                b"{}"
            ))
            .err()
            .map(|error| error.code),
            Some(ErrorCode::MethodNotSupported)
        );
        assert_eq!(
            parse(&request(
                Method::Get,
                "/trpc/missing.procedure",
                None,
                None,
                b""
            ))
            .err()
            .map(|error| error.code),
            Some(ErrorCode::NotFound)
        );
        assert_eq!(
            parse(&request(
                Method::Get,
                "/trpc/system.health",
                Some("?transformer=superjson"),
                None,
                b""
            ))
            .err()
            .map(|error| error.code),
            Some(ErrorCode::BadRequest)
        );
        assert_eq!(
            parse(&request(
                Method::Get,
                "/trpc/system.health",
                None,
                None,
                b"{}"
            ))
            .err()
            .map(|error| error.code),
            Some(ErrorCode::BadRequest)
        );
        assert_eq!(
            parse(&request(
                Method::Post,
                "/trpc/orders.cancel",
                Some("?input=%7B%7D"),
                Some("application/json"),
                b"{}"
            ))
            .err()
            .map(|error| error.code),
            Some(ErrorCode::BadRequest)
        );
        let paths = vec!["system.health"; MAX_QUERY_CALLS + 1].join(",");
        assert_eq!(
            parse(&request(
                Method::Get,
                &format!("/trpc/{paths}"),
                Some("?batch=1"),
                None,
                b""
            ))
            .err()
            .map(|error| error.code),
            Some(ErrorCode::BadRequest)
        );
    }

    fn fixture(name: &str) -> Result<Value, serde_json::Error> {
        let source = match name {
            "single_query" => {
                include_str!("../../../tests/fixtures/reference/trpc/11.18.0/single_query.json")
            }
            "single_mutation" => {
                include_str!("../../../tests/fixtures/reference/trpc/11.18.0/single_mutation.json")
            }
            "bounded_query_batch" => include_str!(
                "../../../tests/fixtures/reference/trpc/11.18.0/bounded_query_batch.json"
            ),
            "unknown_procedure" => include_str!(
                "../../../tests/fixtures/reference/trpc/11.18.0/unknown_procedure.json"
            ),
            _ => "null",
        };
        serde_json::from_str(source)
    }

    fn assert_fixture_response(
        response: &Response,
        fixture: &Value,
    ) -> Result<(), serde_json::Error> {
        assert_eq!(response.status, fixture["response"]["status"]);
        assert_eq!(
            response.content_type,
            fixture["response"]["headers"]["content-type"]
        );
        assert_eq!(response.vary, fixture["response"]["headers"]["vary"]);
        let body: Value = serde_json::from_slice(&response.body)?;
        assert_eq!(body, fixture["response"]["body"]);
        Ok(())
    }

    #[test]
    fn response_envelopes_match_official_trpc_fixtures() -> Result<(), serde_json::Error> {
        assert_fixture_response(
            &success(
                200,
                &json!({"apiVersion":"bunting.v1","contractCompatible":true}),
            ),
            &fixture("single_query")?,
        )?;
        assert_fixture_response(
            &success(
                200,
                &json!({"accepted":true,"input":{"orderId":"9007199254740993"}}),
            ),
            &fixture("single_mutation")?,
        )?;
        assert_fixture_response(
            &batch_success(&[
                json!({"apiVersion":"bunting.v1","contractCompatible":true}),
                json!({"instrumentId":"7"}),
            ]),
            &fixture("bounded_query_batch")?,
        )?;
        assert_fixture_response(
            &error(&WireError::new(
                ErrorCode::NotFound,
                "No procedure found on path \"missing.procedure\"",
                Some("missing.procedure"),
            )),
            &fixture("unknown_procedure")?,
        )?;
        Ok(())
    }
}
