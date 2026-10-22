//! Regression tests for the BNP v1 review (2026-10-10, main 123e588;
//! `docs/reviews/2026-10-10-bnp-v1-review.md`). Each reproduces a confirmed
//! defect; slice 32 fixed all three and removed their `#[ignore]`.
//!
//! The harness is copied from `bnp.rs` with one change to the latency map:
//! the hub sits with team 2 ("two"); venue 1, team 1 and the FIX maker sit
//! at "one", 30 ms away. Reports from the hub therefore reach team 2 before
//! earlier reports from venue 1.
#![allow(dead_code)]

mod support;

use bunting_admission_sequencer::{
    LatencyMap, Link, ParticipantPlacement, PathLatency, VenuePlacement,
};
use bunting_api_contract::ActorRole;
use bunting_client::bnp_wire::{
    ClientMessage, Listing, NewOrder, OrderType, ResumeStatus, ServerMessage, Side, TimeInForce,
};
use bunting_client::{Client, ClientConfig, ClientError};
use bunting_market_types::{ParticipantId, VenueId};
use bunting_server::config::{
    AdmissionConfig, BnpConfig, BnpRosterEntry, RosterEntry, ScenarioConfig, ServerConfig,
};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use support::pki::{Authority, Issued, write};
use support::{TIMEOUT, order};

const FAR_US: u64 = 30_000;
const LISTING: Listing = Listing {
    venue_id: 1,
    instrument_id: 1,
};

struct Venue {
    fix_port: u16,
    bnp_port: u16,
    ca_pem: String,
    team1: Issued,
    team2: Issued,
    revoked: Issued,
    unregistered: Issued,
    foreign: Issued,
    authority: Authority,
    revocation_list: PathBuf,
    revoked_serials: Vec<u64>,
    team2_serial: u64,
}

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

fn map() -> LatencyMap {
    let at = |location: &str| location.to_owned();
    LatencyMap {
        default_location: at("two"),
        participants: vec![
            ParticipantPlacement {
                participant_id: ParticipantId::new(1),
                location: at("one"),
            },
            ParticipantPlacement {
                participant_id: ParticipantId::new(10),
                location: at("one"),
            },
        ],
        venues: vec![VenuePlacement {
            venue_id: VenueId::new(1),
            location: at("one"),
        }],
        local: PathLatency {
            latency_us: 0,
            jitter_us: 0,
        },
        links: vec![Link {
            between: [at("one"), at("two")],
            latency: PathLatency {
                latency_us: FAR_US,
                jitter_us: 0,
            },
        }],
        ..LatencyMap::default()
    }
}

fn start_venue(name: &str) -> Result<Venue, String> {
    let fix_port = free_port()?;
    let bnp_port = free_port()?;
    let directory = std::env::temp_dir().join(format!("bunting-bnp-{name}-{bnp_port}"));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut authority = Authority::new("Bunting test operator CA")?;
    let server = authority.server()?;
    let (team1, _) = authority.client("team 1")?;
    let (team2, team2_serial) = authority.client("team 2")?;
    let (revoked, revoked_serial) = authority.client("revoked")?;
    let (unregistered, _) = authority.client("unregistered")?;
    let foreign = Authority::new("Somebody else's CA")?.client("foreign")?.0;
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
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{fix_port}");
    fix.roster.push(RosterEntry {
        target_comp_id: "MAKER".to_owned(),
        username: "maker".to_owned(),
        password: "bunting-maker-dev".to_owned(),
        role: ActorRole::Participant,
        participant_id: 10,
    });
    fix.max_connections = 3;
    fix.admission = AdmissionConfig::with_map(map());
    let entry = |issued: &Issued, participant_id| {
        Ok::<_, String>(BnpRosterEntry {
            certificate_sha256: issued.fingerprint()?,
            participant_id,
            role: ActorRole::Participant,
        })
    };
    config.bnp = Some(BnpConfig {
        bind: format!("127.0.0.1:{bnp_port}"),
        run_id: 1,
        certificate_chain: write(&directory, "server.pem", &server.certificate_pem)?,
        private_key: write(&directory, "server.key", &server.key_pem)?,
        client_ca: write(&directory, "ca.pem", &authority.ca_pem)?,
        revocation_lists: vec![write(
            &directory,
            "ca.crl",
            &authority.revocation_list(&[revoked_serial])?,
        )?],
        roster: vec![entry(&team1, 1)?, entry(&team2, 2)?, entry(&revoked, 10)?],
        heartbeat_ms: 1_000,
        max_connections: 3,
        max_frame_bytes: 65_536,
        rate_limit_window_ms: 100,
        max_messages_per_interval: 64,
        handshake_timeout_ms: 5_000,
        enrolment: None,
    });
    config.validate().map_err(|error| error.to_string())?;
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    support::wait_listening(&[bnp_port, fix_port])?;
    Ok(Venue {
        fix_port,
        bnp_port,
        ca_pem: authority.ca_pem.clone(),
        team1,
        team2,
        revoked,
        unregistered,
        foreign,
        authority,
        revocation_list: directory.join("ca.crl"),
        revoked_serials: vec![revoked_serial],
        team2_serial,
    })
}

