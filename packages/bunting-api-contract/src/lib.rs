#![forbid(unsafe_code)]
//! Rust-owned public procedure types and deterministic contract metadata.

use core::{fmt, str::FromStr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const API_VERSION: &str = "bunting.v1";

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
pub struct CommandOutput {
    pub accepted: bool,
    pub sequence: SequenceDecimalString,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcedureKind {
    Query,
    Mutation,
}

#[must_use]
pub fn procedure_kind(path: &str) -> Option<ProcedureKind> {
    match path {
        "system.health" | "market.snapshot" => Some(ProcedureKind::Query),
        "orders.submit" | "orders.cancel" => Some(ProcedureKind::Mutation),
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
            {"name":"orders.cancel","kind":"mutation","input":{"runId":"id","instrumentId":"id","commandId":"id","correlationId":"id","expectedSequence":"sequence","logicalTimeNs":"sequence","orderId":"id"},"output":{"accepted":"boolean","sequence":"sequence"}},
            {"name":"orders.submit","kind":"mutation","input":{"runId":"id","instrumentId":"id","commandId":"id","correlationId":"id","expectedSequence":"sequence","logicalTimeNs":"sequence","orderId":"id","side":"side","priceTicks":"marketUnit","quantityLots":"marketUnit"},"output":{"accepted":"boolean","sequence":"sequence"}},
            {"name":"system.health","kind":"query","input":{},"output":{"apiVersion":"string","serviceVersion":"string","orderbookVersion":"string","contractCompatible":"boolean"}}
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
            serde_json::from_str(include_str!("../../../schemas/trpc/bunting.v1.json"))?;
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
}
