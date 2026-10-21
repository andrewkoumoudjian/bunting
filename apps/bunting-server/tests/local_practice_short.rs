//! The local practice scenario (`bunting-trading-state`'s `--local` venue)
//! lets a flat participant sell short within its risk limits and refuses
//! the order that would pass them (RIT cases start flat and allow shorts).

use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_events::{Command, CommandPayload, OrderKind, Side, SubmitOrder};
use bunting_market_types::{
    CommandId, CorrelationId, InstrumentId, IterationId, ListingKey, LogicalTimeNs, OrderId,
    ParticipantId, PriceTicks, QuantityLots, RunId, VenueId,
};

const SCENARIO: &str =
    include_str!("../../../packages/bunting-trading-state/src/local/scenario.json");

fn order(
    state: &RunState,
    n: u128,
    who: u128,
    side: Side,
    instrument: u128,
    quantity: i64,
) -> Command {
    Command {
        run_id: state.run_id(),
        command_id: CommandId::new(n),
        correlation_id: CorrelationId::new(n),
        logical_time: LogicalTimeNs::new(0),
        expected_sequence: state.sequence(),
        actor: ParticipantId::new(who),
        payload: CommandPayload::SubmitOrderAtListing {
            listing_key: ListingKey::new(VenueId::new(1), InstrumentId::new(instrument)),
            order: SubmitOrder {
                order_id: OrderId::new(n),
                instrument_id: InstrumentId::new(instrument),
                participant_id: ParticipantId::new(who),
                side,
                quantity: QuantityLots::new(quantity),
                kind: OrderKind::Limit {
                    price: PriceTicks::new(100),
                },
                anonymous: false,
            },
        },
    }
}

#[test]
fn a_flat_practice_participant_sells_short_up_to_its_limit_and_no_further() -> Result<(), String> {
    let scenario: ScenarioDefinition = serde_json::from_str(SCENARIO).map_err(|e| e.to_string())?;
    let limits = scenario.participants()[&ParticipantId::new(2)]
        .limits()
        .clone();
    assert_eq!(limits.max_absolute_position, QuantityLots::new(1_000_000));
    for instrument in [1_u128, 2] {
        let mut state = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)
            .map_err(|e| format!("{e:?}"))?;
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(2), InstrumentId::new(instrument))
                .quantity,
            QuantityLots::new(0),
            "practice participants start flat"
        );
        // Ten sales of 100 000, each taken by participant 1, leave
        // participant 2 short exactly at the 1 000 000 limit.
        let per_order = limits.max_order_quantity.get();
        let mut n = 1_u128;
        let mut total = 0;
        while total < limits.max_absolute_position.get() {
            let sale = order(&state, n, 2, Side::Sell, instrument, per_order);
            let applied = state.apply(&sale).map_err(|e| format!("{e:?}"))?;
            assert!(
                applied.accepted,
                "short sale {n} of {per_order} on instrument {instrument} refused: {:?}",
                applied.reject_code
            );
            let purchase = order(&state, n + 1, 1, Side::Buy, instrument, per_order);
            let applied = state.apply(&purchase).map_err(|e| format!("{e:?}"))?;
            assert!(applied.accepted, "{:?}", applied.reject_code);
            total += per_order;
            n += 2;
        }
        // One lot past the limit is refused.
        assert_eq!(
            state
                .ledger()
                .position(ParticipantId::new(2), InstrumentId::new(instrument))
                .quantity,
            QuantityLots::new(-1_000_000)
        );
        let command = order(&state, n, 2, Side::Sell, instrument, 1);
        let applied = state.apply(&command).map_err(|e| format!("{e:?}"))?;
        assert!(
            !applied.accepted,
            "a sale past the position limit was accepted"
        );
        assert_eq!(applied.reject_code.as_deref(), Some("PositionLimit"));
    }
    Ok(())
}