impl Venue {
    fn config(&self, issued: &Issued, resume_after: Option<u64>) -> ClientConfig {
        ClientConfig {
            address: format!("127.0.0.1:{}", self.bnp_port),
            server_name: "localhost".to_owned(),
            ca_pem: self.ca_pem.clone().into_bytes(),
            certificate_pem: issued.certificate_pem.clone().into_bytes(),
            private_key_pem: issued.key_pem.clone().into_bytes(),
            resume_after,
            client_name: "bnp-test".to_owned(),
            connect_timeout: Duration::from_secs(5),
        }
    }

    fn connect(&self, issued: &Issued, resume_after: Option<u64>) -> Result<Client, String> {
        Client::connect(&self.config(issued, resume_after)).map_err(|error| error.to_string())
    }

    fn maker(&self) -> Result<support::Client, String> {
        support::Client::logon(self.fix_port, "MAKER", "maker", "bunting-maker-dev")
    }
}

/// Receives until `done` matches a message; returns everything received.
fn until(
    client: &Client,
    done: impl Fn(&ServerMessage) -> bool,
) -> Result<Vec<ServerMessage>, String> {
    let deadline = Instant::now() + TIMEOUT;
    let mut received = Vec::new();
    loop {
        if Instant::now() > deadline {
            return Err(format!("timed out; received {received:?}"));
        }
        if let Some(message) = client
            .recv_timeout(Duration::from_millis(50))
            .map_err(|error| format!("{error}; received {received:?}"))?
        {
            let finished = done(&message);
            received.push(message);
            if finished {
                return Ok(received);
            }
        }
    }
}

fn limit(client_order_id: u64, side: Side, quantity: i64, price: i64) -> ClientMessage {
    ClientMessage::NewOrder(NewOrder {
        client_order_id,
        listing: LISTING,
        side,
        quantity,
        order_type: OrderType::Limit { price },
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        anonymous: false,
        display_quantity: None,
    })
}

fn refused(result: Result<Client, ClientError>) -> Result<String, String> {
    match result {
        Ok(_) => Err("the venue accepted a certificate it must refuse".to_owned()),
        Err(ClientError::Refused(reason)) => Ok(reason),
        Err(other) => Err(format!("expected a refusal, got {other}")),
    }
}

