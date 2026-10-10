//! ADR 0035 end to end: in a multi-venue market each team's virtual
//! distance to each venue decides, on top of its real connection, when its
//! orders reach that venue's book and when that venue's data reaches the
//! team. Two venues listing the same instrument; team 1 sits 1 ms from
//! venue 1 and 40 ms from venue 2, team 2 the reverse.

mod support;

use bunting_admission_sequencer::{LatencyPolicy, PathEntry, PathLatency};
use bunting_api_contract::ActorRole;
use bunting_market_types::{ParticipantId, VenueId};
use bunting_server::config::{AdmissionConfig, RosterEntry, ScenarioConfig, ServerConfig};
use simfix_wire::FixMessage;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT};

const NEAR_US: u64 = 1_000;
const FAR_US: u64 = 40_000;

fn path(participant: u128, venue: u128, latency_us: u64) -> PathEntry {
    PathEntry {
        participant_id: ParticipantId::new(participant),
        venue_id: Some(VenueId::new(venue)),
        path: PathLatency {
            latency_us,
            jitter_us: 0,
        },
    }
}

fn start_server() -> Result<u16, String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let mut scenario: serde_json::Value =
        serde_json::from_str(include_str!("../config/scenario.json"))
            .map_err(|error| error.to_string())?;
    // The same instrument listed on a second venue.
    let mut second = scenario["listings"]["1:1"].clone();
    second["key"] = serde_json::json!("2:1");
    second["symbol"] = serde_json::json!("BNT.B");
    scenario["listings"]["2:1"] = second;
    let path_file = std::env::temp_dir().join(format!("bunting-venues-{port}.json"));
    std::fs::write(&path_file, scenario.to_string()).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: path_file.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.roster.push(RosterEntry {
        target_comp_id: "MAKER".to_owned(),
        username: "maker".to_owned(),
        password: "bunting-maker-dev".to_owned(),
        role: ActorRole::Participant,
        participant_id: 10,
    });
    fix.max_connections = 3;
    fix.admission = AdmissionConfig::with_policy(LatencyPolicy {
        paths: vec![
            path(1, 1, NEAR_US),
            path(1, 2, FAR_US),
            path(2, 1, FAR_US),
            path(2, 2, NEAR_US),
        ],
        ..LatencyPolicy::default()
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

fn order_at(id: &str, side: &str, price: i64, venue: u128) -> FixMessage {
    let mut message = FixMessage::new("D");
    message.push(11, id);
    message.push(48, "1");
    message.push(207, venue.to_string());
    message.push(54, if side == "buy" { "1" } else { "2" });
    message.push(38, "1");
    message.push(40, "2");
    message.push(44, price.to_string());
    message
}

fn book_request(id: &str, venue: u128) -> FixMessage {
    let mut message = FixMessage::new("V");
    for (tag, value) in [
        (262, id.to_owned()),
        (263, "0".to_owned()),
        (264, "5".to_owned()),
        (267, "2".to_owned()),
        (269, "0".to_owned()),
        (269, "1".to_owned()),
        (48, "1".to_owned()),
        (207, venue.to_string()),
    ] {
        message.push(tag, value);
    }
    message
}

fn snapshot_received(client: &Client, id: &str) -> bool {
    client
        .received
        .iter()
        .any(|message| message.msg_type == "W" && message.value(262) == Some(id))
}

/// Keeps both clients reading continuously, as real clients do.
fn pump_both(first: &mut Client, second: &mut Client, duration: Duration) -> Result<(), String> {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        let step = Instant::now() + Duration::from_micros(500);
        first.pump_until(|_| Instant::now() >= step)?;
        let step = Instant::now() + Duration::from_micros(500);
        second.pump_until(|_| Instant::now() >= step)?;
    }
    Ok(())
}

#[test]
fn the_team_nearer_a_venue_wins_there_even_when_it_sends_later() -> Result<(), String> {
    let port = start_server()?;
    let mut maker = Client::logon(port, "MAKER", "maker", "bunting-maker-dev")?;
    maker.send(order_at("901", "sell", 100, 1))?;
    maker.wait_report("901", "0")?;
    maker.send(order_at("902", "sell", 100, 2))?;
    maker.wait_report("902", "0")?;

    let mut team1 = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut team2 = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    for client in [&team1, &team2] {
        client
            .stream
            .set_read_timeout(Some(Duration::from_micros(500)))
            .map_err(|error| error.to_string())?;
    }
    pump_both(&mut team1, &mut team2, Duration::from_millis(200))?;

    // Venue 2's last lot: team 1 (40 ms away) sends 10 ms before team 2
    // (1 ms away). Venue 1's last lot: the mirror image.
    team1.send(order_at("101", "buy", 100, 2))?;
    team2.send(order_at("201", "buy", 100, 1))?;
    std::thread::sleep(Duration::from_millis(10));
    team2.send(order_at("202", "buy", 100, 2))?;
    team1.send(order_at("102", "buy", 100, 1))?;

    let deadline = Instant::now() + TIMEOUT;
    while ["101", "102"]
        .iter()
        .any(|id| team1.reports_for(id).is_empty())
        || ["201", "202"]
            .iter()
            .any(|id| team2.reports_for(id).is_empty())
    {
        if Instant::now() > deadline {
            return Err("orders were not acknowledged".to_owned());
        }
        pump_both(&mut team1, &mut team2, Duration::from_millis(5))?;
    }
    pump_both(&mut team1, &mut team2, Duration::from_millis(150))?;
    let filled = |client: &Client, id: &str| client.has_report(id, "F");
    assert!(
        filled(&team2, "202") && !filled(&team1, "101"),
        "venue 2: the team 1 ms away wins though it sent 10 ms later"
    );
    assert!(
        filled(&team1, "102") && !filled(&team2, "201"),
        "venue 1: the team 1 ms away wins though it sent 10 ms later"
    );
    Ok(())
}

#[test]
fn a_far_venues_data_arrives_later_by_the_round_trip_difference() -> Result<(), String> {
    let port = start_server()?;
    let mut team1 = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let elapsed = |client: &mut Client, id: &str, venue: u128| -> Result<Duration, String> {
        let started = Instant::now();
        client.send(book_request(id, venue))?;
        client.pump_until(|client| snapshot_received(client, id))?;
        Ok(started.elapsed())
    };
    let near = elapsed(&mut team1, "near", 1)?;
    let far = elapsed(&mut team1, "far", 2)?;
    // Request out and snapshot back each cross the virtual path: 2 × 39 ms.
    let difference = far.saturating_sub(near);
    assert!(
        difference >= Duration::from_millis(70) && difference <= Duration::from_millis(120),
        "venue 2 data should take ~78 ms longer than venue 1 data, took {difference:?} ({near:?} vs {far:?})"
    );
    Ok(())
}
