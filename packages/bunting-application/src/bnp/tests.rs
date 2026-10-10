#![allow(clippy::unwrap_used)]

use super::*;
use bnp_wire::FeedFlags;
use bunting_api_contract::{ActorIdentity, ActorRole, UnsignedDecimalString};
use bunting_engine::{
    InstrumentDefinition, InstrumentKind, ListingDefinition, ParticipantDefinition,
    ScenarioDefinition,
};
use bunting_market_types::{
    CurrencyId, IterationId, MoneyMinor, PriceBounds, ScenarioId, ScenarioVersion,
};
use bunting_risk_engine::RiskLimits;

const BUYER: ParticipantId = ParticipantId::new(7);
const SELLER: ParticipantId = ParticipantId::new(8);

fn listing() -> Listing {
    Listing {
        venue_id: 1,
        instrument_id: 1,
    }
}

fn run() -> RunState {
    let participant = |id, positions| {
        ParticipantDefinition::new(
            id,
            true,
            RiskLimits::new(
                QuantityLots::new(100),
                QuantityLots::new(1_000),
                QuantityLots::new(1_000),
            ),
            BTreeMap::from([(CurrencyId::new(1), MoneyMinor::new(100_000))]),
            positions,
        )
    };
    let scenario = ScenarioDefinition::new(
        ScenarioId::new(1),
        ScenarioVersion::new(1),
        [InstrumentDefinition::new(
            InstrumentId::new(1),
            "BNT",
            CurrencyId::new(1),
            InstrumentKind::Equity,
        )
        .with_opening_mark(PriceTicks::new(100))],
        [ListingDefinition::new(
            ListingKey::new(VenueId::new(1), InstrumentId::new(1)),
            "ONE".to_owned(),
            PriceBounds::new(PriceTicks::new(1), PriceTicks::new(1_000)).unwrap(),
        )
        .unwrap()],
        [
            participant(BUYER, BTreeMap::new()),
            participant(
                SELLER,
                BTreeMap::from([(InstrumentId::new(1), QuantityLots::new(50))]),
            ),
        ],
    )
    .unwrap();
    RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario).unwrap()
}

fn limit(client_order_id: u64, side: WireSide, quantity: i64, price: i64) -> NewOrder {
    NewOrder {
        client_order_id,
        listing: listing(),
        side,
        quantity,
        order_type: OrderType::Limit { price },
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        display_quantity: None,
    }
}

/// Applies `command` as the sequencer would: stamped with the run's
/// sequence and a logical time.
fn apply(state: &mut RunState, mut command: Command) -> Vec<EventEnvelope> {
    command.expected_sequence = state.sequence();
    command.logical_time = LogicalTimeNs::new(state.sequence().get() + 1);
    state.apply(&command).unwrap().events
}

#[test]
fn identities_are_namespaced_idempotent_and_reversible() {
    let buyer = BnpIdentity::new(RunId::new(1), BUYER);
    let seller = BnpIdentity::new(RunId::new(1), SELLER);
    let other_run = BnpIdentity::new(RunId::new(2), BUYER);
    let order = limit(5, WireSide::Buy, 1, 100);
    let first = buyer.new_order(&order, CorrelationId::new(1)).unwrap();
    let again = buyer.new_order(&order, CorrelationId::new(2)).unwrap();
    assert_eq!(first.command_id, again.command_id);
    assert_ne!(
        first.command_id,
        seller
            .new_order(&order, CorrelationId::new(1))
            .unwrap()
            .command_id
    );
    let order_id = buyer.order_id(5).unwrap();
    assert_ne!(order_id, seller.order_id(5).unwrap());
    assert_ne!(order_id, other_run.order_id(5).unwrap());
    assert_eq!(buyer.client_order_id(order_id), 5);
    assert_eq!(seller.client_order_id(order_id), 0);
    // Cancels and new orders of the same client ID never share a command.
    assert_ne!(
        buyer.cancel(5, CorrelationId::new(1)).unwrap().command_id,
        first.command_id
    );
    assert_eq!(buyer.order_id(0), Err(BnpMappingError::ZeroIdentifier));
    assert!(buyer.kill_switch(0, CorrelationId::new(1)).is_err());
}

