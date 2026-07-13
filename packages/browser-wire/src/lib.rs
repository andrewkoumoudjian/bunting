#![forbid(unsafe_code)]
//! Bounded browser-facing JSON fetch protocol.

use bunting_api_contract::{BuntingErrorCode, ProcedureKind, procedure_kind};
use percent_encoding::percent_decode_str;
use serde::Serialize;
use serde_json::{Value, json};

pub const MAX_REQUEST_BYTES: usize = 16_384;
pub const MAX_PROCEDURE_PATH_BYTES: usize = 2_048;

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

/// Parses one bounded browser request.
///
/// # Errors
/// Returns a structured error for malformed, unsupported, or oversized requests.
pub fn parse(request: &Request<'_>) -> Result<ParsedRequest, WireError> {
    if request.path.len() > MAX_PROCEDURE_PATH_BYTES || request.body.len() > MAX_REQUEST_BYTES {
        return Err(WireError::new(
            ErrorCode::PayloadTooLarge,
            "request exceeds bounds",
            None,
        ));
    }
    let encoded = request
        .path
        .strip_prefix("/api/")
        .ok_or_else(|| WireError::new(ErrorCode::NotFound, "procedure not found", None))?;
    let decoded = percent_decode_str(encoded)
        .decode_utf8()
        .map_err(|_| WireError::new(ErrorCode::BadRequest, "invalid path", None))?;
    let kind = procedure_kind(&decoded).ok_or_else(|| {
        WireError::new(ErrorCode::NotFound, "procedure not found", Some(&decoded))
    })?;
    let input = match (request.method, kind) {
        (Method::Get, ProcedureKind::Query) => parse_query(request.query)?,
        (Method::Post, ProcedureKind::Mutation) => {
            if request.content_type != Some("application/json") {
                return Err(WireError::new(
                    ErrorCode::UnsupportedMediaType,
                    "application/json required",
                    Some(&decoded),
                ));
            }
            Some(serde_json::from_slice(request.body).map_err(|_| {
                WireError::new(ErrorCode::BadRequest, "invalid JSON", Some(&decoded))
            })?)
        }
        _ => {
            return Err(WireError::new(
                ErrorCode::MethodNotSupported,
                "method mismatch",
                Some(&decoded),
            ));
        }
    };
    let call = Call {
        path: decoded.into_owned(),
        input,
    };
    Ok(if kind == ProcedureKind::Query {
        ParsedRequest::Query(call)
    } else {
        ParsedRequest::Mutation(call)
    })
}

fn parse_query(query: Option<&str>) -> Result<Option<Value>, WireError> {
    let Some(query) = query else {
        return Ok(None);
    };
    let Some(value) = query.strip_prefix("input=") else {
        return Err(WireError::new(
            ErrorCode::BadRequest,
            "only input is supported",
            None,
        ));
    };
    let decoded = percent_decode_str(value)
        .decode_utf8()
        .map_err(|_| WireError::new(ErrorCode::BadRequest, "invalid query", None))?;
    Ok(Some(serde_json::from_str(&decoded).map_err(|_| {
        WireError::new(ErrorCode::BadRequest, "invalid query JSON", None)
    })?))
}

#[must_use]
pub fn success<T: Serialize>(status: u16, value: &T) -> Response {
    json_response(status, &json!({"data": value}))
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
    data: Option<&T>,
) -> Response {
    json_response(
        code.status(),
        &json!({"error":{"code":code.name(),"buntingCode":bunting_code,"message":message,"path":path,"data":data}}),
    )
}
#[must_use]
pub fn error(value: &WireError) -> Response {
    json_response(
        value.code.status(),
        &json!({"error":{"code":value.code.name(),"message":value.message,"path":value.path}}),
    )
}
fn json_response<T: Serialize>(status: u16, value: &T) -> Response {
    Response {
        status,
        content_type: "application/json",
        vary: "authorization",
        body: serde_json::to_vec(value).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_paths_are_distinct_from_legacy_rpc() -> Result<(), WireError> {
        assert!(matches!(
            parse(&Request {
                method: Method::Get,
                path: "/api/system.health",
                query: None,
                content_type: None,
                body: &[]
            })?,
            ParsedRequest::Query(_)
        ));
        assert!(
            parse(&Request {
                method: Method::Get,
                path: "/trpc/system.health",
                query: None,
                content_type: None,
                body: &[]
            })
            .is_err()
        );
        Ok(())
    }
}
