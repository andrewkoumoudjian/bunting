//! FIX and the Bunting Native Protocol are two adapters over one venue path
//! (ADR 0031, ADR 0040): the same order script sent by the same team over
//! FIX at one venue and over BNP at an identical venue must reach the engine
//! as the same commands, commit the same events and leave the same account,
//! and both must cross the same virtual path, neither one faster.
//!
//! Each venue journals to a file origin; the test reads both journals and
//! compares them with identifiers and times removed (they are namespaced per
//! interface and stamped from the run clock, so they differ by design).

mod support;

use bunting_admission_sequencer::{
    Endpoint, LatencyMap, Link, ParticipantPlacement, PathLatency, VenuePlacement,
};
use bunting_api_contract::ActorRole;
use bunting_client::bnp_wire::{
    ClientMessage, Listing, NewOrder, OrderType, ServerMessage, Side, TimeInForce,
};
use bunting_client::{Client, ClientConfig};
use bunting_market_types::{ParticipantId, RunId, VenueId};
use bunting_origin_store::{CommandRecord, JournalInput};
use bunting_server::config::{
    AdmissionConfig, BnpConfig, BnpRosterEntry, RosterEntry, ScenarioConfig, ServerConfig,
    StorageKind,
};
use serde_json::Value;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::pki::{Authority, Issued, write};
use support::{TIMEOUT, cancel, order};

/// The subject team's distance from the venue, each way.
const PATH_US: u64 = 20_000;
const SUBJECT: u128 = 2;
const MAKER: u128 = 10;
const LISTING: Listing = Listing {
    venue_id: 1,
    instrument_id: 1,
};

