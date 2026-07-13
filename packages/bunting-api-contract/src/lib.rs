#![forbid(unsafe_code)]
//! Rust-owned public procedure types and deterministic contract metadata.

use core::{fmt, str::FromStr};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const API_VERSION: &str = "bunting.v1";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecimalString(u128);

impl DecimalString {
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }
    #[must_use]
    pub const fn get(&self) -> u128 {
        self.0
    }
}

impl fmt::Display for DecimalString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for DecimalString {
    type Err = DecimalStringError;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty()
            || (value.len() > 1 && value.starts_with('0'))
            || !value.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(DecimalStringError);
        }
        value.parse().map(Self).map_err(|_| DecimalStringError)
    }
}

impl Serialize for DecimalString {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for DecimalString {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DecimalStringError;
impl fmt::Display for DecimalStringError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("expected a canonical unsigned decimal string")
    }
}
impl std::error::Error for DecimalStringError {}

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
    pub run_id: DecimalString,
    pub instrument_id: DecimalString,
    pub command_id: DecimalString,
    pub correlation_id: DecimalString,
    pub expected_sequence: DecimalString,
    pub logical_time_ns: DecimalString,
    pub order_id: DecimalString,
    pub side: Side,
    pub price_ticks: DecimalString,
    pub quantity_lots: DecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CancelOrderInput {
    pub run_id: DecimalString,
    pub instrument_id: DecimalString,
    pub command_id: DecimalString,
    pub correlation_id: DecimalString,
    pub expected_sequence: DecimalString,
    pub logical_time_ns: DecimalString,
    pub order_id: DecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketSnapshotInput {
    pub run_id: DecimalString,
    pub instrument_id: DecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandOutput {
    pub accepted: bool,
    pub sequence: DecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PriceLevel {
    pub price_ticks: DecimalString,
    pub quantity_lots: DecimalString,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MarketSnapshotOutput {
    pub run_id: DecimalString,
    pub instrument_id: DecimalString,
    pub sequence: DecimalString,
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

/// Generates the implemented Rust contract descriptor in stable key order.
#[must_use]
pub fn generated_schema() -> Value {
    json!({
        "apiVersion": API_VERSION,
        "wideIntegerEncoding": "validated_decimal_string",
        "procedures": [
            {"name":"market.snapshot","kind":"query","input":["runId","instrumentId"]},
            {"name":"orders.cancel","kind":"mutation","input":["runId","instrumentId","commandId","correlationId","expectedSequence","logicalTimeNs","orderId"]},
            {"name":"orders.submit","kind":"mutation","input":["runId","instrumentId","commandId","correlationId","expectedSequence","logicalTimeNs","orderId","side","priceTicks","quantityLots"]},
            {"name":"system.health","kind":"query","input":[]}
        ]
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
    fn wide_values_are_strings_and_reject_noncanonical_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let value: DecimalString = serde_json::from_str("\"9007199254740993\"")?;
        assert_eq!(value.get(), 9_007_199_254_740_993);
        assert!(serde_json::from_str::<DecimalString>("9007199254740993").is_err());
        assert!(serde_json::from_str::<DecimalString>("\"01\"").is_err());
        Ok(())
    }

    #[test]
    fn generated_contract_is_deterministic_and_matches_canonical_procedures()
    -> Result<(), Box<dyn std::error::Error>> {
        assert_eq!(schema_hash().len(), 64);
        let canonical: Value =
            serde_json::from_str(include_str!("../../../schemas/trpc/bunting.v1.json"))?;
        let generated = generated_schema();
        let Some(procedures) = generated["procedures"].as_array() else {
            return Err("generated procedures must be an array".into());
        };
        for procedure in procedures {
            let name = &procedure["name"];
            assert!(canonical["procedures"].as_array().is_some_and(|items| {
                items
                    .iter()
                    .any(|item| &item["name"] == name && item["kind"] == procedure["kind"])
            }));
        }
        Ok(())
    }
}
