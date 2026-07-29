use bunting_engine::{RunState, ScenarioDefinition};
use bunting_market_events::{SimulationCommand, SimulationCommandRequest};
use bunting_market_types::{
    CommandId, CorrelationId, IterationId, LogicalTimeNs, ParticipantId, RunId,
};
use bunting_rs::{ArchivePolicy, COMPETITION_ARCHIVE_VERSION, CompetitionArchive};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let scenario: ScenarioDefinition = serde_json::from_str(include_str!(
        "../../apps/bunting-server/config/scenario.json"
    ))?;
    let initial = RunState::from_scenario(RunId::new(1), IterationId::new(1), &scenario)?;
    let command = SimulationCommandRequest {
        run_id: initial.run_id(),
        command_id: CommandId::new(1),
        correlation_id: CorrelationId::new(1),
        logical_time: LogicalTimeNs::new(0),
        expected_sequence: initial.sequence(),
        actor: ParticipantId::new(1),
        payload: SimulationCommand::StartRun,
    };
    let outcome = initial.transition_simulation(&command)?;
    let archive = CompetitionArchive {
        schema_version: COMPETITION_ARCHIVE_VERSION,
        scenario_id: "1".to_owned(),
        scenario_version: "1".to_owned(),
        engine_version: env!("CARGO_PKG_VERSION").to_owned(),
        seeds: vec![42],
        policy: ArchivePolicy {
            matching_interval_ms: 100,
            max_messages_per_interval: 64,
            max_open_orders: 256,
            reconnect_resting_orders: true,
        },
        initial: initial
            .snapshot_envelope()
            .map_err(|error| std::io::Error::other(format!("{error:?}")))?,
        accepted_commands: vec![command],
        canonical_events: outcome.events,
        final_state_hash: outcome
            .candidate
            .state_hash()
            .map_err(|error| std::io::Error::other(format!("{error:?}")))?,
    };
    println!("{}", archive.to_json()?);
    Ok(())
}