#[test]
fn order_kinds_follow_the_protocol() {
    let identity = BnpIdentity::new(RunId::new(1), BUYER);
    let kind = |order: &NewOrder| match identity
        .new_order(order, CorrelationId::new(1))
        .map(|command| command.payload)
    {
        Ok(CommandPayload::SubmitOrderAtListing { order, .. }) => Ok(order.kind),
        Ok(_) => unreachable!("new orders are listing submissions"),
        Err(error) => Err(error),
    };
    let plain = limit(1, WireSide::Buy, 10, 100);
    assert_eq!(
        kind(&plain),
        Ok(OrderKind::Limit {
            price: PriceTicks::new(100)
        })
    );
    let iceberg = NewOrder {
        display_quantity: Some(2),
        time_in_force: TimeInForce::Day,
        ..plain.clone()
    };
    assert_eq!(
        kind(&iceberg),
        Ok(OrderKind::LimitWithPolicy {
            price: PriceTicks::new(100),
            time_in_force: TimeInForcePolicy::Day,
            post_only: false,
            display_quantity: Some(QuantityLots::new(2)),
        })
    );
    let market = NewOrder {
        order_type: OrderType::Market,
        time_in_force: TimeInForce::Ioc,
        ..plain.clone()
    };
    assert_eq!(kind(&market), Ok(OrderKind::Market));
    assert_eq!(
        kind(&NewOrder {
            time_in_force: TimeInForce::Gtc,
            ..market
        }),
        Err(BnpMappingError::InvalidMarketOrder)
    );
    assert_eq!(
        kind(&NewOrder {
            display_quantity: Some(11),
            ..plain
        }),
        Err(BnpMappingError::InvalidDisplayQuantity)
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one lifecycle from rest to cancel reject"
)]
fn committed_events_become_each_sides_private_messages() {
    let mut state = run();
    let buyer = BnpIdentity::new(RunId::new(1), BUYER);
    let seller = BnpIdentity::new(RunId::new(1), SELLER);
    let resting = apply(
        &mut state,
        seller
            .new_order(&limit(1, WireSide::Sell, 5, 100), CorrelationId::new(1))
            .unwrap(),
    );
    let seller_messages = seller.private_messages(&resting);
    assert!(buyer.private_messages(&resting).is_empty());
    assert!(matches!(
        seller_messages.as_slice(),
        [
            ServerMessage::OrderAccepted {
                client_order_id: 1,
                quantity: 5,
                price: Some(100),
                side: WireSide::Sell,
                ..
            },
            ServerMessage::OrderRested {
                client_order_id: 1,
                remaining: 5,
                ..
            }
        ]
    ));
    let trade = apply(
        &mut state,
        buyer
            .new_order(&limit(9, WireSide::Buy, 2, 100), CorrelationId::new(2))
            .unwrap(),
    );
    let buyer_messages = buyer.private_messages(&trade);
    let seller_messages = seller.private_messages(&trade);
    let fill = |messages: &[ServerMessage]| {
        messages
            .iter()
            .find_map(|message| match message {
                ServerMessage::Fill {
                    client_order_id,
                    side,
                    quantity,
                    liquidity,
                    ..
                } => Some((*client_order_id, *side, *quantity, *liquidity)),
                _ => None,
            })
            .unwrap()
    };
    assert_eq!(
        fill(&buyer_messages),
        (9, WireSide::Buy, 2, Liquidity::Taker)
    );
    assert_eq!(
        fill(&seller_messages),
        (1, WireSide::Sell, 2, Liquidity::Maker)
    );
    assert!(buyer_messages.iter().any(|message| matches!(
        message,
        ServerMessage::OrderDone {
            client_order_id: 9,
            ..
        }
    )));
    assert!(seller_messages.iter().any(|message| matches!(
        message,
        ServerMessage::OrderReduced {
            client_order_id: 1,
            remaining: 3,
            ..
        }
    )));
    // Sequences are the committed events' own, so a cursor resumes exactly.
    let sequences: Vec<u64> = buyer_messages
        .iter()
        .filter_map(|message| message.stamp().map(|stamp| stamp.sequence))
        .collect();
    assert!(sequences.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(sequences[0] > resting.last().unwrap().sequence.get());

    assert_eq!(seller.open_orders(&state).len(), 1);
    assert_eq!(seller.open_orders(&state)[0].remaining_quantity, 3);
    assert!(buyer.open_orders(&state).is_empty());

    let canceled = apply(&mut state, seller.cancel(1, CorrelationId::new(3)).unwrap());
    assert!(matches!(
        seller.private_messages(&canceled).as_slice(),
        [ServerMessage::OrderCanceled {
            client_order_id: 1,
            remaining: 3,
            reason: WireCancelReason::Requested,
            ..
        }]
    ));
    let again = apply(&mut state, seller.cancel(2, CorrelationId::new(4)).unwrap());
    assert!(matches!(
        seller.private_messages(&again).as_slice(),
        [ServerMessage::CancelRejected {
            reason: RejectReason::UnknownOrder,
            ..
        }]
    ));
    assert!(buyer.private_messages(&again).is_empty());
}

#[test]
fn listings_and_balances_project_committed_state() {
    let state = run();
    assert_eq!(
        listings(&state),
        vec![ListingInfo {
            listing: listing(),
            symbol: "ONE".to_owned(),
        }]
    );
    let actor = VerifiedActor::try_from_identity(ActorIdentity {
        actor_id: UnsignedDecimalString::new(8),
        role: ActorRole::Participant,
        participant_id: Some(UnsignedDecimalString::new(8)),
        team_id: None,
    })
    .unwrap();
    let (cash, positions) = account_balances(&state, &actor).unwrap();
    assert_eq!(cash[0].balance, 100_000);
    assert_eq!(positions[0].position, 50);
    // Unused import guard: feed flags are part of the same protocol.
    assert!(FeedFlags::ALL.contains(FeedFlags::TRADES));
}
