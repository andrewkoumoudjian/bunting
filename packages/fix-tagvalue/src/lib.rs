#![forbid(unsafe_code)]
//! Bounded FIX tag-value framing with length and checksum validation.

use core::fmt;
pub const SOH: u8 = 1;
pub const MAX_MESSAGE_BYTES: usize = 65_536;
pub const MAX_FIELDS: usize = 256;

/// One owned FIX field.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Field { pub tag: u32, pub value: Vec<u8> }

/// One validated FIX message.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message { fields: Vec<Field> }
impl Message {
    /// Returns the first value for `tag`.
    #[must_use]
    pub fn get(&self, tag: u32) -> Option<&[u8]> { self.fields.iter().find(|field| field.tag == tag).map(|field| field.value.as_slice()) }
    /// Iterates through validated fields in wire order.
    pub fn fields(&self) -> impl Iterator<Item = &Field> { self.fields.iter() }
}

/// Reports a framing or protocol-bound violation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error { Incomplete, TooLarge, TooManyFields, MalformedField, InvalidBodyLength, InvalidChecksum, InvalidOrdering }
impl fmt::Display for Error { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "{self:?}") } }
impl std::error::Error for Error {}

/// Parses one complete FIX message and returns its consumed byte count.
///
/// # Errors
/// Returns [`Error`] when framing is incomplete, malformed, or violates bounds.
pub fn parse(input: &[u8]) -> Result<(Message, usize), Error> {
    if input.len() > MAX_MESSAGE_BYTES { return Err(Error::TooLarge); }
    let checksum_start = input.windows(4).position(|window| window == [SOH, b'1', b'0', b'=']).map(|index| index + 1).ok_or(Error::Incomplete)?;
    let end = input[checksum_start..].iter().position(|byte| *byte == SOH).map(|index| checksum_start + index + 1).ok_or(Error::Incomplete)?;
    let frame = &input[..end];
    let fields = parse_fields(frame)?;
    if fields.first().map(|f| f.tag) != Some(8) || fields.get(1).map(|f| f.tag) != Some(9) || fields.last().map(|f| f.tag) != Some(10) { return Err(Error::InvalidOrdering); }
    let body_length: usize = core::str::from_utf8(&fields[1].value).map_err(|_| Error::InvalidBodyLength)?.parse().map_err(|_| Error::InvalidBodyLength)?;
    let body_start = nth_delimiter(frame, 2).ok_or(Error::Incomplete)? + 1;
    if checksum_start.saturating_sub(body_start) != body_length { return Err(Error::InvalidBodyLength); }
    let expected: u8 = core::str::from_utf8(&fields.last().ok_or(Error::Incomplete)?.value).map_err(|_| Error::InvalidChecksum)?.parse().map_err(|_| Error::InvalidChecksum)?;
    let actual = frame[..checksum_start].iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    if expected != actual { return Err(Error::InvalidChecksum); }
    Ok((Message { fields }, end))
}

/// Encodes ordered body fields as one FIX message.
///
/// # Errors
/// Returns [`Error`] when a value contains a delimiter or bounds are exceeded.
pub fn encode(begin_string: &[u8], body_fields: &[Field]) -> Result<Vec<u8>, Error> {
    if body_fields.len() + 3 > MAX_FIELDS || begin_string.contains(&SOH) { return Err(Error::TooManyFields); }
    let mut body = Vec::new();
    for field in body_fields { if field.value.contains(&SOH) || matches!(field.tag, 8 | 9 | 10) { return Err(Error::MalformedField); } push_field(&mut body, field.tag, &field.value); }
    let mut output = Vec::new(); push_field(&mut output, 8, begin_string); push_field(&mut output, 9, body.len().to_string().as_bytes()); output.extend_from_slice(&body);
    let checksum = output.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    push_field(&mut output, 10, format!("{checksum:03}").as_bytes());
    if output.len() > MAX_MESSAGE_BYTES { return Err(Error::TooLarge); }
    Ok(output)
}
fn push_field(output: &mut Vec<u8>, tag: u32, value: &[u8]) { output.extend_from_slice(tag.to_string().as_bytes()); output.push(b'='); output.extend_from_slice(value); output.push(SOH); }
fn nth_delimiter(input: &[u8], count: usize) -> Option<usize> { input.iter().enumerate().filter(|(_, b)| **b == SOH).nth(count - 1).map(|(i, _)| i) }
fn parse_fields(input: &[u8]) -> Result<Vec<Field>, Error> { let mut fields = Vec::new(); for raw in input.split(|b| *b == SOH).filter(|f| !f.is_empty()) { if fields.len() == MAX_FIELDS { return Err(Error::TooManyFields); } let eq = raw.iter().position(|b| *b == b'=').ok_or(Error::MalformedField)?; let tag = core::str::from_utf8(&raw[..eq]).map_err(|_| Error::MalformedField)?.parse().map_err(|_| Error::MalformedField)?; fields.push(Field { tag, value: raw[eq + 1..].to_vec() }); } Ok(fields) }

// Rust guideline compliant 2026-02-21

#[cfg(test)] mod tests { use super::*; #[test] fn round_trip_and_partial_reads() -> Result<(), Error> { let encoded = encode(b"FIX.4.4", &[Field { tag: 35, value: b"D".to_vec() }, Field { tag: 34, value: b"1".to_vec() }])?; for split in 0..encoded.len() { assert_eq!(parse(&encoded[..split]), Err(Error::Incomplete)); } let (message, used) = parse(&encoded)?; assert_eq!(used, encoded.len()); assert_eq!(message.get(35), Some(b"D".as_slice())); Ok(()) } #[test] fn checksum_corruption_rejects() -> Result<(), Error> { let mut encoded = encode(b"FIX.4.4", &[Field { tag: 35, value: b"0".to_vec() }])?; encoded[2] ^= 1; assert!(matches!(parse(&encoded), Err(Error::InvalidChecksum))); Ok(()) } }