struct Venue {
    fix_port: u16,
    bnp_port: u16,
    origin: PathBuf,
    ca_pem: String,
    subject: Issued,
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

/// The subject sits `PATH_US` from venue 1; the maker sits with it.
fn map() -> LatencyMap {
    let at = |location: &str| location.to_owned();
    LatencyMap {
        default_location: at("venue"),
        participants: vec![ParticipantPlacement {
            participant_id: ParticipantId::new(SUBJECT),
            location: at("team"),
        }],
        venues: vec![VenuePlacement {
            venue_id: VenueId::new(1),
            location: at("venue"),
        }],
        local: PathLatency {
            latency_us: 0,
            jitter_us: 0,
        },
        links: vec![Link {
            between: [at("venue"), at("team")],
            latency: PathLatency {
                latency_us: PATH_US,
                jitter_us: 0,
            },
        }],
        ..LatencyMap::default()
    }
}

fn start_venue(name: &str) -> Result<Venue, String> {
    let fix_port = free_port()?;
    let bnp_port = free_port()?;
    let directory = std::env::temp_dir().join(format!("bunting-parity-{name}-{bnp_port}"));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut authority = Authority::new("Bunting parity CA")?;
    let server = authority.server()?;
    let (subject, _) = authority.client("subject")?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("config/scenario.json")
            .display()
            .to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let origin = directory.join("origin.json");
    config.storage.kind = StorageKind::File;
    config.storage.path = Some(origin.display().to_string());
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{fix_port}");
    fix.roster.push(RosterEntry {
        target_comp_id: "MAKER".to_owned(),
        username: "maker".to_owned(),
        password: "bunting-maker-dev".to_owned(),
        role: ActorRole::Participant,
        participant_id: MAKER,
    });
    fix.max_connections = 3;
    fix.admission = AdmissionConfig::with_map(map());
    config.bnp = Some(BnpConfig {
        bind: format!("127.0.0.1:{bnp_port}"),
        run_id: 1,
        certificate_chain: write(&directory, "server.pem", &server.certificate_pem)?,
        private_key: write(&directory, "server.key", &server.key_pem)?,
        client_ca: write(&directory, "ca.pem", &authority.ca_pem)?,
        revocation_lists: Vec::new(),
        roster: vec![BnpRosterEntry {
            certificate_sha256: subject.fingerprint()?,
            participant_id: SUBJECT,
            role: ActorRole::Participant,
        }],
        heartbeat_ms: 1_000,
        max_connections: 1,
        max_frame_bytes: 65_536,
        rate_limit_window_ms: 100,
        max_messages_per_interval: 64,
        handshake_timeout_ms: 5_000,
    });
    config.validate().map_err(|error| error.to_string())?;
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", bnp_port)).is_err()
        || TcpStream::connect(("127.0.0.1", fix_port)).is_err()
    {
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(Venue {
        fix_port,
        bnp_port,
        origin,
        ca_pem: authority.ca_pem,
        subject,
    })
}

impl Venue {
    fn bnp(&self) -> Result<Client, String> {
        Client::connect(&ClientConfig {
            address: format!("127.0.0.1:{}", self.bnp_port),
            server_name: "localhost".to_owned(),
            ca_pem: self.ca_pem.clone().into_bytes(),
            certificate_pem: self.subject.certificate_pem.clone().into_bytes(),
            private_key_pem: self.subject.key_pem.clone().into_bytes(),
            resume_after: None,
            client_name: "parity".to_owned(),
            connect_timeout: Duration::from_secs(5),
        })
        .map_err(|error| error.to_string())
    }

    /// The maker's two resting offers, sent over FIX at both venues.
    fn post_offers(&self) -> Result<support::Client, String> {
        let mut maker =
            support::Client::logon(self.fix_port, "MAKER", "maker", "bunting-maker-dev")?;
        maker.send(order("801", "sell", 5, Some(101)))?;
        maker.wait_report("801", "0")?;
        maker.send(order("802", "sell", 5, Some(102)))?;
        maker.wait_report("802", "0")?;
        Ok(maker)
    }

    fn journal(&self) -> Result<Vec<CommandRecord>, String> {
        bunting_server::storage::read_run_journal(Path::new(&self.origin), RunId::new(1))
            .map(|(_, records)| records)
            .map_err(|error| error.to_string())
    }
}

/// The script both interfaces run: a fill, a resting order, its cancel, and
/// an order that sweeps two price levels.
#[derive(Clone, Copy)]
enum Step {
    Buy { id: u64, quantity: i64, price: i64 },
    Cancel { id: u64, original: u64 },
}

const SCRIPT: [Step; 4] = [
    Step::Buy {
        id: 1,
        quantity: 3,
        price: 101,
    },
    Step::Buy {
        id: 2,
        quantity: 4,
        price: 100,
    },
    Step::Cancel { id: 3, original: 2 },
    Step::Buy {
        id: 4,
        quantity: 6,
        price: 102,
    },
];

/// Runs the script over FIX; returns each step's send-to-first-report time.
fn run_fix(venue: &Venue) -> Result<Vec<Duration>, String> {
    let mut team = support::Client::logon(venue.fix_port, "TEAM2", "team2", "bunting-team2-dev")?;
    let mut elapsed = Vec::new();
    for step in SCRIPT {
        let started = Instant::now();
        let (id, message, last) = match step {
            Step::Buy {
                id,
                quantity,
                price,
            } => (
                id,
                order(&id.to_string(), "buy", quantity, Some(price)),
                None,
            ),
            // The venue confirms a cancel on the original order's ID.
            Step::Cancel { id, original } => (
                original,
                cancel(&id.to_string(), &original.to_string()),
                Some("4"),
            ),
        };
        let id = id.to_string();
        let before = team.reports_for(&id).len();
        team.send(message)?;
        {
            let id = id.clone();
            team.pump_until(move |client| client.reports_for(&id).len() > before)?;
        }
        elapsed.push(started.elapsed());
        // Wait for the step's last report so both runs commit in one order:
        // the cancel's confirmation, a resting order's acceptance, or a
        // complete fill (OrdStatus 2).
        team.pump_until(move |client| {
            client.reports_for(&id).iter().any(|report| match last {
                Some(exec_type) => report.value(150) == Some(exec_type),
                None => report.value(39) == Some("2") || report.value(150) == Some("0"),
            })
        })?;
        if matches!(step, Step::Buy { quantity: 6, .. }) {
            team.pump_until(|client| {
                client
                    .reports_for("4")
                    .iter()
                    .any(|report| report.value(39) == Some("2"))
            })?;
        }
    }
    Ok(elapsed)
}

/// Receives until `done` matches; returns the time the first message came.
fn wait(client: &Client, done: impl Fn(&ServerMessage) -> bool) -> Result<Instant, String> {
    let deadline = Instant::now() + TIMEOUT;
    let mut first = None;
    loop {
        if Instant::now() > deadline {
            return Err("timed out waiting for a BNP report".to_owned());
        }
        if let Some(message) = client
            .recv_timeout(Duration::from_millis(20))
            .map_err(|error| error.to_string())?
        {
            let at = *first.get_or_insert_with(Instant::now);
            if done(&message) {
                return Ok(at);
            }
        }
    }
}

/// Runs the script over BNP; returns each step's send-to-first-report time.
fn run_bnp(venue: &Venue) -> Result<Vec<Duration>, String> {
    let team = venue.bnp()?;
    let mut elapsed = Vec::new();
    for step in SCRIPT {
        let started = Instant::now();
        let first = match step {
            Step::Buy {
                id,
                quantity,
                price,
            } => {
                team.send(&ClientMessage::NewOrder(NewOrder {
                    client_order_id: id,
                    listing: LISTING,
                    side: Side::Buy,
                    quantity,
                    order_type: OrderType::Limit { price },
                    time_in_force: TimeInForce::Gtc,
                    post_only: false,
                    anonymous: false,
                    display_quantity: None,
                }))
                .map_err(|error| error.to_string())?;
                wait(&team, |message| match message {
                    ServerMessage::OrderRested {
                        client_order_id, ..
                    }
                    | ServerMessage::OrderDone {
                        client_order_id, ..
                    } => *client_order_id == id,
                    _ => false,
                })?
            }
            Step::Cancel { original, .. } => {
                team.send(&ClientMessage::CancelOrder {
                    client_order_id: original,
                })
                .map_err(|error| error.to_string())?;
                wait(
                    &team,
                    |message| matches!(message, ServerMessage::OrderCanceled { client_order_id, .. } if *client_order_id == original),
                )?
            }
        };
        elapsed.push(first.duration_since(started));
    }
    Ok(elapsed)
}

/// A JSON value with every identifier and time removed: what must match
/// across interfaces.
fn normalized(mut value: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(map) => {
                map.retain(|key, _| {
                    !(key.ends_with("order_id")
                        || key.ends_with("command_id")
                        || key == "correlation_id"
                        || key == "event_id"
                        || key == "logical_time"
                        || key == "expected_sequence")
                });
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut value);
    value
}

/// Through text: `Value` cannot hold the 128-bit identifiers, and they are
/// removed anyway.
fn json(value: &impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_string(value)
        .and_then(|text| serde_json::from_str(&text))
        .map_err(|error| error.to_string())
}

/// One participant command's comparable facts: who sent it, the command and
/// every event it committed (sequence included), without identifiers.
fn comparable(record: &CommandRecord) -> Result<Option<Value>, String> {
    let JournalInput::Command(command) = &record.input else {
        return Ok(None);
    };
    if ![SUBJECT, MAKER].contains(&command.actor.get()) {
        return Ok(None);
    }
    Ok(Some(normalized(serde_json::json!({
        "actor": command.actor.get().to_string(),
        "payload": json(&command.payload)?,
        "events": json(&record.events)?,
    }))))
}

fn subject_admissions(records: &[CommandRecord]) -> Vec<(Endpoint, u64, u64)> {
    records
        .iter()
        .filter(|record| {
            matches!(&record.input, JournalInput::Command(command) if command.actor.get() == SUBJECT)
        })
        .filter_map(|record| record.admission.as_ref())
        .map(|admission| {
            (
                admission.destination,
                admission.path_latency_us,
                admission.release_us.saturating_sub(admission.received_us),
            )
        })
        .collect()
}

fn account(venue: &Venue) -> Result<ServerMessage, String> {
    let client = venue.bnp()?;
    client
        .send(&ClientMessage::AccountRequest { request_id: 1 })
        .map_err(|error| error.to_string())?;
    let deadline = Instant::now() + TIMEOUT;
    while Instant::now() < deadline {
        if let Some(message @ ServerMessage::Account { .. }) = client
            .recv_timeout(Duration::from_millis(20))
            .map_err(|error| error.to_string())?
        {
            return Ok(message);
        }
    }
    Err("no account snapshot".to_owned())
}

#[test]
fn the_same_orders_over_fix_and_bnp_commit_the_same_events_over_the_same_path() -> Result<(), String>
{
    let over_fix = start_venue("fix")?;
    let over_bnp = start_venue("bnp")?;
    let _fix_maker = over_fix.post_offers()?;
    let _bnp_maker = over_bnp.post_offers()?;
    let fix_elapsed = run_fix(&over_fix)?;
    let bnp_elapsed = run_bnp(&over_bnp)?;

    // Same commands and the same committed events, in the same order.
    let fix_journal = over_fix.journal()?;
    let bnp_journal = over_bnp.journal()?;
    let collect = |records: &[CommandRecord]| -> Result<Vec<Value>, String> {
        records
            .iter()
            .filter_map(|record| comparable(record).transpose())
            .collect()
    };
    let fix_facts = collect(&fix_journal)?;
    let bnp_facts = collect(&bnp_journal)?;
    assert_eq!(fix_facts.len(), 6, "2 offers + 4 subject steps over FIX");
    for (index, (fix, bnp)) in fix_facts.iter().zip(&bnp_facts).enumerate() {
        assert_eq!(
            fix,
            bnp,
            "command {index} differs between FIX and BNP:\nFIX {}\nBNP {}",
            serde_json::to_string_pretty(fix).unwrap_or_default(),
            serde_json::to_string_pretty(bnp).unwrap_or_default()
        );
    }
    assert_eq!(fix_facts.len(), bnp_facts.len());

    // Both interfaces crossed the same virtual path to the venue, and the
    // sequencer added nothing beyond it.
    let fix_paths = subject_admissions(&fix_journal);
    let bnp_paths = subject_admissions(&bnp_journal);
    assert_eq!(fix_paths.len(), SCRIPT.len());
    assert_eq!(bnp_paths.len(), SCRIPT.len());
    for (destination, path, waited) in fix_paths.iter().chain(&bnp_paths) {
        assert_eq!(*destination, Endpoint::Venue(VenueId::new(1)));
        assert_eq!(*path, PATH_US);
        assert_eq!(*waited, PATH_US, "release is arrival plus the path");
    }

    // Each first report took at least the round trip on both interfaces.
    let round_trip = Duration::from_micros(2 * PATH_US);
    for (index, elapsed) in fix_elapsed.iter().chain(&bnp_elapsed).enumerate() {
        assert!(
            *elapsed >= round_trip,
            "step {index} answered in {elapsed:?}, under the {round_trip:?} round trip"
        );
    }

    // The same account at both venues.
    let strip = |message: ServerMessage| match message {
        ServerMessage::Account {
            cash, positions, ..
        } => Some((cash, positions)),
        _ => None,
    };
    let fix_account = strip(account(&over_fix)?);
    let bnp_account = strip(account(&over_bnp)?);
    assert!(fix_account.is_some());
    assert_eq!(fix_account, bnp_account);
    Ok(())
}
