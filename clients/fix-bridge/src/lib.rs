#![forbid(unsafe_code)]
//! Native participant-side FIX-to-tRPC application mapping.

use bunting_api_contract::{CancelOrderInput, CommandOutput, SequenceDecimalString, Side, SignedDecimalString, SubmitOrderInput, UnsignedDecimalString};
use bunting_fix_session::{Session, StateStore};
use bunting_fix_tagvalue::{Field, MAX_MESSAGE_BYTES, Message, encode, parse};
use bunting_trpc_client::{Client, ClientError, Transport};
use core::{fmt, str::FromStr};

/// Reports framing, session, mapping, and upstream failures.
#[derive(Debug)]
pub enum BridgeError { Frame(bunting_fix_tagvalue::Error), Session(bunting_fix_session::Error), Missing(u32), Invalid(u32), Unsupported(Vec<u8>), Upstream(ClientError), BufferFull }
impl fmt::Display for BridgeError { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") } }
impl std::error::Error for BridgeError {}

/// Buffers bounded partial TCP reads until complete FIX frames arrive.
#[derive(Default)] pub struct FrameBuffer { bytes: Vec<u8> }
impl FrameBuffer {
    /// Appends a TCP fragment and extracts every complete validated message.
    ///
    /// # Errors
    /// Returns a framing error or [`BridgeError::BufferFull`] at the hard bound.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Message>, BridgeError> { if self.bytes.len().saturating_add(bytes.len()) > MAX_MESSAGE_BYTES { return Err(BridgeError::BufferFull); } self.bytes.extend_from_slice(bytes); let mut messages = Vec::new(); loop { match parse(&self.bytes) { Ok((message, used)) => { self.bytes.drain(..used); messages.push(message); }, Err(bunting_fix_tagvalue::Error::Incomplete) => break, Err(error) => return Err(BridgeError::Frame(error)) } } Ok(messages) }
}

/// Maps authenticated local FIX application messages to typed tRPC calls.
pub struct Bridge<T, S> { client: Client<T>, session: Session<S> }
impl<T: Transport, S: StateStore> Bridge<T, S> {
    #[must_use] pub const fn new(client: Client<T>, session: Session<S>) -> Self { Self { client, session } }
    /// Processes one validated FIX message and returns a FIX response frame.
    ///
    /// # Errors
    /// Returns exact session, mapping, or upstream failures without rewriting fields.
    pub fn handle(&mut self, message: &Message) -> Result<Vec<u8>, BridgeError> {
        let received = number::<u32>(message, 34)?; let possible_duplicate = message.get(43) == Some(b"Y"); self.session.accept_inbound(received, possible_duplicate).map_err(BridgeError::Session)?;
        match required(message, 35)? { b"A" => { self.session.logon().map_err(BridgeError::Session)?; self.admin(b"A", Vec::new()) }, b"0" => self.admin(b"0", Vec::new()), b"2" => { let begin = number::<u32>(message, 7)?; let end = number::<u32>(message, 16)?; self.admin(b"4", vec![field(123, b"Y"), field(36, &(end.max(begin) + 1).to_string().into_bytes())]) }, b"D" => { let input = submit_input(message)?; let output = self.client.submit(&input).map_err(BridgeError::Upstream)?; self.execution_report(message, &output) }, b"F" => { let input = cancel_input(message)?; let output = self.client.cancel(&input).map_err(BridgeError::Upstream)?; self.execution_report(message, &output) }, other => Err(BridgeError::Unsupported(other.to_vec())) }
    }
    fn admin(&mut self, msg_type: &[u8], mut fields: Vec<Field>) -> Result<Vec<u8>, BridgeError> { fields.insert(0, field(35, msg_type)); let sequence = self.session.next_outbound().map_err(BridgeError::Session)?; fields.insert(1, field(34, sequence.to_string().as_bytes())); encode(b"FIX.4.4", &fields).map_err(BridgeError::Frame) }
    fn execution_report(&mut self, request: &Message, output: &CommandOutput) -> Result<Vec<u8>, BridgeError> { let mut fields = vec![field(35, b"8"), field(34, self.session.next_outbound().map_err(BridgeError::Session)?.to_string().as_bytes()), field(11, required(request, 11)?), field(37, output.order_id.as_ref().map(ToString::to_string).unwrap_or_default().as_bytes())]; if output.accepted { fields.extend([field(39, b"0"), field(150, b"0")]); } else { fields.extend([field(39, b"8"), field(150, b"8"), field(58, output.reject_code.as_deref().unwrap_or("REJECTED").as_bytes())]); } encode(b"FIX.4.4", &fields).map_err(BridgeError::Frame) }
}

fn submit_input(message: &Message) -> Result<SubmitOrderInput, BridgeError> { Ok(SubmitOrderInput { run_id: decimal(message, 9001)?, instrument_id: decimal(message, 48)?, command_id: decimal(message, 9717)?, correlation_id: decimal(message, 11)?, expected_sequence: sequence(message, 9002)?, logical_time_ns: sequence(message, 9003)?, order_id: decimal(message, 37)?, side: match required(message, 54)? { b"1" => Side::Buy, b"2" => Side::Sell, _ => return Err(BridgeError::Invalid(54)) }, price_ticks: signed(message, 44)?, quantity_lots: signed(message, 38)? }) }
fn cancel_input(message: &Message) -> Result<CancelOrderInput, BridgeError> { Ok(CancelOrderInput { run_id: decimal(message, 9001)?, instrument_id: decimal(message, 48)?, command_id: decimal(message, 9717)?, correlation_id: decimal(message, 11)?, expected_sequence: sequence(message, 9002)?, logical_time_ns: sequence(message, 9003)?, order_id: decimal(message, 41)? }) }
fn required(message: &Message, tag: u32) -> Result<&[u8], BridgeError> { message.get(tag).ok_or(BridgeError::Missing(tag)) }
fn number<T: FromStr>(message: &Message, tag: u32) -> Result<T, BridgeError> { core::str::from_utf8(required(message, tag)?).map_err(|_| BridgeError::Invalid(tag))?.parse().map_err(|_| BridgeError::Invalid(tag)) }
fn decimal(message: &Message, tag: u32) -> Result<UnsignedDecimalString, BridgeError> { core::str::from_utf8(required(message, tag)?).map_err(|_| BridgeError::Invalid(tag))?.parse().map_err(|_| BridgeError::Invalid(tag)) }
fn sequence(message: &Message, tag: u32) -> Result<SequenceDecimalString, BridgeError> { core::str::from_utf8(required(message, tag)?).map_err(|_| BridgeError::Invalid(tag))?.parse().map_err(|_| BridgeError::Invalid(tag)) }
fn signed(message: &Message, tag: u32) -> Result<SignedDecimalString, BridgeError> { core::str::from_utf8(required(message, tag)?).map_err(|_| BridgeError::Invalid(tag))?.parse().map_err(|_| BridgeError::Invalid(tag)) }
fn field(tag: u32, value: &[u8]) -> Field { Field { tag, value: value.to_vec() } }

// Rust guideline compliant 2026-02-21

#[cfg(test)] mod tests { use super::*; use bunting_fix_session::MemoryStore; use bunting_trpc_client::{HttpRequest, HttpResponse, RetryPolicy}; struct Fake; impl Transport for Fake { type Error = &'static str; fn send(&mut self, request: &HttpRequest) -> Result<HttpResponse, Self::Error> { assert_eq!(request.path_and_query, "/trpc/orders.submit"); Ok(HttpResponse { status: 200, body: br#"{"result":{"data":{"accepted":true,"rejectCode":null,"committedSequence":"9","orderId":"77","snapshotChecksum":null}}}"#.to_vec() }) } } #[test] fn new_order_maps_through_typed_client() -> Result<(), Box<dyn std::error::Error>> { let frame = encode(b"FIX.4.4", &[field(35,b"D"),field(34,b"1"),field(11,b"101"),field(48,b"2"),field(54,b"1"),field(38,b"5"),field(44,b"10"),field(37,b"77"),field(9001,b"1"),field(9002,b"0"),field(9003,b"12"),field(9717,b"100")])?; let (message, _) = parse(&frame)?; let session = Session::restore(MemoryStore::default())?; let mut bridge = Bridge::new(Client::new(Fake, RetryPolicy::bounded(1)), session); let response = bridge.handle(&message)?; let (report, _) = parse(&response)?; assert_eq!(report.get(35), Some(b"8".as_slice())); assert_eq!(report.get(39), Some(b"0".as_slice())); Ok(()) } }
