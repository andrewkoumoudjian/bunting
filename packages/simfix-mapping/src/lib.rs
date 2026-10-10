#![forbid(unsafe_code)]
//! FIX 5.0 SP2 application mapping with session concerns kept outside market authority.

use bunting_market_events::{NewsAudience, OrderKind, Side};
use bunting_market_types::{
    CurrencyId, InstrumentId, ListingKey, MoneyMinor, NewsId, ParticipantId, PriceTicks,
    QuantityLots, VenueId,
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
        listing_key: ListingKey,
        request_type: MarketDataRequestType,
        market_depth: usize,
        entry_types: Vec<MarketDataEntryType>,
        /// `false` for order-by-order (266=N), else price levels.
        aggregated: bool,
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

/// FIX `SubscriptionRequestType` (263).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketDataRequestType {
    /// `0`: one snapshot.
    Snapshot,
    /// `1`: a snapshot followed by incremental updates.
    Subscribe,
    /// `2`: stop the updates of an earlier subscription with the same 262.
    Unsubscribe,
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

/// Mandatory numeric exchange for a FIX `NewOrderSingle`. Non-submission
/// messages return None; exchange selection never defaults by instrument.
///
/// # Errors
/// Returns an error for an absent, zero or malformed listing identifier.
pub fn fix_order_listing(message: &FixMessage) -> Result<Option<ListingKey>, MappingError> {
    if message.msg_type != "D" {
        return Ok(None);
    }
    let instrument = parse::<u128>(message, 48)?;
    let venue = parse::<u128>(message, 207)?;
    if instrument == 0 {
        return Err(MappingError::InvalidTag(48));
    }
    if venue == 0 {
        return Err(MappingError::InvalidTag(207));
    }
    Ok(Some(ListingKey::new(
        VenueId::new(venue),
        InstrumentId::new(instrument),
    )))
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
            let instrument_id = fix_order_listing(message)?
                .ok_or(MappingError::MissingTag(207))?
                .instrument_id;
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
                    // BuntingAnonymous (10021): hide this order's broker
                    // identifier on venues that publish them.
                    anonymous: match message.value(10021) {
                        None | Some("N") => false,
                        Some("Y") => true,
                        Some(_) => return Err(MappingError::InvalidTag(10021)),
                    },
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
            let request_type = match required(message, 263)? {
                "0" => MarketDataRequestType::Snapshot,
                "1" => MarketDataRequestType::Subscribe,
                "2" => MarketDataRequestType::Unsubscribe,
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
                listing_key: ListingKey::new(
                    VenueId::new(parse(message, 207)?),
                    InstrumentId::new(parse(message, 48)?),
                ),
                request_type,
                market_depth: parse(message, 264)?,
                entry_types,
                // 266 (AggregatedBook): N asks for order-by-order data.
                aggregated: match message.value(266) {
                    None | Some("Y") => true,
                    Some("N") => false,
                    Some(_) => return Err(MappingError::InvalidTag(266)),
                },
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
    listing_key: ListingKey,
    bids: &[(PriceTicks, QuantityLots)],
    asks: &[(PriceTicks, QuantityLots)],
) -> FixMessage {
    let mut message = FixMessage::new("W");
    message.push(262, request_id);
    message.push(48, listing_key.instrument_id.get().to_string());
    message.push(207, listing_key.venue_id.get().to_string());
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

/// Broker identifiers (ADR 0036): `MDEntryBuyer` (288) and
/// `MDEntrySeller` (289), absent where the venue publishes none or the
/// order is anonymous.
fn push_brokers(
    message: &mut FixMessage,
    buyer: Option<ParticipantId>,
    seller: Option<ParticipantId>,
) {
    if let Some(buyer) = buyer {
        message.push(288, buyer.get().to_string());
    }
    if let Some(seller) = seller {
        message.push(289, seller.get().to_string());
    }
}

/// One displayed order in an order-by-order snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SnapshotOrder {
    pub side: Side,
    pub reference: u64,
    pub price: PriceTicks,
    pub quantity: QuantityLots,
    pub broker: Option<ParticipantId>,
}

/// An order-by-order (L3) snapshot of one listing (266=N): every displayed
/// order by its anonymous public reference (278), bids then asks, each in
/// matching priority with its queue position (290, from 1 per side), and
/// its broker identifier (288 or 289) where published.
#[must_use]
pub fn order_snapshot(
    request_id: &str,
    listing_key: ListingKey,
    orders: &[SnapshotOrder],
) -> FixMessage {
    let mut message = FixMessage::new("W");
    message.push(262, request_id);
    message.push(48, listing_key.instrument_id.get().to_string());
    message.push(207, listing_key.venue_id.get().to_string());
    message.push(266, "N");
    message.push(268, orders.len().to_string());
    let mut position = [0_u64; 2];
    for order in orders {
        let index = usize::from(order.side == Side::Sell);
        position[index] = position[index].saturating_add(1);
        message.push(269, if order.side == Side::Buy { "0" } else { "1" });
        message.push(278, order.reference.to_string());
        match order.side {
            Side::Buy => push_brokers(&mut message, order.broker, None),
            Side::Sell => push_brokers(&mut message, None, order.broker),
        }
        message.push(270, order.price.get().to_string());
        message.push(271, order.quantity.get().to_string());
        message.push(290, position[index].to_string());
    }
    message
}

/// The consolidated tape's snapshot of one instrument: each venue's best bid
/// and offer as the tape's processor last saw them, every entry naming its
/// venue (207). `83` is the report sequence of the last change included, so
/// the increments that follow start at the next one.
#[must_use]
pub fn consolidated_snapshot(
    request_id: &str,
    instrument_id: InstrumentId,
    last_report_sequence: u64,
    quotes: &[(ListingKey, Side, PriceTicks, QuantityLots)],
) -> FixMessage {
    let mut message = FixMessage::new("W");
    message.push(262, request_id);
    message.push(48, instrument_id.get().to_string());
    message.push(207, CONSOLIDATED_EXCHANGE.to_string());
    message.push(83, last_report_sequence.to_string());
    message.push(268, quotes.len().to_string());
    for (listing_key, side, price, quantity) in quotes {
        message.push(269, if *side == Side::Buy { "0" } else { "1" });
        message.push(207, listing_key.venue_id.get().to_string());
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

/// One entry of a public incremental refresh.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MarketDataIncrement {
    /// The visible quantity at one price level after the change (zero for
    /// a `Delete`).
    Level {
        action: MarketDataUpdateAction,
        side: Side,
        price: PriceTicks,
        quantity: QuantityLots,
    },
    /// One displayed order on an order-by-order feed, by its anonymous
    /// public reference (278); the displayed quantity after the change.
    /// `broker` is the owner's broker identifier where the venue publishes
    /// them (288 on bids, 289 on offers).
    Order {
        action: MarketDataUpdateAction,
        side: Side,
        reference: u64,
        price: PriceTicks,
        quantity: QuantityLots,
        broker: Option<ParticipantId>,
    },
    /// One trade; on order-by-order feeds `reference` (278) names the
    /// resting order it executed against. `buyer` (288) and `seller` (289)
    /// are broker identifiers where the venue publishes them.
    Trade {
        price: PriceTicks,
        quantity: QuantityLots,
        reference: Option<u64>,
        buyer: Option<ParticipantId>,
        seller: Option<ParticipantId>,
    },
}

/// `SecurityExchange` (207) value of the consolidated tape in market-data
/// requests and messages; venue IDs are positive.
pub const CONSOLIDATED_EXCHANGE: u128 = 0;

/// Maps committed public changes to one FIX 5.0 SP2
/// `MarketDataIncrementalRefresh` (35=X). Every entry names its listing
/// (48 + 207) and carries the feed's next report sequence (83), increasing
/// by one per entry, so a gap is detectable.
#[must_use]
pub fn market_incremental(
    request_id: &str,
    first_report_sequence: u64,
    entries: &[(ListingKey, MarketDataIncrement)],
) -> FixMessage {
    let mut message = FixMessage::new("X");
    message.push(262, request_id);
    message.push(268, entries.len().to_string());
    for (offset, (listing_key, entry)) in (0_u64..).zip(entries) {
        let side_type = |side| if side == Side::Buy { "0" } else { "1" };
        let (action, entry_type, reference, price, quantity) = match *entry {
            MarketDataIncrement::Level {
                action,
                side,
                price,
                quantity,
            } => (action, side_type(side), None, price, quantity),
            MarketDataIncrement::Order {
                action,
                side,
                reference,
                price,
                quantity,
                ..
            } => (action, side_type(side), Some(reference), price, quantity),
            MarketDataIncrement::Trade {
                price,
                quantity,
                reference,
                ..
            } => (MarketDataUpdateAction::New, "2", reference, price, quantity),
        };
        let (buyer, seller) = match *entry {
            MarketDataIncrement::Order {
                side: Side::Buy,
                broker,
                ..
            } => (broker, None),
            MarketDataIncrement::Order {
                side: Side::Sell,
                broker,
                ..
            } => (None, broker),
            MarketDataIncrement::Trade { buyer, seller, .. } => (buyer, seller),
            MarketDataIncrement::Level { .. } => (None, None),
        };
        message.push(
            279,
            match action {
                MarketDataUpdateAction::New => "0",
                MarketDataUpdateAction::Change => "1",
                MarketDataUpdateAction::Delete => "2",
            },
        );
        message.push(269, entry_type);
        message.push(48, listing_key.instrument_id.get().to_string());
        message.push(207, listing_key.venue_id.get().to_string());
        if let Some(reference) = reference {
            message.push(278, reference.to_string());
        }
        push_brokers(&mut message, buyer, seller);
        message.push(270, price.get().to_string());
        message.push(271, quantity.get().to_string());
        message.push(83, first_report_sequence.saturating_add(offset).to_string());
    }
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
        message.push(207, "2");
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
        assert_eq!(
            fix_order_listing(&message)?,
            Some(ListingKey::new(VenueId::new(2), InstrumentId::new(7)))
        );
        let mut missing_exchange = message.clone();
        missing_exchange.fields.retain(|field| field.tag != 207);
        assert_eq!(
            map_inbound(
                &missing_exchange,
                MappingContext {
                    participant_id: ParticipantId::new(9),
                    next_intent_id: IntentId::new(10),
                },
            ),
            Err(MappingError::Dictionary(WireError::MissingRequiredTag(207)))
        );
        assert_eq!(order.quantity, QuantityLots::new(3));
        assert_eq!(
            order.kind,
            OrderKind::Limit {
                price: PriceTicks::new(101)
            }
        );
        assert!(!order.anonymous);
        let context = MappingContext {
            participant_id: ParticipantId::new(9),
            next_intent_id: IntentId::new(10),
        };
        let mut anonymous = message.clone();
        anonymous.push(10021, "Y");
        let InboundApplication::Intent(ExecutionIntent::Submit { order, .. }) =
            map_inbound(&anonymous, context)?
        else {
            return Err(MappingError::UnsupportedMessage);
        };
        assert!(order.anonymous);
        let mut invalid = message.clone();
        invalid.push(10021, "maybe");
        assert_eq!(
            map_inbound(&invalid, context),
            Err(MappingError::InvalidTag(10021))
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
            (207, "1"),
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
        for (value, expected) in [
            ("0", MarketDataRequestType::Snapshot),
            ("1", MarketDataRequestType::Subscribe),
            ("2", MarketDataRequestType::Unsubscribe),
        ] {
            market.fields.retain(|field| field.tag != 263);
            market.push(263, value);
            let InboundApplication::MarketDataRequest { request_type, .. } =
                map_inbound(&market, context)?
            else {
                return Err(MappingError::UnsupportedMessage);
            };
            assert_eq!(request_type, expected);
        }
        Ok(())
    }

    #[test]
    fn snapshot_and_incremental_messages_use_competition_group_layout() {
        let snapshot = market_snapshot(
            "book",
            ListingKey::new(VenueId::new(1), InstrumentId::new(7)),
            &[(PriceTicks::new(100), QuantityLots::new(2))],
            &[(PriceTicks::new(101), QuantityLots::new(3))],
        );
        assert_eq!(snapshot.value(268), Some("2"));
        let listing = ListingKey::new(VenueId::new(1), InstrumentId::new(7));
        let update = market_incremental(
            "book",
            5,
            &[
                (
                    listing,
                    MarketDataIncrement::Level {
                        action: MarketDataUpdateAction::Change,
                        side: Side::Buy,
                        price: PriceTicks::new(100),
                        quantity: QuantityLots::new(4),
                    },
                ),
                (
                    listing,
                    MarketDataIncrement::Trade {
                        price: PriceTicks::new(101),
                        quantity: QuantityLots::new(1),
                        reference: None,
                        buyer: None,
                        seller: None,
                    },
                ),
            ],
        );
        assert_eq!(update.msg_type, "X");
        assert_eq!(update.value(207), Some("1"));
        assert_eq!(update.value(268), Some("2"));
        assert_eq!(update.value(279), Some("1"));
        assert_eq!(update.value(83), Some("5"));
        let types: Vec<_> = update
            .fields
            .iter()
            .filter(|field| field.tag == 269 || field.tag == 83)
            .map(|field| field.value.as_str())
            .collect();
        assert_eq!(types, ["0", "5", "2", "6"]);
        // Public entries never carry party, order or account identity.
        for tag in [37, 448, 1, 11, 41, 278] {
            assert_eq!(update.value(tag), None);
        }
    }

    #[test]
    fn order_by_order_requests_snapshots_and_entries_carry_public_references()
    -> Result<(), MappingError> {
        let mut request = FixMessage::new("V");
        for (tag, value) in [
            (262, "l3"),
            (263, "1"),
            (264, "0"),
            (266, "N"),
            (267, "1"),
            (269, "1"),
            (48, "7"),
            (207, "2"),
        ] {
            request.push(tag, value);
        }
        let context = MappingContext {
            participant_id: ParticipantId::new(9),
            next_intent_id: IntentId::new(1),
        };
        assert!(matches!(
            map_inbound(&request, context)?,
            InboundApplication::MarketDataRequest {
                aggregated: false,
                ..
            }
        ));
        let mut invalid = request.clone();
        invalid.fields.retain(|field| field.tag != 266);
        invalid.push(266, "X");
        assert!(matches!(
            map_inbound(&invalid, context),
            Err(MappingError::InvalidTag(266))
        ));

        let listing = ListingKey::new(VenueId::new(2), InstrumentId::new(7));
        let snapshot = order_snapshot(
            "l3",
            listing,
            &[
                (Side::Buy, 4, 99, 2, Some(9)),
                (Side::Sell, 1, 101, 3, None),
                (Side::Sell, 3, 101, 1, Some(8)),
            ]
            .map(|(side, reference, price, quantity, broker)| SnapshotOrder {
                side,
                reference,
                price: PriceTicks::new(price),
                quantity: QuantityLots::new(quantity),
                broker: broker.map(ParticipantId::new),
            }),
        );
        assert_eq!(snapshot.value(288), Some("9"));
        assert_eq!(snapshot.value(289), Some("8"));
        let positions: Vec<_> = snapshot
            .fields
            .iter()
            .filter(|field| [278, 290].contains(&field.tag))
            .map(|field| field.value.as_str())
            .collect();
        assert_eq!(positions, ["4", "1", "1", "1", "3", "2"]);
        let update = market_incremental(
            "l3",
            1,
            &[
                (
                    listing,
                    MarketDataIncrement::Trade {
                        price: PriceTicks::new(101),
                        quantity: QuantityLots::new(1),
                        reference: Some(1),
                        buyer: Some(ParticipantId::new(9)),
                        seller: None,
                    },
                ),
                (
                    listing,
                    MarketDataIncrement::Order {
                        action: MarketDataUpdateAction::Change,
                        side: Side::Sell,
                        reference: 1,
                        price: PriceTicks::new(101),
                        quantity: QuantityLots::new(2),
                        broker: None,
                    },
                ),
            ],
        );
        let references: Vec<_> = update
            .fields
            .iter()
            .filter(|field| field.tag == 278)
            .map(|field| field.value.as_str())
            .collect();
        assert_eq!(references, ["1", "1"]);
        assert_eq!(update.value(288), Some("9"));
        assert_eq!(update.value(289), None, "anonymous seller");
        for tag in [37, 448, 1, 11, 41] {
            assert_eq!(update.value(tag), None);
        }
        Ok(())
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
