//! Built-in agents trade through the same admission sequencer as FIX teams
//! (ADR 0035, Step 4): the agent sits at its own location in the latency
//! map, and it learns of fills on its resting orders that teams cause.

mod support;

use bunting_admission_sequencer::{LatencyMap, Link, ParticipantPlacement, PathLatency};
use bunting_market_types::ParticipantId;
use bunting_server::config::{AdmissionConfig, ScenarioConfig, ServerConfig};
use simfix_wire::FixMessage;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT, order};

/// The local scenario's built-in agent.
const AGENT: u128 = 10;

/// Starts the venue with its built-in agent, an inventory-skewed market
/// maker quoting 5 lots two ticks either side of mid, placed 30 ms from the
/// venue.
fn start_server() -> Result<u16, String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let path = std::env::temp_dir().join(format!("bunting-agents-{port}.json"));
    std::fs::write(&path, include_str!("../config/scenario.json"))
        .map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.scenario = Some(ScenarioConfig {
        path: path.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let runtime = config.runtime.as_mut().ok_or("local profile has agents")?;
    runtime.wall_tick_ms = 20;
    runtime.scheduler.agents[0].kind =
        serde_json::from_value(serde_json::json!("inventory_skewed_market_maker"))
            .map_err(|error| error.to_string())?;
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.admission = AdmissionConfig::with_map(LatencyMap {
        default_location: "venue".to_owned(),
        participants: vec![ParticipantPlacement {
            participant_id: ParticipantId::new(AGENT),
            location: "agent_desk".to_owned(),
        }],
        links: vec![Link {
            between: ["agent_desk".to_owned(), "venue".to_owned()],
            latency: PathLatency {
                latency_us: 30_000,
                jitter_us: 0,
            },
        }],
        ..LatencyMap::default()
    });
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(port)
}

/// Bid and ask prices in one market-data snapshot.
fn levels(snapshot: &FixMessage) -> (Vec<i64>, Vec<i64>) {
    let (mut bids, mut asks) = (Vec::new(), Vec::new());
    let mut side = None;
    for field in &snapshot.fields {
        match field.tag {
            269 => side = Some(field.value.clone()),
            270 => {
                let price = field.value.parse::<f64>().unwrap_or(0.0);
                #[allow(clippy::cast_possible_truncation)]
                let price = price.round() as i64;
                match side.as_deref() {
                    Some("0") => bids.push(price),
                    Some("1") => asks.push(price),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    (bids, asks)
}

fn book(client: &mut Client, request: &mut u32) -> Result<(Vec<i64>, Vec<i64>), String> {
    *request += 1;
    let id = format!("book-{request}");
    let mut message = FixMessage::new("V");
    for (tag, value) in [
        (262, id.clone()),
        (263, "0".to_owned()),
        (264, "10".to_owned()),
        (267, "2".to_owned()),
        (269, "0".to_owned()),
        (269, "1".to_owned()),
        (48, "1".to_owned()),
        (207, "1".to_owned()),
    ] {
        message.push(tag, value);
    }
    client.send(message)?;
    let wanted = id.clone();
    client.pump_until(move |client| {
        client
            .received
            .iter()
            .any(|message| message.msg_type == "W" && message.value(262) == Some(&wanted))
    })?;
    client
        .received
        .iter()
        .rev()
        .find(|message| message.msg_type == "W" && message.value(262) == Some(&id))
        .map(levels)
        .ok_or_else(|| "snapshot missing".to_owned())
}

fn wait_for_book(
    client: &mut Client,
    request: &mut u32,
    done: impl Fn(&[i64], &[i64]) -> bool,
) -> Result<(Vec<i64>, Vec<i64>), String> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let (bids, asks) = book(client, request)?;
        if done(&bids, &asks) {
            return Ok((bids, asks));
        }
        if Instant::now() > deadline {
            return Err(format!("book never matched: bids {bids:?} asks {asks:?}"));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_built_in_agent_trades_through_the_sequencer_and_learns_of_team_fills() -> Result<(), String> {
    let port = start_server()?;
    let mut team = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut request = 0;
    // Flat, the agent quotes 98 / 102 around the 100 fundamental.
    let (_, asks) = wait_for_book(&mut team, &mut request, |bids, asks| {
        bids.contains(&98) && asks.contains(&102)
    })?;
    assert_eq!(asks, vec![102], "a flat agent does not skew");

    // The team lifts one of the agent's offers.
    team.send(order("701", "buy", 5, Some(102)))?;
    team.wait_report("701", "F")?;

    // Short one quote, the agent centres a tick higher and widens a tick
    // (new offers at 104), which it can only do if it learned of the fill
    // the team caused through the committed-event feed.
    wait_for_book(&mut team, &mut request, |_, asks| asks.contains(&104))?;
    Ok(())
}