/// F1: a participant's kill switch must cancel that participant's live
/// orders and block only that participant. Today it sets one run-wide flag
/// (`bunting-engine` `ActivateKillSwitch`): nothing is canceled and every
/// other team's new orders are rejected with `KillSwitchActive`.
#[test]
fn one_teams_kill_switch_cancels_its_orders_and_blocks_only_that_team() -> Result<(), String> {
    let venue = start_venue("review-kill")?;
    let team1 = venue.connect(&venue.team1, None)?;
    let team2 = venue.connect(&venue.team2, None)?;
    team1
        .send(&limit(1, Side::Buy, 1, 91))
        .map_err(|error| error.to_string())?;
    until(&team1, |message| {
        matches!(
            message,
            ServerMessage::OrderRested {
                client_order_id: 1,
                ..
            }
        )
    })?;
    team1
        .send(&ClientMessage::KillSwitch { request_id: 1 })
        .map_err(|error| error.to_string())?;
    until(&team1, |message| {
        matches!(message, ServerMessage::KillSwitchActivated { .. })
    })?;
    team1
        .send(&ClientMessage::OpenOrdersRequest { request_id: 2 })
        .map_err(|error| error.to_string())?;
    let open = until(&team1, |message| {
        matches!(message, ServerMessage::OpenOrders { .. })
    })?;
    assert!(
        matches!(open.last(), Some(ServerMessage::OpenOrders { orders, .. }) if orders.is_empty()),
        "the kill switch left team 1's orders live: {:?}",
        open.last()
    );
    // A buy: team 2 holds no inventory, so a sell is refused for that
    // reason whatever the kill switch does.
    team2
        .send(&limit(5, Side::Buy, 1, 90))
        .map_err(|error| error.to_string())?;
    let reply = until(&team2, |message| {
        matches!(
            message,
            ServerMessage::OrderRejected {
                client_order_id: 5,
                ..
            } | ServerMessage::OrderRested {
                client_order_id: 5,
                ..
            }
        )
    })?;
    assert!(
        matches!(reply.last(), Some(ServerMessage::OrderRested { .. })),
        "team 1's kill switch blocked team 2: {:?}",
        reply.last()
    );
    Ok(())
}

/// F2: a report committed earlier at the far venue reaches team 2 after a
/// later report from the near hub. The client cursor is the highest
/// sequence seen, so a resume from it skips the earlier fill while the
/// venue answers `Replaying`. The fill must be delivered, or the venue must
/// report a gap.
#[test]
fn a_resume_after_out_of_order_delivery_never_loses_a_fill() -> Result<(), String> {
    let venue = start_venue("review-resume")?;
    let mut maker = venue.maker()?;
    let team = venue.connect(&venue.team2, None)?;
    team.send(&limit(1, Side::Buy, 3, 99))
        .map_err(|error| error.to_string())?;
    until(&team, |message| {
        matches!(
            message,
            ServerMessage::OrderRested {
                client_order_id: 1,
                ..
            }
        )
    })?;
    // The maker, beside venue 1, fills the bid; the fill needs 30 ms to
    // reach team 2.
    maker.send(order("901", "sell", 3, Some(99)))?;
    maker.wait_report("901", "F")?;
    // Applied at the hub beside team 2: committed after the fill, delivered
    // before it.
    team.send(&ClientMessage::KillSwitch { request_id: 1 })
        .map_err(|error| error.to_string())?;
    let seen = until(&team, |message| {
        matches!(message, ServerMessage::KillSwitchActivated { .. })
    })?;
    let fill_seen = seen
        .iter()
        .any(|message| matches!(message, ServerMessage::Fill { .. }));
    let cursor = team.cursor();
    team.close();
    let resumed = venue.connect(&venue.team2, Some(cursor))?;
    let gap = resumed.welcome().resume == ResumeStatus::Gap;
    let replay = until(&resumed, |message| {
        matches!(message, ServerMessage::ReplayComplete { .. })
    })
    .unwrap_or_default();
    let fill_replayed = replay
        .iter()
        .any(|message| matches!(message, ServerMessage::Fill { .. }));
    assert!(
        fill_seen || fill_replayed || gap,
        "the fill was lost while the venue reported a complete replay: {replay:?}"
    );
    Ok(())
}

/// F3: unauthenticated TCP connections must not use up the session slots
/// of registered teams. Today `2 x max_connections` idle sockets (no TLS at
/// all) make a registered team's connection fail.
#[test]
fn idle_unauthenticated_sockets_cannot_lock_out_a_registered_team() -> Result<(), String> {
    let venue = start_venue("review-slots")?;
    let idle: Vec<TcpStream> = (0..12)
        .map(|_| TcpStream::connect(("127.0.0.1", venue.bnp_port)))
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    std::thread::sleep(Duration::from_millis(200));
    let result = venue.connect(&venue.team1, None);
    drop(idle);
    let client = result?;
    assert_eq!(client.welcome().participant_id, 1);
    Ok(())
}
