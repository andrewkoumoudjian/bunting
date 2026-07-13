#![expect(
    clippy::unwrap_used,
    reason = "conformance fixtures and setup must be valid test inputs"
)]

use bunting_market_events::Side;
use bunting_market_types::{PriceTicks, QuantityLots};
use nbc_matcher_oracle::{
    CancelOutcome, Fill, MAX_OPEN_ORDERS, MatchError, MatchOutcome, NbcOrderBook,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Fixture {
    reference: Reference,
    cases: Cases,
}
#[derive(Debug, Deserialize)]
struct Reference {
    classification: String,
    jar_sha256: String,
}
#[derive(Debug, Deserialize)]
struct Cases {
    price_time_partial_fill: PartialFill,
    open_order_limit: OpenOrderLimit,
}
#[derive(Debug, Deserialize)]
struct PartialFill {
    fill_order_ids: Vec<String>,
    fill_maker_flags: Vec<bool>,
    fill_quantities: Vec<i64>,
    fill_remaining: Vec<i64>,
    execution_price_ticks: Vec<i64>,
}
#[derive(Debug, Deserialize)]
struct OpenOrderLimit {
    maximum: usize,
    checked_before_matching: bool,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../../../tests/conformance/nbc/matching/jar-bytecode.expected.v1.json"
    ))
    .unwrap()
}
fn submit(
    book: &mut NbcOrderBook,
    order_id: &str,
    participant_id: &str,
    side: Side,
    price: i64,
    quantity: i64,
) -> Result<MatchOutcome, MatchError> {
    book.submit_limit(
        order_id,
        participant_id,
        side,
        PriceTicks::new(price),
        QuantityLots::new(quantity),
        false,
    )
}

#[test]
fn price_time_partial_fills_match_bytecode_fixture() {
    let expected = fixture();
    assert_eq!(expected.reference.classification, "bytecode_observed");
    assert_eq!(
        expected.reference.jar_sha256,
        "80afc2816970b2538dcaff808008bfebdce5426ac248c074859626605547e254"
    );
    let mut book = NbcOrderBook::new();
    submit(&mut book, "make-1", "maker-a", Side::Sell, 1000, 100).unwrap();
    submit(&mut book, "make-2", "maker-b", Side::Sell, 1000, 300).unwrap();
    let result = submit(&mut book, "take", "taker", Side::Buy, 1100, 300).unwrap();
    let fills = result.fills();
    assert_eq!(
        fills.iter().map(Fill::order_id).collect::<Vec<_>>(),
        expected
            .cases
            .price_time_partial_fill
            .fill_order_ids
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        fills.iter().map(Fill::is_maker).collect::<Vec<_>>(),
        expected.cases.price_time_partial_fill.fill_maker_flags
    );
    assert_eq!(
        fills
            .iter()
            .map(|fill| fill.quantity().get())
            .collect::<Vec<_>>(),
        expected.cases.price_time_partial_fill.fill_quantities
    );
    assert_eq!(
        fills
            .iter()
            .map(|fill| fill.remaining().get())
            .collect::<Vec<_>>(),
        expected.cases.price_time_partial_fill.fill_remaining
    );
    assert_eq!(
        fills
            .iter()
            .map(|fill| fill.price().get())
            .collect::<Vec<_>>(),
        expected.cases.price_time_partial_fill.execution_price_ticks
    );
    assert_eq!(book.open_order_count("maker-a"), 0);
    assert_eq!(book.open_order_count("maker-b"), 1);
}

#[test]
fn self_match_check_occurs_once_at_each_level_head() {
    let mut book = NbcOrderBook::new();
    submit(&mut book, "other", "other", Side::Sell, 1000, 100).unwrap();
    submit(&mut book, "own", "student", Side::Sell, 1000, 100).unwrap();
    let result = submit(&mut book, "take", "student", Side::Buy, 1000, 200).unwrap();
    assert_eq!(result.fills().len(), 4);
    assert_eq!(result.fills()[2].participant_id(), "student");
    assert_eq!(result.fills()[3].participant_id(), "student");
    submit(&mut book, "own-head", "student", Side::Sell, 1100, 100).unwrap();
    assert_eq!(
        submit(&mut book, "take-2", "student", Side::Buy, 1100, 100),
        Err(MatchError::SelfMatch)
    );
}

#[test]
fn cancel_is_id_only_and_unknown_is_a_noop() {
    let mut book = NbcOrderBook::new();
    submit(&mut book, "resting", "owner", Side::Buy, 900, 100).unwrap();
    assert!(matches!(book.cancel("resting"), CancelOutcome::Canceled(_)));
    assert_eq!(book.cancel("missing"), CancelOutcome::NotFound);
    assert_eq!(book.open_order_count("owner"), 0);
}

#[test]
fn hard_order_limit_is_checked_before_a_marketable_order() {
    let expected = fixture();
    assert_eq!(expected.cases.open_order_limit.maximum, MAX_OPEN_ORDERS);
    assert!(expected.cases.open_order_limit.checked_before_matching);
    let mut book = NbcOrderBook::new();
    submit(&mut book, "liquidity", "maker", Side::Sell, 1000, 100).unwrap();
    for index in 0..MAX_OPEN_ORDERS {
        submit(
            &mut book,
            &format!("rest-{index}"),
            "limited",
            Side::Buy,
            900,
            100,
        )
        .unwrap();
    }
    assert_eq!(
        submit(&mut book, "marketable", "limited", Side::Buy, 1000, 100),
        Err(MatchError::OrderLimitExceeded)
    );
}

#[test]
fn checked_external_units_reject_java_edge_cases() {
    let mut book = NbcOrderBook::new();
    assert_eq!(
        submit(&mut book, "zero", "student", Side::Buy, 1000, 0),
        Err(MatchError::InvalidQuantity)
    );
    assert_eq!(
        submit(&mut book, "odd", "student", Side::Buy, 1000, 50),
        Err(MatchError::InvalidQuantity)
    );
    assert_eq!(
        submit(&mut book, "price", "student", Side::Buy, 0, 100),
        Err(MatchError::InvalidPrice)
    );
}
