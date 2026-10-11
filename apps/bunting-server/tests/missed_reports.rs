//! Reports a FIX session never wrote reach the participant's next session:
//! those still on their virtual path when the connection closed, including
//! one overtaken by a later report from a nearer venue, and those committed
//! while the participant was away. Reports already written are not sent
//! again (FIX resend recovers those).

mod support;

use bunting_admission_sequencer::{
    LatencyMap, Link, ParticipantPlacement, PathLatency, VenuePlacement,
};
use bunting_api_contract::ActorRole;
use bunting_market_types::{ParticipantId, VenueId};
use bunting_server::config::{
    AdmissionConfig, RosterEntry, ScenarioConfig, ServerConfig, StorageKind,
};
use simfix_wire::FixMessage;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT};

/// Venue 2 is this far from team 1 and the maker; venue 1 is 1 ms away.
const FAR_US: u64 = 1_000_000;

fn latency(latency_us: u64) -> PathLatency {
    PathLatency {
        latency_us,
        jitter_us: 0,
    }
}

fn map() -> LatencyMap {
    let at = |location: &str| location.to_owned();
    LatencyMap {
        default_location: at("here"),
        participants: vec![ParticipantPlacement {
            participant_id: ParticipantId::new(1),
            location: at("here"),
        }],
        venues: vec![
            VenuePlacement {
                venue_id: VenueId::new(1),
                location: at("here"),
            },
            VenuePlacement {
                venue_id: VenueId::new(2),
                location: at("far"),
            },
        ],
        local: latency(1_000),
        links: vec![Link {
            between: [at("here"), at("far")],
            latency: latency(FAR_US),
        }],
        ..LatencyMap::default()
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
    let mut second = scenario["listings"]["1:1"].clone();
    second["key"] = serde_json::json!("2:1");
    second["symbol"] = serde_json::json!("BNT.B");
    scenario["listings"]["2:1"] = second;
    scenario["participants"]["1"]["initial_positions"] = serde_json::json!({ "1": 1_000 });
    let directory = std::env::temp_dir().join(format!("bunting-missed-reports-{port}"));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let scenario_path = directory.join("scenario.json");
    std::fs::write(&scenario_path, scenario.to_string()).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    // Durable storage, as hosted: the FIX session and its client-order
    // mapping survive the reconnect.
    config.storage.kind = StorageKind::File;
    config.storage.path = Some(directory.join("origin.json").display().to_string());
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: scenario_path.display().to_string(),
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
    fix.admission = AdmissionConfig::with_map(map());
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

fn fills(client: &Client, id: &str) -> usize {
    client
        .reports_for(id)
        .iter()
        .filter(|message| message.value(150) == Some("F"))
        .count()
}

#[test]
fn a_reconnecting_session_gets_every_report_its_last_one_never_wrote() -> Result<(), String> {
    let port = start_server()?;
    let mut team = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut maker = Client::logon(port, "MAKER", "maker", "bunting-maker-dev")?;
    // Team 1 offers one lot at the far venue (101) and two at the near one
    // (102, 103).
    team.send(order_at("101", "sell", 100, 2))?;
    team.send(order_at("102", "sell", 100, 1))?;
    team.send(order_at("103", "sell", 101, 1))?;
    team.wait_report("102", "0")?;
    team.wait_report("103", "0")?;
    team.wait_report("101", "0")?;
    // The maker lifts the far lot first: it fills when the order reaches
    // venue 2, one second from now, and the fill takes another second to
    // come back. Just after the far fill, it lifts the near lot, whose fill
    // is back within milliseconds: the later report overtakes the earlier.
    maker.send(order_at("201", "buy", 100, 2))?;
    std::thread::sleep(Duration::from_micros(FAR_US + 200_000));
    maker.send(order_at("202", "buy", 100, 1))?;
    team.wait_report("102", "F")?;
    assert_eq!(fills(&team, "101"), 0, "the far fill is still on its path");
    // Team 1 drops before the far fill arrives; while it is away the maker
    // lifts its last lot.
    team.stream
        .shutdown(std::net::Shutdown::Both)
        .map_err(|error| error.to_string())?;
    maker.send(order_at("203", "buy", 101, 1))?;
    maker.wait_report("203", "F")?;
    team.received.clear();
    let mut team = team.reconnect(port)?;
    team.wait_report("101", "F")?;
    team.wait_report("103", "F")?;
    // Nothing the last session wrote comes again.
    let until = Instant::now() + Duration::from_millis(300);
    team.pump_until(|_| Instant::now() >= until)?;
    assert_eq!(fills(&team, "101"), 1);
    assert_eq!(fills(&team, "103"), 1);
    assert_eq!(fills(&team, "102"), 0, "the near fill was already written");
    Ok(())
}
