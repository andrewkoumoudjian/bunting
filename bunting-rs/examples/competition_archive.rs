use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_events::{
    Command, CommandPayload, OrderKind, Side, SimulationCommand, SimulationCommandRequest,
    SubmitOrder,
};
use bunting_market_types::{
    CommandId, CorrelationId, EventSequence, InstrumentId, IterationId, LogicalTimeNs, OrderId,
    ParticipantId, PriceTicks, QuantityLots, RunId,
};
use bunting_origin_store::{Execution, JournalInput, LiveRun, RunLimits};
use bunting_rs::CompetitionArchive;

fn order(sequence: u64, id: u128, participant: u128, side: Side) -> JournalInput {
    JournalInput::Command(Command {
        run_id: RunId::new(1),
        command_id: CommandId::new(id),
        correlation_id: CorrelationId::new(id),
        logical_time: LogicalTimeNs::new(0),
        expected_sequence: EventSequence::new(sequence),
        actor: ParticipantId::new(participant),
        payload: CommandPayload::SubmitOrder(SubmitOrder {
            order_id: OrderId::new(id),
            instrument_id: InstrumentId::new(1),
            participant_id: ParticipantId::new(participant),
            side,
            quantity: QuantityLots::new(5),
            kind: OrderKind::Limit {
                price: PriceTicks::new(100),
            },
            anonymous: false,
        }),
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario: ScenarioDefinition = serde_json::from_str(include_str!(
        "../../apps/bunting-server/config/scenario.json"
    ))?;
    let genesis = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)?;
    let envelope = genesis
        .snapshot_envelope()
        .map_err(|error| std::io::Error::other(format!("{error:?}")))?;
    let mut live = LiveRun::genesis(genesis, RunLimits::default())
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    // A trade, then a simulation command: order flow and administration
    // share one journal, and the archive replays both.
    let inputs = [
        order(0, 1, 10, Side::Sell),
        order(1, 2, 1, Side::Buy),
        JournalInput::Simulation(SimulationCommandRequest {
            run_id: RunId::new(1),
            command_id: CommandId::new(3),
            correlation_id: CorrelationId::new(3),
            logical_time: LogicalTimeNs::new(0),
            expected_sequence: EventSequence::new(2),
            actor: ParticipantId::new(1),
            payload: SimulationCommand::PauseRun,
        }),
    ];
    let mut records = Vec::new();
    for input in &inputs {
        match live
            .execute(input)
            .map_err(|error| std::io::Error::other(error.to_string()))?
        {
            Execution::Committed(record) => records.push(*record),
            Execution::Duplicate(_) => return Err("unexpected duplicate".into()),
        }
    }
    let archive = CompetitionArchive::from_journal(envelope, records)?;
    println!("{}", archive.to_json()?);
    Ok(())
}
