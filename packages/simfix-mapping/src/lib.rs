#![forbid(unsafe_code)]
//! FIX 5.0 SP2 application mapping with session concerns kept outside market authority.

use bunting_market_events::{NewsAudience, OrderKind, Side};
use bunting_market_types::{
    CurrencyId, InstrumentId, MoneyMinor, NewsId, ParticipantId, PriceTicks, QuantityLots,
};
use quarcc_execution_engine::{
    ExecutionIntent, NormalizedVenueReport, VenueReportKind,
    ids::{ClientOrderId, IntentId, LocalOrderId},
    order::DesiredOrder,
};
use serde::{Deserialize, Serialize};
use simfix_wire::{FixMessage, WireError, validate_competition};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Direct,
    QuarccManaged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InboundApplication {
    Intent(ExecutionIntent),
    MarketDataRequest {
        request_id: String,
        instrument_id: InstrumentId,
        subscription: bool,
        market_depth: usize,
        entry_types: Vec<MarketDataEntryType>,
    },
    Competition(CompetitionRequest),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CompetitionRequest {
    Discovery,
    Account,
    News,
    Tender {
        action: TenderAction,
        tender_id: Option<u128>,
    },
    Score,
    Risk,
    RunControl {
        action: String,
        payload_json: Option<String>,
    },
    RiskAdmin {
        action: String,
        payload_json: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TenderAction {
    List,
    Accept,
    Decline,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunAdvancePayload {
    pub steps: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunReasonPayload {
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishNewsPayload {
    pub news_id: NewsId,
    pub audience: NewsAudience,
    pub headline: String,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyFinePayload {
    pub participant_id: ParticipantId,
    pub currency_id: CurrencyId,
    pub amount: MoneyMinor,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketDataEntryType {
    Bid,
    Offer,
    Trade,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MappingError {
    UnsupportedMessage,
    MissingTag(u32),
    InvalidTag(u32),
    UnsupportedOrderType,
    UnsupportedSubscriptionType,
    Dictionary(WireError),
    Serialization,
    PayloadTooLarge,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappingContext {
    pub participant_id: ParticipantId,
    pub next_intent_id: IntentId,
}

/// Maps FIX orders and market-data requests into transport-neutral application inputs.
///
/// # Errors
///
/// Returns an error when a required field is absent, malformed, or unsupported.
pub fn map_inbound(
    message: &FixMessage,
    context: MappingContext,
) -> Result<InboundApplication, MappingError> {
    validate_competition(message).map_err(MappingError::Dictionary)?;
    match message.msg_type.as_str() {
        "D" => {
            let client_order_id = ClientOrderId::new(parse(message, 11)?);
            let instrument_id = InstrumentId::new(parse(message, 48)?);
            let side = parse_side(required(message, 54)?)?;
            let quantity = QuantityLots::new(parse(message, 38)?);
            let kind = parse_order_kind(message)?;
            Ok(InboundApplication::Intent(ExecutionIntent::Submit {
                intent_id: context.next_intent_id,
                order: DesiredOrder {
                    client_order_id,
                    instrument_id,
                    participant_id: context.participant_id,
                    side,
                    quantity,
                    kind,
                },
            }))
        }
        "F" => Ok(InboundApplication::Intent(ExecutionIntent::Cancel {
            intent_id: context.next_intent_id,
            client_order_id: ClientOrderId::new(parse(message, 41)?),
        })),
        "G" => Ok(InboundApplication::Intent(ExecutionIntent::Replace {
            intent_id: context.next_intent_id,
            client_order_id: ClientOrderId::new(parse(message, 41)?),
            quantity: QuantityLots::new(parse(message, 38)?),
            kind: parse_order_kind(message)?,
        })),
        "H" => Ok(InboundApplication::Intent(ExecutionIntent::Query {
            intent_id: context.next_intent_id,
            local_order_id: LocalOrderId::new(parse(message, 37)?),
        })),
        "V" => {
            let subscription = match required(message, 263)? {
                "0" => false,
                "1" | "2" => true,
                _ => return Err(MappingError::UnsupportedSubscriptionType),
            };
            let group = message
                .repeating_group(267, 269, &[])
                .map_err(MappingError::Dictionary)?;
            let entry_types = group
                .entries
                .iter()
                .map(|entry| match entry[0].value.as_str() {
                    "0" => Ok(MarketDataEntryType::Bid),
                    "1" => Ok(MarketDataEntryType::Offer),
                    "2" => Ok(MarketDataEntryType::Trade),
                    _ => Err(MappingError::InvalidTag(269)),
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(InboundApplication::MarketDataRequest {
                request_id: required(message, 262)?.to_owned(),
                instrument_id: InstrumentId::new(parse(message, 48)?),
                subscription,
                market_depth: parse(message, 264)?,
                entry_types,
            })
        }
        _ => map_competition(message),
    }
}

fn map_competition(message: &FixMessage) -> Result<InboundApplication, MappingError> {
    match message.msg_type.as_str() {
        "x" => Ok(InboundApplication::Competition(
            CompetitionRequest::Discovery,
        )),
        "AN" => Ok(InboundApplication::Competition(CompetitionRequest::Account)),
        "BE" if message.value(10016) == Some("news") => {
            Ok(InboundApplication::Competition(CompetitionRequest::News))
        }
        "U6" => {
            let action = match message.value(10018).unwrap_or("list") {
                "list" => TenderAction::List,
                "accept" => TenderAction::Accept,
                "decline" => TenderAction::Decline,
                _ => return Err(MappingError::InvalidTag(10018)),
            };
            let tender_id = message
                .value(10017)
                .map(str::parse)
                .transpose()
                .map_err(|_| MappingError::InvalidTag(10017))?;
            if action != TenderAction::List && tender_id.is_none() {
                return Err(MappingError::MissingTag(10017));
            }
            Ok(InboundApplication::Competition(
                CompetitionRequest::Tender { action, tender_id },
            ))
        }
        "U9" => Ok(InboundApplication::Competition(CompetitionRequest::Score)),
        "UB" if message.value(10018).is_none_or(|action| action == "query") => {
            Ok(InboundApplication::Competition(CompetitionRequest::Risk))
        }
        "UA" => Ok(InboundApplication::Competition(
            CompetitionRequest::RunControl {
                action: required(message, 10018)?.to_owned(),
                payload_json: message.value(10020).map(ToOwned::to_owned),
            },
        )),
        "UB" => Ok(InboundApplication::Competition(
            CompetitionRequest::RiskAdmin {
                action: required(message, 10018)?.to_owned(),
                payload_json: message.value(10020).map(ToOwned::to_owned),
            },
        )),
        _ => Err(MappingError::UnsupportedMessage),
    }
}

/// Encodes a typed competition projection in the bounded Bunting Orchestra overlay.
///
/// # Errors
/// Returns an error when the payload cannot be encoded or exceeds the profile bound.
pub fn competition_report(
    msg_type: &str,
    audience: &str,
    resource_kind: &str,
    action: &str,
    status: &str,
    committed_sequence: u64,
    payload: &impl Serialize,
) -> Result<FixMessage, MappingError> {
    let payload_json = serde_json::to_string(payload).map_err(|_| MappingError::Serialization)?;
    if payload_json.len() > 16_384 {
        return Err(MappingError::PayloadTooLarge);
    }
    let mut message = FixMessage::new(msg_type);
    message.push(10010, committed_sequence.to_string());
    message.push(10012, audience);
    message.push(10016, resource_kind);
    message.push(10018, action);
    message.push(10019, status);
    message.push(10020, payload_json);
    Ok(message)
}

/// Converts a normalized committed venue result to its FIX application response.
///
/// # Errors
///
/// Returns an error when the report lacks an order identity required by FIX.
pub fn map_execution_report(report: &NormalizedVenueReport) -> Result<FixMessage, MappingError> {
    let local = report.local_order_id.ok_or(MappingError::MissingTag(37))?;
    let mut message = match &report.kind {
        VenueReportKind::CancelRejected { reason } => {
            let mut value = FixMessage::new("9");
            value.push(39, "0");
            value.push(58, reason);
            value
        }
        kind => {
            let mut value = FixMessage::new("8");
            let (exec_type, ord_status) = match kind {
                VenueReportKind::Accepted => ("0", "0"),
                VenueReportKind::Fill { .. }
                    if report
                        .leaves_quantity
                        .is_some_and(|quantity| quantity.get() == 0) =>
                {
                    ("F", "2")
                }
                VenueReportKind::Cancelled => ("4", "4"),
                VenueReportKind::Replaced => ("5", "5"),
                VenueReportKind::Rejected { .. } => ("8", "8"),
                VenueReportKind::Expired => ("C", "C"),
                VenueReportKind::CancelRejected { .. } => unreachable!(),
                VenueReportKind::Fill { .. } => ("F", "1"),
            };
            value.push(150, exec_type);
            value.push(39, ord_status);
            if let VenueReportKind::Fill {
                last_quantity,
                cumulative_quantity,
                price,
            } = kind
            {
                value.push(32, last_quantity.get().to_string());
                value.push(14, cumulative_quantity.get().to_string());
                value.push(31, price.get().to_string());
                if let Some(leaves) = report.leaves_quantity {
                    value.push(151, leaves.get().to_string());
                }
            }
            if let VenueReportKind::Rejected { reason } = kind {
                value.push(58, reason);
            }
            value
        }
    };
    message.push(37, local.get().to_string());
    message.push(17, report.report_id.get().to_string());
    if let Some(client) = &report.client_order_id {
        message.push(11, client.get().to_string());
    }
    Ok(message)
}

#[must_use]
pub fn market_snapshot(
    request_id: &str,
    instrument_id: InstrumentId,
    bids: &[(PriceTicks, QuantityLots)],
    asks: &[(PriceTicks, QuantityLots)],
) -> FixMessage {
    let mut message = FixMessage::new("W");
    message.push(262, request_id);
    message.push(48, instrument_id.get().to_string());
    message.push(268, (bids.len() + asks.len()).to_string());
    for (price, quantity) in bids {
        message.push(269, "0");
        message.push(270, price.get().to_string());
        message.push(271, quantity.get().to_string());
    }
    for (price, quantity) in asks {
        message.push(269, "1");
        message.push(270, price.get().to_string());
        message.push(271, quantity.get().to_string());
    }
    message
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketDataUpdateAction {
    New,
    Change,
    Delete,
}

/// Maps one committed level change to FIX 5.0 SP2 `MarketDataIncrementalRefresh`.
#[must_use]
pub fn market_incremental(
    request_id: &str,
    instrument_id: InstrumentId,
    side: Side,
    action: MarketDataUpdateAction,
    price: PriceTicks,
    quantity: QuantityLots,
) -> FixMessage {
    let mut message = FixMessage::new("X");
    message.push(262, request_id);
    message.push(268, "1");
    message.push(
        279,
        match action {
            MarketDataUpdateAction::New => "0",
            MarketDataUpdateAction::Change => "1",
            MarketDataUpdateAction::Delete => "2",
        },
    );
    message.push(269, if side == Side::Buy { "0" } else { "1" });
    message.push(48, instrument_id.get().to_string());
    message.push(270, price.get().to_string());
    message.push(271, quantity.get().to_string());
    message
}

#[must_use]
pub fn business_reject(reference_type: &str, reason: &str) -> FixMessage {
    let mut message = FixMessage::new("j");
    message.push(372, reference_type);
    message.push(380, "3");
    message.push(58, reason);
    message
}

fn required(message: &FixMessage, tag: u32) -> Result<&str, MappingError> {
    message.value(tag).ok_or(MappingError::MissingTag(tag))
}
fn parse<T: std::str::FromStr>(message: &FixMessage, tag: u32) -> Result<T, MappingError> {
    required(message, tag)?
        .parse()
        .map_err(|_| MappingError::InvalidTag(tag))
}
fn parse_side(value: &str) -> Result<Side, MappingError> {
    match value {
        "1" => Ok(Side::Buy),
        "2" => Ok(Side::Sell),
        _ => Err(MappingError::InvalidTag(54)),
    }
}
fn parse_order_kind(message: &FixMessage) -> Result<OrderKind, MappingError> {
    match required(message, 40)? {
        "1" => Ok(OrderKind::Market),
        "2" => Ok(OrderKind::Limit {
            price: PriceTicks::new(parse(message, 44)?),
        }),
        _ => Err(MappingError::UnsupportedOrderType),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use quarcc_execution_engine::ids::{ReportId, VenueOrderId};

    #[test]
    fn new_order_single_maps_to_exact_intent() -> Result<(), MappingError> {
        let mut message = FixMessage::new("D");
        message.push(11, "1");
        message.push(48, "7");
        message.push(54, "1");
        message.push(38, "3");
        message.push(40, "2");
        message.push(44, "101");
        let mapped = map_inbound(
            &message,
            MappingContext {
                participant_id: ParticipantId::new(9),
                next_intent_id: IntentId::new(10),
            },
        )?;
        let InboundApplication::Intent(ExecutionIntent::Submit { order, .. }) = mapped else {
            return Err(MappingError::UnsupportedMessage);
        };
        assert_eq!(order.quantity, QuantityLots::new(3));
        assert_eq!(
            order.kind,
            OrderKind::Limit {
                price: PriceTicks::new(101)
            }
        );
        Ok(())
    }

    #[test]
    fn all_supported_inbound_types_map_or_validate_deterministically() -> Result<(), MappingError> {
        let context = MappingContext {
            participant_id: ParticipantId::new(9),
            next_intent_id: IntentId::new(10),
        };
        let mut cancel = FixMessage::new("F");
        for (tag, value) in [(11, "2"), (41, "1"), (48, "7"), (54, "1")] {
            cancel.push(tag, value);
        }
        assert!(matches!(
            map_inbound(&cancel, context)?,
            InboundApplication::Intent(ExecutionIntent::Cancel { .. })
        ));

        let mut replace = FixMessage::new("G");
        for (tag, value) in [
            (11, "3"),
            (41, "1"),
            (48, "7"),
            (54, "1"),
            (38, "4"),
            (40, "2"),
            (44, "102"),
        ] {
            replace.push(tag, value);
        }
        assert!(matches!(
            map_inbound(&replace, context)?,
            InboundApplication::Intent(ExecutionIntent::Replace { .. })
        ));

        let mut status = FixMessage::new("H");
        status.push(37, "1");
        assert!(matches!(
            map_inbound(&status, context)?,
            InboundApplication::Intent(ExecutionIntent::Query { .. })
        ));

        let mut market = FixMessage::new("V");
        for (tag, value) in [
            (262, "book"),
            (263, "1"),
            (264, "10"),
            (267, "2"),
            (269, "0"),
            (269, "1"),
            (48, "7"),
        ] {
            market.push(tag, value);
        }
        let InboundApplication::MarketDataRequest {
            market_depth,
            entry_types,
            ..
        } = map_inbound(&market, context)?
        else {
            return Err(MappingError::UnsupportedMessage);
        };
        assert_eq!(market_depth, 10);
        assert_eq!(
            entry_types,
            vec![MarketDataEntryType::Bid, MarketDataEntryType::Offer]
        );
        Ok(())
    }

    #[test]
    fn snapshot_and_incremental_messages_use_competition_group_layout() {
        let snapshot = market_snapshot(
            "book",
            InstrumentId::new(7),
            &[(PriceTicks::new(100), QuantityLots::new(2))],
            &[(PriceTicks::new(101), QuantityLots::new(3))],
        );
        assert_eq!(snapshot.value(268), Some("2"));
        let update = market_incremental(
            "book",
            InstrumentId::new(7),
            Side::Buy,
            MarketDataUpdateAction::Change,
            PriceTicks::new(100),
            QuantityLots::new(4),
        );
        assert_eq!(update.msg_type, "X");
        assert_eq!(update.value(279), Some("1"));
    }

    #[test]
    fn execution_reports_distinguish_partial_and_complete_fills() -> Result<(), MappingError> {
        let report = |leaves| NormalizedVenueReport {
            report_id: ReportId::new(1),
            source_sequence: Some(4),
            client_order_id: Some(ClientOrderId::new(2)),
            local_order_id: Some(LocalOrderId::new(3)),
            venue_order_id: Some(VenueOrderId::new("venue-3")),
            leaves_quantity: Some(QuantityLots::new(leaves)),
            kind: VenueReportKind::Fill {
                last_quantity: QuantityLots::new(2),
                cumulative_quantity: QuantityLots::new(4),
                price: PriceTicks::new(101),
            },
        };
        let partial = map_execution_report(&report(6))?;
        assert_eq!(partial.value(39), Some("1"));
        assert_eq!(partial.value(151), Some("6"));
        let complete = map_execution_report(&report(0))?;
        assert_eq!(complete.value(39), Some("2"));
        assert_eq!(complete.value(151), Some("0"));
        Ok(())
    }

    #[test]
    fn competition_requests_use_standard_messages_and_bounded_extension_reports() {
        let context = MappingContext {
            participant_id: ParticipantId::new(9),
            next_intent_id: IntentId::new(10),
        };
        assert_eq!(
            map_inbound(&FixMessage::new("x"), context).unwrap(),
            InboundApplication::Competition(CompetitionRequest::Discovery)
        );
        assert_eq!(
            map_inbound(&FixMessage::new("AN"), context).unwrap(),
            InboundApplication::Competition(CompetitionRequest::Account)
        );
        let mut tender = FixMessage::new("U6");
        tender.push(10018, "accept");
        tender.push(10017, "42");
        assert_eq!(
            map_inbound(&tender, context).unwrap(),
            InboundApplication::Competition(CompetitionRequest::Tender {
                action: TenderAction::Accept,
                tender_id: Some(42),
            })
        );
        let report = competition_report(
            "U9",
            "private",
            "score",
            "snapshot",
            "ok",
            7,
            &serde_json::json!({"score":"100","policy":"bunting.score.nlv-rank.v1"}),
        )
        .unwrap();
        assert_eq!(report.value(10010), Some("7"));
        assert!(report.value(10020).unwrap().len() <= 16_384);
    }
}
