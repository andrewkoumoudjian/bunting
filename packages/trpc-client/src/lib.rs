#![forbid(unsafe_code)]
//! Typed, bounded requests for Bunting's native tRPC endpoint.

use bunting_api_contract::{CancelOrderInput, CommandOutput, HealthOutput, MarketSnapshotInput, MarketSnapshotOutput, SubmitOrderInput};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::fmt;

pub const MAX_RESPONSE_BYTES: usize = 65_536;
pub const MAX_ATTEMPTS: u8 = 3;

/// A complete transport-neutral HTTP request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpRequest {
    pub method: &'static str,
    pub path_and_query: String,
    pub content_type: Option<&'static str>,
    pub body: Vec<u8>,
}

/// A transport-neutral HTTP response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HttpResponse { pub status: u16, pub body: Vec<u8> }

/// Sends one request without exposing a specific HTTP runtime.
pub trait Transport {
    type Error: fmt::Display;
    fn send(&mut self, request: &HttpRequest) -> Result<HttpResponse, Self::Error>;
}

/// Reports bounded client or upstream failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientError { Encode, Transport(String), ResponseTooLarge, InvalidEnvelope, Upstream { status: u16, code: Option<String>, message: String } }
impl fmt::Display for ClientError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") }
}
impl std::error::Error for ClientError {}

/// Controls bounded retries for transport and transient upstream failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy { pub max_attempts: u8 }
impl Default for RetryPolicy { fn default() -> Self { Self { max_attempts: MAX_ATTEMPTS } } }
impl RetryPolicy {
    #[must_use]
    pub const fn bounded(max_attempts: u8) -> Self { Self { max_attempts: if max_attempts > MAX_ATTEMPTS { MAX_ATTEMPTS } else { max_attempts } } }
}

/// Calls the implemented Bunting procedures through an injected transport.
pub struct Client<T> { transport: T, retry: RetryPolicy }
impl<T: Transport> Client<T> {
    #[must_use]
    pub const fn new(transport: T, retry: RetryPolicy) -> Self { Self { transport, retry } }

    /// Queries service health.
    ///
    /// # Errors
    /// Returns [`ClientError`] for transport, bounds, envelope, or upstream failures.
    pub fn health(&mut self) -> Result<HealthOutput, ClientError> { self.query("system.health", &()) }

    /// Queries one committed market snapshot.
    ///
    /// # Errors
    /// Returns [`ClientError`] for transport, bounds, envelope, or upstream failures.
    pub fn market_snapshot(&mut self, input: &MarketSnapshotInput) -> Result<MarketSnapshotOutput, ClientError> { self.query("market.snapshot", input) }

    /// Submits one idempotently identified order command.
    ///
    /// # Errors
    /// Returns [`ClientError`] for transport, bounds, envelope, or upstream failures.
    pub fn submit(&mut self, input: &SubmitOrderInput) -> Result<CommandOutput, ClientError> { self.mutation("orders.submit", input) }

    /// Cancels one idempotently identified order command.
    ///
    /// # Errors
    /// Returns [`ClientError`] for transport, bounds, envelope, or upstream failures.
    pub fn cancel(&mut self, input: &CancelOrderInput) -> Result<CommandOutput, ClientError> { self.mutation("orders.cancel", input) }

    fn query<I: Serialize, O: DeserializeOwned>(&mut self, path: &str, input: &I) -> Result<O, ClientError> {
        let json = serde_json::to_string(input).map_err(|_| ClientError::Encode)?;
        let request = HttpRequest { method: "GET", path_and_query: format!("/trpc/{path}?input={}", utf8_percent_encode(&json, NON_ALPHANUMERIC)), content_type: None, body: Vec::new() };
        self.execute(&request, true)
    }

    fn mutation<I: Serialize, O: DeserializeOwned>(&mut self, path: &str, input: &I) -> Result<O, ClientError> {
        let body = serde_json::to_vec(input).map_err(|_| ClientError::Encode)?;
        let request = HttpRequest { method: "POST", path_and_query: format!("/trpc/{path}"), content_type: Some("application/json"), body };
        // Bunting command IDs make replay safe; the Worker returns the durable duplicate outcome.
        self.execute(&request, true)
    }

    fn execute<O: DeserializeOwned>(&mut self, request: &HttpRequest, idempotent: bool) -> Result<O, ClientError> {
        let attempts = self.retry.max_attempts.max(1);
        for attempt in 1..=attempts {
            match self.transport.send(request) {
                Ok(response) if response.body.len() > MAX_RESPONSE_BYTES => return Err(ClientError::ResponseTooLarge),
                Ok(response) if response.status < 500 || !idempotent || attempt == attempts => return decode(response),
                Ok(_) => {}
                Err(error) if !idempotent || attempt == attempts => return Err(ClientError::Transport(error.to_string())),
                Err(_) => {}
            }
        }
        Err(ClientError::InvalidEnvelope)
    }
}

fn decode<O: DeserializeOwned>(response: HttpResponse) -> Result<O, ClientError> {
    let value: Value = serde_json::from_slice(&response.body).map_err(|_| ClientError::InvalidEnvelope)?;
    if let Some(data) = value.get("result").and_then(|v| v.get("data")) {
        return serde_json::from_value(data.clone()).map_err(|_| ClientError::InvalidEnvelope);
    }
    let error = value.get("error").ok_or(ClientError::InvalidEnvelope)?;
    Err(ClientError::Upstream { status: response.status, code: error.get("data").and_then(|v| v.get("code")).and_then(Value::as_str).map(str::to_owned), message: error.get("message").and_then(Value::as_str).unwrap_or("upstream error").to_owned() })
}

// Rust guideline compliant 2026-02-21

#[cfg(test)]
mod tests {
    use super::*;
    struct Fake { calls: usize }
    impl Transport for Fake { type Error = &'static str; fn send(&mut self, request: &HttpRequest) -> Result<HttpResponse, Self::Error> { self.calls += 1; if self.calls == 1 { return Err("offline"); } assert_eq!(request.method, "GET"); Ok(HttpResponse { status: 200, body: br#"{"result":{"data":{"apiVersion":"bunting.v1","serviceVersion":"test","orderbookVersion":"0.10.3","contractCompatible":true}}}"#.to_vec() }) } }
    #[test]
    fn query_retries_within_bound() -> Result<(), ClientError> { let mut client = Client::new(Fake { calls: 0 }, RetryPolicy::default()); assert!(client.health()?.contract_compatible); assert_eq!(client.transport.calls, 2); Ok(()) }
    #[test]
    fn policy_caps_attempts() { assert_eq!(RetryPolicy::bounded(200).max_attempts, MAX_ATTEMPTS); }
}
