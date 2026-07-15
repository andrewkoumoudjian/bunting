#![forbid(unsafe_code)]
//! Rust-owned public procedure types and deterministic contract metadata.

use core::{fmt, str::FromStr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const API_VERSION: &str = "bunting.v1";
pub const PRODUCT_CONTRACT_VERSION: &str = "bunting.product.v1";
pub const FIX_COMPETITION_PROFILE_VERSION: &str = "bunting.fix44.competition.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecimalStringError;
impl fmt::Display for DecimalStringError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("expected a canonical in-range decimal string")
    }
}
impl std::error::Error for DecimalStringError {}

macro_rules! decimal_string {
    ($name:ident, $inner:ty, $valid:expr) => {
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $name($inner);
        impl $name {
            #[must_use]
            pub const fn new(value: $inner) -> Self {
                Self(value)
            }
            #[must_use]
            pub const fn get(&self) -> $inner {
                self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
        impl FromStr for $name {
            type Err = DecimalStringError;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                if !($valid)(value) {
                    return Err(DecimalStringError);
                }
                value.parse().map(Self).map_err(|_| DecimalStringError)
            }
        }
        impl Serialize for $name {
            fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0.to_string())
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(serde::de::Error::custom)
            }
        }
    };
}

fn canonical_unsigned(value: &str) -> bool {
    !value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn canonical_signed(value: &str) -> bool {
    if let Some(magnitude) = value.strip_prefix('-') {
        canonical_unsigned(magnitude) && magnitude != "0"
    } else {
        canonical_unsigned(value)
    }
}

decimal_string!(UnsignedDecimalString, u128, canonical_unsigned);
decimal_string!(SequenceDecimalString, u64, canonical_unsigned);
decimal_string!(SignedDecimalString, i64, canonical_signed);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HealthOutput {
    pub api_version: String,
    pub service_version: String,
    pub orderbook_version: String,
    pub contract_compatible: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubmitOrderInput {
    pub run_id: UnsignedDecimalString,
    pub instrument_id: UnsignedDecimalString,
    pub command_id: UnsignedDecimalString,
    pub correlation_id: UnsignedDecimalString,
    pub expected_sequence: SequenceDecimalString,
    pub logical_time_ns: SequenceDecimalString,
    pub order_id: UnsignedDecimalString,
    pub side: Side,
    pub price_ticks: SignedDecimalString,
    pub quantity_lots: SignedDecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelOrderInput {
    pub run_id: UnsignedDecimalString,
    pub instrument_id: UnsignedDecimalString,
    pub command_id: UnsignedDecimalString,
    pub correlation_id: UnsignedDecimalString,
    pub expected_sequence: SequenceDecimalString,
    pub logical_time_ns: SequenceDecimalString,
    pub order_id: UnsignedDecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketSnapshotInput {
    pub run_id: UnsignedDecimalString,
    pub instrument_id: UnsignedDecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketSubscribeInput {
    pub run_id: UnsignedDecimalString,
    pub instrument_id: UnsignedDecimalString,
    pub after_sequence: SequenceDecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountsSubscribeInput {
    pub run_id: UnsignedDecimalString,
    pub after_sequence: SequenceDecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandOutput {
    pub accepted: bool,
    pub reject_code: Option<String>,
    pub committed_sequence: SequenceDecimalString,
    pub order_id: Option<UnsignedDecimalString>,
    pub snapshot_checksum: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BuntingErrorCode {
    Unauthenticated,
    InvalidInput,
    NotFound,
    DuplicateCommandConflict,
    VersionConflict,
    RiskRejected,
    OriginUnavailable,
    InternalContractMismatch,
}

impl BuntingErrorCode {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unauthenticated => "UNAUTHENTICATED",
            Self::InvalidInput => "INVALID_INPUT",
            Self::NotFound => "NOT_FOUND",
            Self::DuplicateCommandConflict => "DUPLICATE_COMMAND_CONFLICT",
            Self::VersionConflict => "VERSION_CONFLICT",
            Self::RiskRejected => "RISK_REJECTED",
            Self::OriginUnavailable => "ORIGIN_UNAVAILABLE",
            Self::InternalContractMismatch => "INTERNAL_CONTRACT_MISMATCH",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PriceLevel {
    pub price_ticks: SignedDecimalString,
    pub quantity_lots: SignedDecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketSnapshotOutput {
    pub run_id: UnsignedDecimalString,
    pub instrument_id: UnsignedDecimalString,
    pub sequence: SequenceDecimalString,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
}

/// Authenticated product roles. A built-in agent is a participant-scoped
/// service identity, not an administrator or a second market authority.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorRole {
    Participant,
    Team,
    Instructor,
    Administrator,
    BuiltInAgent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActorIdentity {
    pub actor_id: UnsignedDecimalString,
    pub role: ActorRole,
    pub participant_id: Option<UnsignedDecimalString>,
    pub team_id: Option<UnsignedDecimalString>,
}

/// Audience attached to every externally publishable projection or event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum Audience {
    Public,
    Participant(UnsignedDecimalString),
    Team(UnsignedDecimalString),
    Instructor,
    Administrator,
}

/// Applies the product's deny-by-default projection boundary.
#[must_use]
pub fn audience_allows(actor: &ActorIdentity, audience: &Audience) -> bool {
    if actor.role == ActorRole::Administrator {
        return true;
    }
    match audience {
        Audience::Public => true,
        Audience::Participant(participant_id) => {
            matches!(actor.role, ActorRole::Participant | ActorRole::BuiltInAgent)
                && actor.participant_id.as_ref() == Some(participant_id)
        }
        Audience::Team(team_id) => {
            matches!(
                actor.role,
                ActorRole::Participant | ActorRole::Team | ActorRole::BuiltInAgent
            ) && actor.team_id.as_ref() == Some(team_id)
        }
        Audience::Instructor => actor.role == ActorRole::Instructor,
        Audience::Administrator => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcedureKind {
    Query,
    Mutation,
    Subscription,
}

#[must_use]
pub fn procedure_kind(path: &str) -> Option<ProcedureKind> {
    match path {
        "system.health" | "market.snapshot" => Some(ProcedureKind::Query),
        "orders.submit" | "orders.cancel" => Some(ProcedureKind::Mutation),
        "market.subscribe" | "accounts.subscribe" => Some(ProcedureKind::Subscription),
        _ => None,
    }
}

/// Generates the complete implemented Rust contract descriptor in stable key order.
#[must_use]
pub fn generated_schema() -> Value {
    json!({
        "schemaVersion": "bunting.rust-contract.v1",
        "apiVersion": API_VERSION,
        "types": {
            "id": {"encoding":"unsigned_decimal_string","rust":"u128","minimum":"0","maximum":"340282366920938463463374607431768211455"},
            "sequence": {"encoding":"unsigned_decimal_string","rust":"u64","minimum":"0","maximum":"18446744073709551615"},
            "marketUnit": {"encoding":"signed_decimal_string","rust":"i64","minimum":"-9223372036854775808","maximum":"9223372036854775807"},
            "side": {"encoding":"json_string","values":["buy","sell"]}
        },
        "procedures": [
            {"name":"market.snapshot","kind":"query","input":{"runId":"id","instrumentId":"id"},"output":{"runId":"id","instrumentId":"id","sequence":"sequence","bids":"priceLevel[]","asks":"priceLevel[]"}},
            {"name":"orders.cancel","kind":"mutation","input":{"runId":"id","instrumentId":"id","commandId":"id","correlationId":"id","expectedSequence":"sequence","logicalTimeNs":"sequence","orderId":"id"},"output":{"accepted":"boolean","rejectCode":"string?","committedSequence":"sequence","orderId":"id?","snapshotChecksum":"string?"}},
            {"name":"orders.submit","kind":"mutation","input":{"runId":"id","instrumentId":"id","commandId":"id","correlationId":"id","expectedSequence":"sequence","logicalTimeNs":"sequence","orderId":"id","side":"side","priceTicks":"marketUnit","quantityLots":"marketUnit"},"output":{"accepted":"boolean","rejectCode":"string?","committedSequence":"sequence","orderId":"id?","snapshotChecksum":"string?"}},
            {"name":"system.health","kind":"query","input":{},"output":{"apiVersion":"string","serviceVersion":"string","orderbookVersion":"string","contractCompatible":"boolean"}}
            ,{"name":"market.subscribe","kind":"subscription","input":{"runId":"id","instrumentId":"id","afterSequence":"sequence"},"output":{"event":"committedEvent|stream.reset|market.snapshot"}}
            ,{"name":"accounts.subscribe","kind":"subscription","input":{"runId":"id","afterSequence":"sequence"},"output":{"event":"committedPrivateEvent|stream.reset"}}
        ],
        "structures": {"priceLevel":{"priceTicks":"marketUnit","quantityLots":"marketUnit"}}
    })
}

#[must_use]
pub fn schema_hash() -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = serde_json::to_vec(&generated_schema()).unwrap_or_default();
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimal_types_enforce_signedness_ranges_and_canonical_json()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(
            serde_json::from_str::<SignedDecimalString>("\"-9223372036854775808\"")?.get(),
            i64::MIN
        );
        assert_eq!(
            serde_json::from_str::<SignedDecimalString>("\"9223372036854775807\"")?.get(),
            i64::MAX
        );
        assert_eq!(
            serde_json::from_str::<SignedDecimalString>("\"-17\"")?.get(),
            -17
        );
        for invalid in [
            "17",
            "\"+1\"",
            "\"01\"",
            "\"-0\"",
            "\"-01\"",
            "\"9223372036854775808\"",
            "\"-9223372036854775809\"",
        ] {
            assert!(
                serde_json::from_str::<SignedDecimalString>(invalid).is_err(),
                "accepted {invalid}"
            );
        }
        assert_eq!(
            serde_json::from_str::<UnsignedDecimalString>(
                "\"340282366920938463463374607431768211455\""
            )?
            .get(),
            u128::MAX
        );
        assert_eq!(
            serde_json::from_str::<SequenceDecimalString>("\"18446744073709551615\"")?.get(),
            u64::MAX
        );
        assert!(serde_json::from_str::<UnsignedDecimalString>("-1").is_err());
        assert!(serde_json::from_str::<SequenceDecimalString>("\"18446744073709551616\"").is_err());
        Ok(())
    }

    #[test]
    fn generated_contract_exactly_matches_hash_pinned_canonical_artifact()
    -> Result<(), Box<dyn std::error::Error>> {
        let canonical: Value =
            serde_json::from_str(include_str!("../../../schemas/browser/bunting.v1.json"))?;
        assert_eq!(
            generated_schema(),
            canonical["implemented_rust_contract"]["descriptor"]
        );
        assert_eq!(
            schema_hash(),
            canonical["implemented_rust_contract"]["sha256"]
        );
        Ok(())
    }

    #[test]
    fn audience_boundaries_are_deny_by_default() {
        let participant = ActorIdentity {
            actor_id: UnsignedDecimalString::new(1),
            role: ActorRole::Participant,
            participant_id: Some(UnsignedDecimalString::new(10)),
            team_id: Some(UnsignedDecimalString::new(20)),
        };
        let other_participant = Audience::Participant(UnsignedDecimalString::new(11));
        assert!(audience_allows(&participant, &Audience::Public));
        assert!(audience_allows(
            &participant,
            &Audience::Participant(UnsignedDecimalString::new(10))
        ));
        assert!(audience_allows(
            &participant,
            &Audience::Team(UnsignedDecimalString::new(20))
        ));
        assert!(!audience_allows(&participant, &other_participant));
        assert!(!audience_allows(&participant, &Audience::Instructor));
        assert!(!audience_allows(&participant, &Audience::Administrator));

        let instructor = ActorIdentity {
            actor_id: UnsignedDecimalString::new(2),
            role: ActorRole::Instructor,
            participant_id: None,
            team_id: None,
        };
        assert!(audience_allows(&instructor, &Audience::Instructor));
        assert!(!audience_allows(
            &instructor,
            &Audience::Participant(UnsignedDecimalString::new(10))
        ));

        let administrator = ActorIdentity {
            actor_id: UnsignedDecimalString::new(3),
            role: ActorRole::Administrator,
            participant_id: None,
            team_id: None,
        };
        assert!(audience_allows(&administrator, &other_participant));
        assert!(audience_allows(&administrator, &Audience::Administrator));
    }

    #[test]
    fn versioned_product_schemas_are_well_formed() -> Result<(), Box<dyn std::error::Error>> {
        let product: Value = serde_json::from_str(include_str!(
            "../../../schemas/product/bunting.product.v1.json"
        ))?;
        let fix: Value = serde_json::from_str(include_str!(
            "../../../schemas/fix/bunting.fix44.competition.v1.json"
        ))?;
        assert_eq!(product["contractVersion"], PRODUCT_CONTRACT_VERSION);
        assert_eq!(fix["profileVersion"], FIX_COMPETITION_PROFILE_VERSION);
        assert_eq!(product["fixProfile"], FIX_COMPETITION_PROFILE_VERSION);
        assert_eq!(product["applicationService"]["authority"], "bunting-engine");

        let messages = fix["messages"].as_array().ok_or("messages")?;
        let mut message_types = std::collections::BTreeSet::new();
        for message in messages {
            let message_type = message["msgType"].as_str().ok_or("msgType")?;
            assert!(
                message_types.insert(message_type),
                "duplicate {message_type}"
            );
            let audience = message["audience"].as_str().ok_or("audience")?;
            assert!(matches!(
                audience,
                "public" | "private" | "admin" | "session"
            ));
        }
        let fields = fix["extensionFields"].as_array().ok_or("fields")?;
        let mut tags = std::collections::BTreeSet::new();
        for field in fields {
            let tag = field["tag"].as_u64().ok_or("tag")?;
            assert!(
                tag >= 10_000,
                "extension tag {tag} is outside Bunting range"
            );
            assert!(tags.insert(tag), "duplicate extension tag {tag}");
        }
        assert_eq!(fix["transport"]["cloudflareIngress"], "none");
        Ok(())
    }
}
