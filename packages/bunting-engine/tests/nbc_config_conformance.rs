use bunting_engine::compatibility::nbc::{ConfigError, ExactDecimal, ScenarioConfig};
use serde::Deserialize;

const INPUT: &[u8] =
    include_bytes!("../../../tests/conformance/nbc/config/normal-market.input.v1.json");
const EXPECTED: &[u8] =
    include_bytes!("../../../tests/conformance/nbc/config/normal-market.expected.v1.json");

#[derive(Debug, Deserialize)]
struct Expected {
    scenario_id: String,
    duration_steps: u32,
    step_interval_ms: u32,
    tick_size: String,
    deterministic_config_sha256: String,
    deterministic_provenance_sha256: String,
}

#[test]
fn parses_jar_linked_normal_market_fixture() -> Result<(), Box<dyn std::error::Error>> {
    let config = ScenarioConfig::from_json(INPUT)?;
    let expected: Expected = serde_json::from_slice(EXPECTED)?;

    assert_eq!(config.scenario_id, expected.scenario_id);
    assert_eq!(config.duration_steps.get(), expected.duration_steps);
    assert_eq!(config.step_interval_ms.get(), expected.step_interval_ms);
    assert_eq!(config.market.tick_size.as_str(), expected.tick_size);
    assert_eq!(
        config.deterministic_hash()?.as_str(),
        expected.deterministic_config_sha256
    );
    assert_eq!(
        config.provenance_hash()?.as_str(),
        expected.deterministic_provenance_sha256
    );
    Ok(())
}

#[test]
fn rejects_unknown_fields_before_execution() -> Result<(), Box<dyn std::error::Error>> {
    let mut value: serde_json::Value = serde_json::from_slice(INPUT)?;
    value["surprise"] = serde_json::json!(true);
    let bytes = serde_json::to_vec(&value)?;
    assert!(matches!(
        ScenarioConfig::from_json(&bytes),
        Err(ConfigError::InvalidJson(_))
    ));
    Ok(())
}

#[test]
fn rejects_zero_time_and_noncanonical_decimals() -> Result<(), Box<dyn std::error::Error>> {
    let mut value: serde_json::Value = serde_json::from_slice(INPUT)?;
    value["duration_steps"] = serde_json::json!(0);
    let bytes = serde_json::to_vec(&value)?;
    assert!(ScenarioConfig::from_json(&bytes).is_err());

    assert_eq!(
        ExactDecimal::parse("0.50"),
        Err(ConfigError::InvalidDecimal("0.50".to_owned()))
    );
    assert!(ExactDecimal::parse("0.5").is_ok());
    Ok(())
}

#[test]
fn hash_is_independent_of_json_layout() -> Result<(), Box<dyn std::error::Error>> {
    let compact = serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(INPUT)?)?;
    let first = ScenarioConfig::from_json(INPUT)?.deterministic_hash()?;
    let second = ScenarioConfig::from_json(&compact)?.deterministic_hash()?;
    assert_eq!(first, second);
    Ok(())
}
