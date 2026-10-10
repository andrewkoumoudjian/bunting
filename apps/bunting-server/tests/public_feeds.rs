//! Per-venue public market data (slice 21): a FIX subscription to one
//! venue's listing streams that venue's anonymous trades and depth changes,
//! each sent from the venue over the subscriber's virtual path (ADR 0035).
//! Team 1 sits 1 ms from venue 1 and 40 ms from venue 2, team 2 the reverse;
//! the maker sits with venue 1.

mod support;

use bunting_admission_sequencer::{
    LatencyMap, Link, ParticipantPlacement, PathLatency, VenuePlacement,
};
use bunting_api_contract::ActorRole;
use bunting_market_types::{ParticipantId, VenueId};
use bunting_server::config::{AdmissionConfig, RosterEntry, ScenarioConfig, ServerConfig};
use simfix_wire::FixMessage;
use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT};

const NEAR_US: u64 = 1_000;
const FAR_US: u64 = 40_000;

fn latency(latency_us: u64) -> PathLatency {
    PathLatency {
        latency_us,
        jitter_us: 0,
    }
}

fn two_cities() -> LatencyMap {
    let at = |location: &str| location.to_owned();
    LatencyMap {
        default_location: at("one"),
        participants: vec![
            ParticipantPlacement {
                participant_id: ParticipantId::new(1),
                location: at("one"),
            },
            ParticipantPlacement {
                participant_id: ParticipantId::new(2),
                location: at("two"),
            },
        ],
        venues: vec![
            VenuePlacement {
                venue_id: VenueId::new(1),
                location: at("one"),
            },
            VenuePlacement {
                venue_id: VenueId::new(2),
                location: at("two"),
            },
        ],
        local: latency(NEAR_US),
        links: vec![Link {
            between: [at("one"), at("two")],
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
    let path_file = std::env::temp_dir().join(format!("bunting-feeds-{port}.json"));
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
    fix.admission = AdmissionConfig::with_map(two_cities());
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

fn order_at(id: &str, side: &str, quantity: i64, price: i64, venue: u128) -> FixMessage {
    let mut message = FixMessage::new("D");
    message.push(11, id);
    message.push(48, "1");
    message.push(207, venue.to_string());
    message.push(54, if side == "buy" { "1" } else { "2" });
    message.push(38, quantity.to_string());
    message.push(40, "2");
    message.push(44, price.to_string());
    message
}

fn cancel_at(id: &str, original: &str, side: &str) -> FixMessage {
    let mut message = FixMessage::new("F");
    message.push(11, id);
    message.push(41, original);
    message.push(48, "1");
    message.push(54, if side == "buy" { "1" } else { "2" });
    message
}

/// FIX `V`: 263 = 0 snapshot, 1 subscribe, 2 unsubscribe.
fn market_request(id: &str, request_type: &str, venue: u128) -> FixMessage {
    let mut message = FixMessage::new("V");
    for (tag, value) in [
        (262, id.to_owned()),
        (263, request_type.to_owned()),
        (264, "0".to_owned()),
        (267, "3".to_owned()),
        (269, "0".to_owned()),
        (269, "1".to_owned()),
        (269, "2".to_owned()),
        (48, "1".to_owned()),
        (207, venue.to_string()),
    ] {
        message.push(tag, value);
    }
    message
}

fn received(client: &Client, msg_type: &str, id: &str) -> Vec<FixMessage> {
    client
        .received
        .iter()
        .filter(|message| message.msg_type == msg_type && message.value(262) == Some(id))
        .cloned()
        .collect()
}

/// Every repeating-group entry of an `X`, as `(279, 269, 270, 271)`.
fn entries(message: &FixMessage) -> Vec<(String, String, i64, i64)> {
    let mut entries = Vec::new();
    let mut current: Option<(String, String, i64, i64)> = None;
    for field in &message.fields {
        match field.tag {
            279 => {
                entries.extend(current.take());
                current = Some((field.value.clone(), String::new(), 0, 0));
            }
            269 => {
                if let Some(entry) = &mut current {
                    entry.1.clone_from(&field.value);
                }
            }
            270 => {
                if let Some(entry) = &mut current {
                    entry.2 = field.value.parse().unwrap_or(-1);
                }
            }
            271 => {
                if let Some(entry) = &mut current {
                    entry.3 = field.value.parse().unwrap_or(-1);
                }
            }
            _ => {}
        }
    }
    entries.extend(current);
    entries
}

type Book = (BTreeMap<i64, i64>, BTreeMap<i64, i64>);

fn snapshot_book(message: &FixMessage) -> Book {
    let mut book = Book::default();
    let mut side = "";
    let mut price = 0;
    for field in &message.fields {
        match field.tag {
            269 => side = if field.value == "0" { "bid" } else { "ask" },
            270 => price = field.value.parse().unwrap_or(-1),
            271 => {
                let quantity = field.value.parse().unwrap_or(-1);
                if side == "bid" {
                    book.0.insert(price, quantity);
                } else {
                    book.1.insert(price, quantity);
                }
            }
            _ => {}
        }
    }
    book
}

/// The subscriber's book: its snapshot with every increment applied.
fn feed_book(client: &Client, id: &str) -> Result<Book, String> {
    let snapshot = received(client, "W", id);
    let mut book = snapshot_book(snapshot.first().ok_or("no snapshot")?);
    let mut expected_report = 1;
    for message in received(client, "X", id) {
        for (offset, (action, entry_type, price, quantity)) in
            entries(&message).into_iter().enumerate()
        {
            let report = message
                .fields
                .iter()
                .filter(|field| field.tag == 83)
                .nth(offset)
                .and_then(|field| field.value.parse::<u64>().ok());
            assert_eq!(report, Some(expected_report), "feed {id} has a gap");
            expected_report += 1;
            let levels = match entry_type.as_str() {
                "0" => &mut book.0,
                "1" => &mut book.1,
                _ => continue,
            };
            if action == "2" {
                levels.remove(&price);
            } else {
                levels.insert(price, quantity);
            }
        }
    }
    Ok(book)
}

fn pump_all(clients: &mut [&mut Client], duration: Duration) -> Result<(), String> {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        for client in clients.iter_mut() {
            let step = Instant::now() + Duration::from_micros(300);
            client.pump_until(|_| Instant::now() >= step)?;
        }
    }
    Ok(())
}

fn fast_reads(client: &Client) -> Result<(), String> {
    client
        .stream
        .set_read_timeout(Some(Duration::from_micros(300)))
        .map_err(|error| error.to_string())
}

#[test]
fn a_venue_feed_reaches_each_team_over_its_own_path() -> Result<(), String> {
    let port = start_server()?;
    let mut maker = Client::logon(port, "MAKER", "maker", "bunting-maker-dev")?;
    let mut team1 = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut team2 = Client::logon(port, "TEAM2", "team2", "bunting-team2-dev")?;
    for client in [&maker, &team1, &team2] {
        fast_reads(client)?;
    }
    team1.send(market_request("v1", "1", 1))?;
    team2.send(market_request("v1", "1", 1))?;
    let deadline = Instant::now() + TIMEOUT;
    while received(&team1, "W", "v1").is_empty() || received(&team2, "W", "v1").is_empty() {
        if Instant::now() > deadline {
            return Err("subscriptions were not acknowledged".to_owned());
        }
        pump_all(&mut [&mut team1, &mut team2], Duration::from_millis(5))?;
    }

    // An offer appears on venue 1; time when each team learns of it.
    maker.send(order_at("901", "sell", 5, 100, 1))?;
    let sent = Instant::now();
    let mut first_seen: [Option<Instant>; 2] = [None, None];
    let deadline = Instant::now() + TIMEOUT;
    while first_seen.iter().any(Option::is_none) {
        if Instant::now() > deadline {
            return Err("the offer never reached both feeds".to_owned());
        }
        pump_all(
            &mut [&mut maker, &mut team1, &mut team2],
            Duration::from_micros(600),
        )?;
        for (seen, client) in first_seen.iter_mut().zip([&team1, &team2]) {
            if seen.is_none()
                && received(client, "X", "v1").iter().any(|message| {
                    entries(message).contains(&("0".to_owned(), "1".to_owned(), 100, 5))
                })
            {
                *seen = Some(Instant::now());
            }
        }
    }
    let [Some(near), Some(far)] = first_seen else {
        return Err("unreachable".to_owned());
    };
    // Delays only grow under load, so the far bound is a lower bound: the
    // offer crossed maker -> venue 1 (1 ms) then venue 1 -> team 2 (40 ms).
    let near = near.duration_since(sent);
    let far = far.duration_since(sent);
    assert!(
        far >= Duration::from_millis(40) && near < far,
        "venue 1's feed must reach team 2 no sooner than its 40 ms path and after team 1: near {near:?}, far {far:?}"
    );

    // Team 1 lifts two lots: both feeds show the anonymous trade and the
    // reduced level, and nothing that identifies who traded.
    team1.send(order_at("101", "buy", 2, 100, 1))?;
    team1.wait_report("101", "F")?;
    pump_all(
        &mut [&mut maker, &mut team1, &mut team2],
        Duration::from_millis(150),
    )?;
    for client in [&team1, &team2] {
        let updates = received(client, "X", "v1");
        let all: Vec<_> = updates.iter().flat_map(entries).collect();
        assert!(
            all.contains(&("0".to_owned(), "2".to_owned(), 100, 2)),
            "{all:?}"
        );
        assert!(
            all.contains(&("1".to_owned(), "1".to_owned(), 100, 3)),
            "{all:?}"
        );
        for message in &updates {
            for tag in [1, 11, 37, 41, 448, 452, 523] {
                assert_eq!(message.value(tag), None, "tag {tag} leaks identity");
            }
        }
        let (bids, asks) = feed_book(client, "v1")?;
        assert!(bids.is_empty());
        assert_eq!(asks, BTreeMap::from([(100, 3)]));
    }
    Ok(())
}

#[test]
fn snapshot_plus_increments_track_the_book_until_unsubscribed() -> Result<(), String> {
    let port = start_server()?;
    let mut maker = Client::logon(port, "MAKER", "maker", "bunting-maker-dev")?;
    let mut team1 = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    for client in [&maker, &team1] {
        fast_reads(client)?;
    }
    // Resting depth before the subscription arrives at the venue.
    maker.send(order_at("801", "sell", 4, 103, 1))?;
    maker.wait_report("801", "0")?;
    team1.send(market_request("feed", "1", 1))?;
    // Book activity racing the subscription's snapshot.
    maker.send(order_at("802", "sell", 2, 104, 1))?;
    team1.send(order_at("101", "buy", 3, 99, 1))?;
    maker.send(order_at("803", "sell", 1, 103, 1))?;
    maker.wait_report("803", "0")?;
    team1.wait_report("101", "0")?;
    maker.send(cancel_at("804", "802", "sell"))?;
    maker.wait_report("802", "4")?;
    team1.send(order_at("102", "buy", 1, 103, 1))?;
    team1.wait_report("102", "F")?;
    pump_all(&mut [&mut maker, &mut team1], Duration::from_millis(50))?;

    // The subscriber's reconstructed book equals a fresh snapshot.
    team1.send(market_request("check", "0", 1))?;
    team1.pump_until(|client| !received(client, "W", "check").is_empty())?;
    let fresh = snapshot_book(&received(&team1, "W", "check")[0]);
    assert_eq!(feed_book(&team1, "feed")?, fresh);
    assert_eq!(fresh.1, BTreeMap::from([(103, 4)]));
    assert_eq!(fresh.0, BTreeMap::from([(99, 3)]));

    // After unsubscribing, venue 1 activity no longer reaches this feed.
    team1.send(market_request("feed", "2", 1))?;
    pump_all(&mut [&mut maker, &mut team1], Duration::from_millis(20))?;
    let before = received(&team1, "X", "feed").len();
    maker.send(order_at("805", "sell", 1, 105, 1))?;
    maker.wait_report("805", "0")?;
    pump_all(&mut [&mut maker, &mut team1], Duration::from_millis(50))?;
    assert_eq!(received(&team1, "X", "feed").len(), before);
    Ok(())
}
