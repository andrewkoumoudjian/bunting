#![forbid(unsafe_code)]
//! Bounded, transport-neutral FIX tag-value framing.

use rustyfix_dictionary::{Dictionary, FixDatatype};
use serde::{Deserialize, Serialize};

pub const SOH: u8 = 0x01;
/// FIX Transport session version used by the competition profile.
pub const FIXT_11: &str = "FIXT.1.1";
/// `ApplVerID` value assigned to FIX 5.0 SP2.
pub const FIX_50_SP2_APPL_VER_ID: &str = "9";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Field {
    pub tag: u32,
    pub value: String,
}

impl Field {
    #[must_use]
    pub fn new(tag: u32, value: impl Into<String>) -> Self {
        Self {
            tag,
            value: value.into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FixMessage {
    pub begin_string: String,
    pub msg_type: String,
    pub fields: Vec<Field>,
}

impl FixMessage {
    #[must_use]
    pub fn new(msg_type: impl Into<String>) -> Self {
        Self {
            begin_string: FIXT_11.to_owned(),
            msg_type: msg_type.into(),
            fields: Vec::new(),
        }
    }

    #[must_use]
    pub fn value(&self, tag: u32) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.tag == tag)
            .map(|field| field.value.as_str())
    }

    pub fn push(&mut self, tag: u32, value: impl Into<String>) {
        self.fields.push(Field::new(tag, value));
    }

    /// Serializes a message with exact `BodyLength` and `CheckSum` fields.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid field values or configured bounds.
    pub fn encode(&self, limits: &WireLimits) -> Result<Vec<u8>, WireError> {
        if self.fields.len() > limits.max_fields {
            return Err(WireError::TooManyFields);
        }
        validate_value(&self.begin_string, limits)?;
        validate_value(&self.msg_type, limits)?;
        let mut body = Vec::new();
        append_field(&mut body, 35, &self.msg_type, limits)?;
        for field in &self.fields {
            if matches!(field.tag, 8 | 9 | 10 | 35) {
                return Err(WireError::ReservedTag(field.tag));
            }
            append_field(&mut body, field.tag, &field.value, limits)?;
        }
        let mut frame = Vec::new();
        append_field(&mut frame, 8, &self.begin_string, limits)?;
        append_field(&mut frame, 9, &body.len().to_string(), limits)?;
        frame.extend_from_slice(&body);
        let checksum = frame.iter().fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
        append_field(&mut frame, 10, &format!("{checksum:03}"), limits)?;
        if frame.len() > limits.max_message_bytes {
            return Err(WireError::MessageTooLarge);
        }
        Ok(frame)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WireLimits {
    pub max_message_bytes: usize,
    pub max_field_bytes: usize,
    pub max_fields: usize,
    pub max_buffer_bytes: usize,
}

impl Default for WireLimits {
    fn default() -> Self {
        Self {
            max_message_bytes: 65_536,
            max_field_bytes: 8_192,
            max_fields: 256,
            max_buffer_bytes: 131_072,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireError {
    BufferFull,
    MessageTooLarge,
    FieldTooLarge,
    TooManyFields,
    InvalidUtf8,
    InvalidField,
    InvalidBeginString,
    InvalidBodyLength,
    InvalidChecksum,
    MissingMsgType,
    MissingRequiredTag(u32),
    DuplicateTag(u32),
    InvalidMessageType,
    InvalidRepeatingGroup(u32),
    ReservedTag(u32),
    DictionaryUnavailable,
    UnknownTag(u32),
}

#[derive(Debug)]
pub struct Decoder {
    limits: WireLimits,
    buffer: Vec<u8>,
    dictionary: CompetitionDictionary,
}

impl Decoder {
    /// Creates a bounded decoder using the standard FIXT.1.1 and FIX 5.0 SP2 dictionaries.
    ///
    /// # Errors
    /// Returns an error if the released, embedded dictionaries cannot be loaded.
    pub fn try_new(limits: WireLimits) -> Result<Self, WireError> {
        Ok(Self {
            limits,
            buffer: Vec::new(),
            dictionary: CompetitionDictionary::load()?,
        })
    }

    #[must_use]
    pub fn retained_bytes(&self) -> usize {
        self.buffer.len()
    }

    /// Retains incomplete trailing bytes and returns every complete frame.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed frames or when the bounded receive buffer is exceeded.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<FixMessage>, WireError> {
        if self.buffer.len().saturating_add(bytes.len()) > self.limits.max_buffer_bytes {
            return Err(WireError::BufferFull);
        }
        self.buffer.extend_from_slice(bytes);
        let mut messages = Vec::new();
        loop {
            let Some(frame_len) = complete_frame_len(&self.buffer, &self.limits)? else {
                break;
            };
            let frame: Vec<u8> = self.buffer.drain(..frame_len).collect();
            let message = parse_frame(&frame, &self.limits)?;
            self.dictionary.validate(&message)?;
            messages.push(message);
        }
        Ok(messages)
    }
}

fn validate_value(value: &str, limits: &WireLimits) -> Result<(), WireError> {
    if value.as_bytes().contains(&SOH) || value.contains('=') {
        return Err(WireError::InvalidField);
    }
    if value.len() > limits.max_field_bytes {
        return Err(WireError::FieldTooLarge);
    }
    Ok(())
}

fn append_field(
    output: &mut Vec<u8>,
    tag: u32,
    value: &str,
    limits: &WireLimits,
) -> Result<(), WireError> {
    validate_value(value, limits)?;
    output.extend_from_slice(tag.to_string().as_bytes());
    output.push(b'=');
    output.extend_from_slice(value.as_bytes());
    output.push(SOH);
    Ok(())
}

fn delimiter_after(bytes: &[u8], start: usize) -> Option<usize> {
    bytes
        .get(start..)?
        .iter()
        .position(|byte| *byte == SOH)
        .map(|offset| start + offset)
}

fn complete_frame_len(bytes: &[u8], limits: &WireLimits) -> Result<Option<usize>, WireError> {
    if bytes.is_empty() {
        return Ok(None);
    }
    if !bytes.starts_with(b"8=FIXT.1.1\x01") {
        return Err(WireError::InvalidBeginString);
    }
    let Some(begin_end) = delimiter_after(bytes, 0) else {
        return Ok(None);
    };
    let body_length_start = begin_end + 1;
    if bytes.get(body_length_start..body_length_start + 2) != Some(b"9=") {
        return Err(WireError::InvalidBodyLength);
    }
    let Some(length_end) = delimiter_after(bytes, body_length_start) else {
        return Ok(None);
    };
    let length_text = std::str::from_utf8(&bytes[body_length_start + 2..length_end])
        .map_err(|_| WireError::InvalidBodyLength)?;
    let body_length = length_text
        .parse::<usize>()
        .map_err(|_| WireError::InvalidBodyLength)?;
    let checksum_start = (length_end + 1)
        .checked_add(body_length)
        .ok_or(WireError::MessageTooLarge)?;
    let total = checksum_start
        .checked_add(7)
        .ok_or(WireError::MessageTooLarge)?;
    if total > limits.max_message_bytes {
        return Err(WireError::MessageTooLarge);
    }
    if bytes.len() < total {
        return Ok(None);
    }
    if bytes.get(checksum_start..checksum_start + 3) != Some(b"10=")
        || bytes.get(total - 1) != Some(&SOH)
    {
        return Err(WireError::InvalidBodyLength);
    }
    Ok(Some(total))
}

fn parse_frame(frame: &[u8], limits: &WireLimits) -> Result<FixMessage, WireError> {
    let checksum_start = frame
        .len()
        .checked_sub(7)
        .ok_or(WireError::InvalidChecksum)?;
    let expected = std::str::from_utf8(&frame[checksum_start + 3..checksum_start + 6])
        .map_err(|_| WireError::InvalidChecksum)?
        .parse::<u8>()
        .map_err(|_| WireError::InvalidChecksum)?;
    let actual = frame[..checksum_start]
        .iter()
        .fold(0_u8, |sum, byte| sum.wrapping_add(*byte));
    if actual != expected {
        return Err(WireError::InvalidChecksum);
    }
    let mut parsed = Vec::new();
    for raw in frame[..checksum_start]
        .split(|byte| *byte == SOH)
        .filter(|field| !field.is_empty())
    {
        let equals = raw
            .iter()
            .position(|byte| *byte == b'=')
            .ok_or(WireError::InvalidField)?;
        let tag = std::str::from_utf8(&raw[..equals])
            .map_err(|_| WireError::InvalidUtf8)?
            .parse::<u32>()
            .map_err(|_| WireError::InvalidField)?;
        let value = std::str::from_utf8(&raw[equals + 1..]).map_err(|_| WireError::InvalidUtf8)?;
        validate_value(value, limits)?;
        parsed.push(Field::new(tag, value));
    }
    if parsed.len() > limits.max_fields + 3 {
        return Err(WireError::TooManyFields);
    }
    let begin_string = parsed
        .first()
        .filter(|field| field.tag == 8)
        .ok_or(WireError::InvalidBeginString)?
        .value
        .clone();
    if begin_string != FIXT_11 {
        return Err(WireError::InvalidBeginString);
    }
    if parsed.get(1).map(|field| field.tag) != Some(9) {
        return Err(WireError::InvalidBodyLength);
    }
    let msg_type = parsed
        .get(2)
        .filter(|field| field.tag == 35)
        .ok_or(WireError::MissingMsgType)?
        .value
        .clone();
    let fields = parsed
        .into_iter()
        .filter(|field| !matches!(field.tag, 8 | 9 | 35))
        .collect();
    Ok(FixMessage {
        begin_string,
        msg_type,
        fields,
    })
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RepeatingGroup {
    pub count_tag: u32,
    pub delimiter_tag: u32,
    pub entries: Vec<Vec<Field>>,
}

impl FixMessage {
    /// Parses a repeating group whose entries begin with `delimiter_tag` and
    /// contain only the supplied member tags.
    ///
    /// # Errors
    /// Returns an error when the declared count, delimiter, or member layout is invalid.
    pub fn repeating_group(
        &self,
        count_tag: u32,
        delimiter_tag: u32,
        member_tags: &[u32],
    ) -> Result<RepeatingGroup, WireError> {
        let count_index = self
            .fields
            .iter()
            .position(|field| field.tag == count_tag)
            .ok_or(WireError::MissingRequiredTag(count_tag))?;
        let count = self.fields[count_index]
            .value
            .parse::<usize>()
            .map_err(|_| WireError::InvalidRepeatingGroup(count_tag))?;
        let mut entries = Vec::with_capacity(count);
        let mut current = Vec::new();
        for field in self.fields.iter().skip(count_index + 1) {
            if field.tag == delimiter_tag {
                if !current.is_empty() {
                    entries.push(core::mem::take(&mut current));
                }
                current.push(field.clone());
            } else if member_tags.contains(&field.tag) {
                if current.is_empty() || current.iter().any(|item: &Field| item.tag == field.tag) {
                    return Err(WireError::InvalidRepeatingGroup(count_tag));
                }
                current.push(field.clone());
            } else {
                break;
            }
        }
        if !current.is_empty() {
            entries.push(current);
        }
        if entries.len() != count
            || entries
                .iter()
                .any(|entry| entry.first().map(|field| field.tag) != Some(delimiter_tag))
        {
            return Err(WireError::InvalidRepeatingGroup(count_tag));
        }
        Ok(RepeatingGroup {
            count_tag,
            delimiter_tag,
            entries,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MessageRule {
    pub msg_type: &'static str,
    pub required_tags: &'static [u32],
    pub allowed_tags: &'static [u32],
}

const STANDARD_HEADER_TAGS: &[u32] = &[49, 56, 34, 43, 52, 122];
const NEW_ORDER_SINGLE: MessageRule = MessageRule {
    msg_type: "D",
    required_tags: &[11, 48, 54, 38, 40],
    allowed_tags: &[11, 48, 54, 38, 40, 44, 59, 60],
};
const CANCEL_REQUEST: MessageRule = MessageRule {
    msg_type: "F",
    required_tags: &[11, 41, 48, 54],
    allowed_tags: &[11, 41, 37, 48, 54, 60],
};
const REPLACE_REQUEST: MessageRule = MessageRule {
    msg_type: "G",
    required_tags: &[11, 41, 48, 54, 38, 40],
    allowed_tags: &[11, 41, 37, 48, 54, 38, 40, 44, 59, 60],
};
const STATUS_REQUEST: MessageRule = MessageRule {
    msg_type: "H",
    required_tags: &[37],
    allowed_tags: &[11, 37, 48, 54],
};
const MARKET_DATA_REQUEST: MessageRule = MessageRule {
    msg_type: "V",
    required_tags: &[262, 263, 264, 267, 269, 48],
    allowed_tags: &[262, 263, 264, 265, 267, 269, 146, 55, 48],
};

#[must_use]
pub fn competition_rule(msg_type: &str) -> Option<&'static MessageRule> {
    match msg_type {
        "D" => Some(&NEW_ORDER_SINGLE),
        "F" => Some(&CANCEL_REQUEST),
        "G" => Some(&REPLACE_REQUEST),
        "H" => Some(&STATUS_REQUEST),
        "V" => Some(&MARKET_DATA_REQUEST),
        _ => None,
    }
}

/// Validates the supported competition application subset.
///
/// # Errors
/// Returns an error for a missing, duplicate, or unsupported application tag.
pub fn validate_competition(message: &FixMessage) -> Result<(), WireError> {
    CompetitionDictionary::load()?.validate(message)?;
    let Some(rule) = competition_rule(&message.msg_type) else {
        return Ok(());
    };
    for required in rule.required_tags {
        if !message.fields.iter().any(|field| field.tag == *required) {
            return Err(WireError::MissingRequiredTag(*required));
        }
    }
    for (index, field) in message.fields.iter().enumerate() {
        if message.fields[..index]
            .iter()
            .any(|prior| prior.tag == field.tag)
            && !matches!(field.tag, 269 | 55 | 48)
        {
            return Err(WireError::DuplicateTag(field.tag));
        }
        if !STANDARD_HEADER_TAGS.contains(&field.tag) && !rule.allowed_tags.contains(&field.tag) {
            return Err(WireError::InvalidField);
        }
    }
    Ok(())
}

/// Loaded standard dictionaries for the competition profile.
#[derive(Debug)]
pub struct CompetitionDictionary {
    session: Dictionary,
    application: Dictionary,
}

impl CompetitionDictionary {
    /// Loads the released FIXT.1.1 and FIX 5.0 SP2 dictionaries.
    ///
    /// # Errors
    /// Returns an error if either embedded dictionary is malformed.
    pub fn load() -> Result<Self, WireError> {
        Ok(Self {
            session: Dictionary::fixt11().map_err(|_| WireError::DictionaryUnavailable)?,
            application: Dictionary::fix50sp2().map_err(|_| WireError::DictionaryUnavailable)?,
        })
    }

    /// Validates standard message and field identities while permitting the bounded Bunting overlay.
    ///
    /// # Errors
    /// Returns an error for unknown messages or tags.
    pub fn validate(&self, message: &FixMessage) -> Result<(), WireError> {
        if message.begin_string != FIXT_11 {
            return Err(WireError::InvalidBeginString);
        }
        let standard = self.session.message_by_msgtype(&message.msg_type).is_some()
            || self
                .application
                .message_by_msgtype(&message.msg_type)
                .is_some();
        let extension = matches!(message.msg_type.as_str(), "U6" | "U9" | "UA" | "UB" | "UC");
        if !standard && !extension {
            return Err(WireError::InvalidMessageType);
        }
        for field in &message.fields {
            if self.session.field_by_tag(field.tag).is_none()
                && self.application.field_by_tag(field.tag).is_none()
                && !(10_000..=10_020).contains(&field.tag)
            {
                return Err(WireError::UnknownTag(field.tag));
            }
        }
        Ok(())
    }

    /// Returns whether the standard dictionary declares a field as exact decimal `Price`.
    #[must_use]
    pub fn is_price_field(&self, tag: u32) -> bool {
        self.application
            .field_by_tag(tag)
            .is_some_and(|field| field.fix_datatype() == FixDatatype::Price)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn decoder_handles_partial_and_coalesced_frames() {
        let limits = WireLimits::default();
        let mut first = FixMessage::new("0");
        first.push(34, "1");
        let mut second = FixMessage::new("1");
        second.push(112, "probe");
        let first = first.encode(&limits).unwrap();
        let second = second.encode(&limits).unwrap();
        let split = first.len() / 2;
        let mut decoder = Decoder::try_new(limits).unwrap();
        assert!(decoder.push(&first[..split]).unwrap().is_empty());
        let mut tail = first[split..].to_vec();
        tail.extend_from_slice(&second);
        let messages = decoder.push(&tail).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].value(112), Some("probe"));
        assert_eq!(decoder.retained_bytes(), 0);
    }

    #[test]
    fn checksum_corruption_is_rejected() {
        let limits = WireLimits::default();
        let mut encoded = FixMessage::new("0").encode(&limits).unwrap();
        encoded[2] = b'X';
        assert_eq!(
            Decoder::try_new(limits).unwrap().push(&encoded),
            Err(WireError::InvalidBeginString)
        );
    }

    #[test]
    fn body_length_and_checksum_are_exact() {
        let limits = WireLimits::default();
        let mut message = FixMessage::new("D");
        message.push(11, "100");
        message.push(48, "7");
        message.push(54, "1");
        message.push(38, "2");
        message.push(40, "2");
        message.push(44, "101");
        let frame = message.encode(&limits).unwrap();
        let decoded = Decoder::try_new(limits).unwrap().push(&frame).unwrap();
        assert_eq!(decoded, vec![message]);
    }

    #[test]
    fn dictionary_and_repeating_group_reject_malformed_messages() {
        let mut request = FixMessage::new("V");
        request.push(262, "book");
        request.push(263, "1");
        request.push(264, "10");
        request.push(267, "2");
        request.push(269, "0");
        request.push(269, "1");
        request.push(48, "7");
        validate_competition(&request).unwrap();
        let group = request.repeating_group(267, 269, &[]).unwrap();
        assert_eq!(group.entries.len(), 2);
        request.fields.retain(|field| field.tag != 48);
        assert_eq!(
            validate_competition(&request),
            Err(WireError::MissingRequiredTag(48))
        );
    }

    #[test]
    fn standard_dictionaries_cover_session_application_and_exact_price() {
        let dictionary = CompetitionDictionary::load().unwrap();
        for message_type in ["A", "0", "D", "V", "AN", "BE"] {
            let message = FixMessage::new(message_type);
            dictionary.validate(&message).unwrap();
        }
        assert!(dictionary.is_price_field(44));
        assert_eq!("9223372036854775807".parse::<i64>().unwrap(), i64::MAX);
    }

    #[test]
    fn unknown_standard_identity_is_rejected() {
        let dictionary = CompetitionDictionary::load().unwrap();
        assert_eq!(
            dictionary.validate(&FixMessage::new("ZZ")),
            Err(WireError::InvalidMessageType)
        );
    }
}
