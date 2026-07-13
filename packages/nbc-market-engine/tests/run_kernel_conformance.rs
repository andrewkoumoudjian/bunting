use bunting_nbc_market_engine::{
    KernelError, RunKernel, RunStatus, ScenarioConfig, ScheduledEvent,
};
use serde::Deserialize;

const CONFIG: &[u8] =
    include_bytes!("../../../tests/conformance/nbc/config/normal-market.input.v1.json");
const INPUT: &[u8] =
    include_bytes!("../../../tests/conformance/nbc/run-kernel/jar-observed.input.v1.json");
const EXPECTED: &[u8] =
    include_bytes!("../../../tests/conformance/nbc/run-kernel/jar-observed.expected.v1.json");

#[derive(Debug, Deserialize)]
struct FixtureInput {
    run_id: String,
    duration_steps: u32,
    events: Vec<FixtureEvent>,
}

#[derive(Debug, Deserialize)]
struct FixtureEvent {
    id: String,
    trigger_step: u32,
}

#[derive(Debug, Deserialize)]
struct Expected {
    initial_step: u32,
    advances: Vec<ExpectedAdvance>,
}

#[derive(Debug, Deserialize)]
struct ExpectedAdvance {
    executed_step: u32,
    current_step: u32,
    triggered_event_ids: Vec<String>,
    status: String,
}

#[test]
fn matches_jar_observed_lifecycle_and_event_order() -> Result<(), Box<dyn std::error::Error>> {
    let input: FixtureInput = serde_json::from_slice(INPUT)?;
    let expected: Expected = serde_json::from_slice(EXPECTED)?;
    let mut config_value: serde_json::Value = serde_json::from_slice(CONFIG)?;
    config_value["duration_steps"] = input.duration_steps.into();
    let config = ScenarioConfig::from_json(&serde_json::to_vec(&config_value)?)?;
    let events = input
        .events
        .into_iter()
        .map(|event| ScheduledEvent::new(event.id, event.trigger_step))
        .collect::<Result<Vec<_>, _>>()?;
    let mut run = RunKernel::start(input.run_id, config, events)?;

    assert_eq!(run.current_step(), expected.initial_step);
    for expected_advance in expected.advances {
        let actual = run.advance()?;
        assert_eq!(actual.executed_step(), expected_advance.executed_step);
        assert_eq!(actual.current_step(), expected_advance.current_step);
        assert_eq!(
            actual.triggered_event_ids(),
            expected_advance.triggered_event_ids
        );
        let expected_status = match expected_advance.status.as_str() {
            "active" => RunStatus::Active,
            "completed" => RunStatus::Completed,
            value => return Err(format!("unsupported fixture status: {value}").into()),
        };
        assert_eq!(actual.status(), &expected_status);
    }
    assert_eq!(run.advance(), Err(KernelError::RunNotActive));
    Ok(())
}

#[test]
fn rejects_unreachable_events_and_bounds_termination() -> Result<(), Box<dyn std::error::Error>> {
    let mut config_value: serde_json::Value = serde_json::from_slice(CONFIG)?;
    config_value["duration_steps"] = 2.into();
    let config = ScenarioConfig::from_json(&serde_json::to_vec(&config_value)?)?;
    let unreachable = ScheduledEvent::new("unreachable", 2)?;
    assert!(matches!(
        RunKernel::start("run", config, vec![unreachable]),
        Err(KernelError::EventOutsideRun { .. })
    ));

    let config = ScenarioConfig::from_json(CONFIG)?;
    let mut run = RunKernel::start("run", config, Vec::new())?;
    assert_eq!(run.terminate("manual stop"), Ok(()));
    assert_eq!(
        run.status(),
        &RunStatus::Terminated {
            reason: "manual stop".to_owned()
        }
    );
    assert_eq!(run.advance(), Err(KernelError::RunNotActive));
    Ok(())
}

#[test]
fn legacy_parameters_remain_inert() -> Result<(), Box<dyn std::error::Error>> {
    let baseline = ScenarioConfig::from_json(CONFIG)?;
    let mut changed_value: serde_json::Value = serde_json::from_slice(CONFIG)?;
    changed_value["legacy_parameters"]["specialEvents"] = serde_json::json!([{
        "type": "UNTRANSLATED_EVENT",
        "triggerStep": 0
    }]);
    let changed = ScenarioConfig::from_json(&serde_json::to_vec(&changed_value)?)?;
    let mut baseline_run = RunKernel::start("baseline", baseline, Vec::new())?;
    let mut changed_run = RunKernel::start("changed", changed, Vec::new())?;

    assert_eq!(baseline_run.advance()?, changed_run.advance()?);
    Ok(())
}
