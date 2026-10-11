//! Bunting Native Protocol end to end (ADR 0040): mutual TLS with
//! registered certificates, orders and fills against a FIX maker, a direct
//! venue feed, resume after a disconnect, and the same latency path as FIX.
//!
//! Team 1 and the maker sit with venue 1; team 2 sits 30 ms away from it.

mod support;

use bunting_admission_sequencer::{
    LatencyMap, Link, ParticipantPlacement, PathLatency, VenuePlacement,
};
use bunting_api_contract::ActorRole;
use bunting_client::bnp_wire::{
    CancelReason, ClientMessage, FeedFlags, Liquidity, Listing, NewOrder, NewsAudience,
    OperatorAction, OrderEntry, OrderType, RejectReason, ResumeStatus, ServerMessage, Side,
    TimeInForce, msg_type,
};
use bunting_client::{Client, ClientConfig, ClientError, FeedBook, TapeBook};
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
    instructor: Issued,
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
        default_location: at("one"),
        participants: vec![ParticipantPlacement {
            participant_id: ParticipantId::new(2),
            location: at("two"),
        }],
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
    start_venue_with(name, map())
}

fn start_venue_with(name: &str, map: LatencyMap) -> Result<Venue, String> {
    let fix_port = free_port()?;
    let bnp_port = free_port()?;
    let directory = std::env::temp_dir().join(format!("bunting-bnp-{name}-{bnp_port}"));
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let mut authority = Authority::new("Bunting test operator CA")?;
    let server = authority.server()?;
    let (team1, _) = authority.client("team 1")?;
    let (team2, team2_serial) = authority.client("team 2")?;
    let (instructor, _) = authority.client("instructor")?;
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
    fix.admission = AdmissionConfig::with_map(map);
    let entry = |issued: &Issued, participant_id| {
        Ok::<_, String>(BnpRosterEntry {
            certificate_sha256: issued.fingerprint()?,
            participant_id,
            role: ActorRole::Participant,
        })
    };
    // An instructor's ID is its actor ID; it names no participant.
    let operator = BnpRosterEntry {
        certificate_sha256: instructor.fingerprint()?,
        participant_id: 50,
        role: ActorRole::Instructor,
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
        roster: vec![
            entry(&team1, 1)?,
            entry(&team2, 2)?,
            entry(&revoked, 10)?,
            operator,
        ],
        heartbeat_ms: 1_000,
        max_connections: 4,
        max_frame_bytes: 65_536,
        rate_limit_window_ms: 100,
        max_messages_per_interval: 64,
        handshake_timeout_ms: 5_000,
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
        instructor,
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

#[test]
fn only_registered_unrevoked_certificates_from_the_operator_ca_get_a_session() -> Result<(), String>
{
    let venue = start_venue("identity")?;
    let reason = refused(Client::connect(&venue.config(&venue.unregistered, None)))?;
    assert!(reason.contains("not registered"), "{reason}");
    // Revoked and foreign certificates fail the TLS handshake itself, before
    // any BNP message.
    for issued in [&venue.revoked, &venue.foreign] {
        let result = Client::connect(&venue.config(issued, None));
        assert!(
            matches!(
                result,
                Err(ClientError::Refused(_) | ClientError::Tls(_) | ClientError::Io(_))
            ),
            "{:?}",
            result.err().map(|error| error.to_string())
        );
    }
    // A client that does not trust the venue's CA refuses the venue.
    let mut untrusting = venue.config(&venue.team1, None);
    untrusting.ca_pem = Authority::new("not the operator")?.ca_pem.into_bytes();
    assert!(matches!(
        Client::connect(&untrusting),
        Err(ClientError::Tls(_))
    ));
    // The registered certificate gets the identity the roster gives it.
    let client = venue.connect(&venue.team1, None)?;
    assert_eq!(client.welcome().participant_id, 1);
    assert_eq!(client.welcome().resume, ResumeStatus::Live);
    // One live BNP session per participant.
    let second = Client::connect(&venue.config(&venue.team1, None));
    assert!(second.is_err());
    Ok(())
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one order lifecycle checked end to end"
)]
fn orders_fill_against_a_fix_maker_and_the_feed_shows_the_trade() -> Result<(), String> {
    let venue = start_venue("trading")?;
    let mut maker = venue.maker()?;
    let team = venue.connect(&venue.team1, None)?;
    team.send(&ClientMessage::ListingsRequest { request_id: 1 })
        .map_err(|error| error.to_string())?;
    let listings = until(&team, |message| {
        matches!(message, ServerMessage::Listings { .. })
    })?;
    assert!(matches!(
        listings.last(),
        Some(ServerMessage::Listings { listings, .. }) if listings.iter().any(|info| info.listing == LISTING && info.symbol == "BNT")
    ));
    team.send(&ClientMessage::Subscribe {
        request_id: 7,
        listing: LISTING,
        flags: FeedFlags::ALL,
    })
    .map_err(|error| error.to_string())?;
    let mut book = FeedBook::new(7);
    for message in until(&team, |message| {
        matches!(message, ServerMessage::MarketSnapshot { .. })
    })? {
        book.apply(&message).map_err(|error| error.to_string())?;
    }
    assert!(book.is_ready());

    maker.send(order("901", "sell", 5, Some(100)))?;
    maker.wait_report("901", "0")?;
    // The maker's resting offer reaches the feed.
    for message in until(&team, |message| {
        matches!(message, ServerMessage::MarketUpdate { .. })
    })? {
        book.apply(&message).map_err(|error| error.to_string())?;
    }
    assert_eq!(
        book.best_ask().map(|level| (level.price, level.quantity)),
        Some((100, 5))
    );

    team.send(&limit(1, Side::Buy, 2, 100))
        .map_err(|error| error.to_string())?;
    let reports = until(&team, |message| {
        matches!(
            message,
            ServerMessage::OrderDone {
                client_order_id: 1,
                ..
            }
        )
    })?;
    assert!(reports.iter().any(|message| matches!(
        message,
        ServerMessage::OrderAccepted {
            client_order_id: 1,
            quantity: 2,
            price: Some(100),
            ..
        }
    )));
    assert!(reports.iter().any(|message| matches!(
        message,
        ServerMessage::Fill {
            client_order_id: 1,
            side: Side::Buy,
            price: 100,
            quantity: 2,
            liquidity: Liquidity::Taker,
            ..
        }
    )));
    // Feed updates that arrived alongside the private reports.
    let mut traded = false;
    for message in &reports {
        book.apply(message).map_err(|error| error.to_string())?;
        traded |= book.trades().contains(&(100, 2));
    }
    // The FIX maker sees the same trade from its side.
    maker.wait_report("901", "F")?;
    let fill = maker
        .reports_for("901")
        .into_iter()
        .find(|message| message.value(150) == Some("F"))
        .cloned()
        .ok_or("maker fill")?;
    assert_eq!(fill.value(31), Some("100"));
    assert_eq!(fill.value(32), Some("2"));
    // The feed carries the anonymous trade and the reduced offer.
    let deadline = Instant::now() + TIMEOUT;
    while !(traded && book.best_ask().map(|level| level.quantity) == Some(3)) {
        if Instant::now() > deadline {
            return Err(format!("feed never showed the trade: {book:?}"));
        }
        if let Some(message) = team
            .recv_timeout(Duration::from_millis(50))
            .map_err(|error| error.to_string())?
        {
            book.apply(&message).map_err(|error| error.to_string())?;
            traded |= book.trades().contains(&(100, 2));
        }
    }

    // Account and open orders come from the ledger and the book.
    team.send(&ClientMessage::AccountRequest { request_id: 2 })
        .map_err(|error| error.to_string())?;
    let account = until(&team, |message| {
        matches!(message, ServerMessage::Account { .. })
    })?;
    assert!(matches!(
        account.last(),
        Some(ServerMessage::Account { positions, .. })
            if positions.iter().any(|position| position.instrument_id == 1 && position.position == 2)
    ));
    // Cancelling a filled order is rejected by the engine, privately.
    team.send(&ClientMessage::CancelOrder { client_order_id: 1 })
        .map_err(|error| error.to_string())?;
    until(&team, |message| {
        matches!(
            message,
            ServerMessage::CancelRejected {
                client_order_id: 1,
                ..
            }
        )
    })?;
    // A request the protocol refuses never reaches the engine.
    team.send(&ClientMessage::NewOrder(NewOrder {
        order_type: OrderType::Market,
        time_in_force: TimeInForce::Gtc,
        ..match limit(2, Side::Buy, 1, 100) {
            ClientMessage::NewOrder(order) => order,
            _ => return Err("limit builds a new order".to_owned()),
        }
    }))
    .map_err(|error| error.to_string())?;
    until(&team, |message| {
        matches!(
            message,
            ServerMessage::Reject {
                request_type: msg_type::NEW_ORDER,
                reference: 2,
                ..
            }
        )
    })?;
    Ok(())
}

#[test]
fn a_reconnecting_client_resumes_exactly_after_its_cursor() -> Result<(), String> {
    let venue = start_venue("resume")?;
    let mut maker = venue.maker()?;
    let team = venue.connect(&venue.team1, None)?;
    team.send(&limit(1, Side::Buy, 4, 99))
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
    let cursor = team.cursor();
    assert!(cursor > 0);
    team.close();
    // While the team is away the maker trades against its resting bid.
    maker.send(order("901", "sell", 3, Some(99)))?;
    maker.wait_report("901", "F")?;

    let resumed = venue.connect(&venue.team1, Some(cursor))?;
    assert_eq!(resumed.welcome().resume, ResumeStatus::Replaying);
    let replay = until(&resumed, |message| {
        matches!(message, ServerMessage::ReplayComplete { .. })
    })?;
    // Exactly the missed fill, nothing already seen.
    assert!(replay.iter().any(|message| matches!(
        message,
        ServerMessage::Fill {
            client_order_id: 1,
            quantity: 3,
            liquidity: Liquidity::Maker,
            ..
        }
    )));
    assert!(
        replay
            .iter()
            .filter_map(ServerMessage::stamp)
            .all(|stamp| stamp.sequence > cursor)
    );
    assert!(
        !replay
            .iter()
            .any(|message| matches!(message, ServerMessage::OrderRested { .. }))
    );
    // The live stream continues after the replay.
    resumed
        .send(&ClientMessage::CancelOrder { client_order_id: 1 })
        .map_err(|error| error.to_string())?;
    until(&resumed, |message| {
        matches!(
            message,
            ServerMessage::OrderCanceled {
                client_order_id: 1,
                remaining: 1,
                ..
            }
        )
    })?;
    let after = resumed.cursor();
    resumed.close();
    // A cursor this venue never published is a gap: rebuild from snapshots.
    let lost = venue.connect(&venue.team1, Some(after + 1_000_000))?;
    assert_eq!(lost.welcome().resume, ResumeStatus::Gap);
    lost.send(&ClientMessage::OpenOrdersRequest { request_id: 3 })
        .map_err(|error| error.to_string())?;
    let open = until(&lost, |message| {
        matches!(message, ServerMessage::OpenOrders { .. })
    })?;
    assert!(
        matches!(open.last(), Some(ServerMessage::OpenOrders { orders, .. }) if orders.is_empty())
    );
    Ok(())
}

#[test]
fn bnp_crosses_the_same_virtual_path_as_fix() -> Result<(), String> {
    let venue = start_venue("latency")?;
    // Team 2 sits 30 ms from venue 1: its order travels 30 ms to the venue
    // and the report 30 ms back, on top of its real connection.
    let team = venue.connect(&venue.team2, None)?;
    let sent = Instant::now();
    team.send(&limit(1, Side::Buy, 1, 90))
        .map_err(|error| error.to_string())?;
    until(&team, |message| {
        matches!(
            message,
            ServerMessage::OrderAccepted {
                client_order_id: 1,
                ..
            }
        )
    })?;
    let round_trip = sent.elapsed();
    assert!(
        round_trip >= Duration::from_micros(2 * FAR_US),
        "report arrived after {round_trip:?}"
    );
    // Session messages are never delayed: a ping returns at once.
    let pinged = Instant::now();
    team.send(&ClientMessage::Ping { nonce: 9 })
        .map_err(|error| error.to_string())?;
    until(&team, |message| {
        matches!(message, ServerMessage::Pong { nonce: 9, .. })
    })?;
    assert!(pinged.elapsed() < Duration::from_micros(FAR_US));
    Ok(())
}

#[test]
fn a_revocation_list_update_logs_out_the_live_session_without_a_restart() -> Result<(), String> {
    let venue = start_venue("revocation")?;
    let team1 = venue.connect(&venue.team1, None)?;
    let team2 = venue.connect(&venue.team2, None)?;
    // The operator revokes team 2 and replaces the CRL file in place.
    let mut serials = venue.revoked_serials.clone();
    serials.push(venue.team2_serial);
    let crl = venue.authority.revocation_list(&serials)?;
    let staged = venue.revocation_list.with_extension("crl.new");
    std::fs::write(&staged, crl).map_err(|error| error.to_string())?;
    std::fs::rename(&staged, &venue.revocation_list).map_err(|error| error.to_string())?;
    // Team 2's live session is told why and closed (the client surfaces
    // the venue's logout reason as the close).
    let closed = until(&team2, |_| false).err().unwrap_or_default();
    assert!(closed.contains("no longer trusted"), "{closed}");
    // A new handshake with the revoked certificate fails.
    let deadline = Instant::now() + TIMEOUT;
    while Client::connect(&venue.config(&venue.team2, None)).is_ok() {
        assert!(
            Instant::now() < deadline,
            "revoked certificate still accepted"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // A damaged CRL file is not applied: the previous trust, revocation
    // included, stays in force.
    std::fs::write(&venue.revocation_list, "not a CRL").map_err(|error| error.to_string())?;
    std::thread::sleep(Duration::from_millis(1_500));
    assert!(Client::connect(&venue.config(&venue.team2, None)).is_err());
    // Team 1 is unaffected.
    team1
        .send(&ClientMessage::Ping { nonce: 9 })
        .map_err(|error| error.to_string())?;
    until(&team1, |message| {
        matches!(message, ServerMessage::Pong { nonce: 9, .. })
    })?;
    Ok(())
}

#[test]
fn idle_unauthenticated_sockets_cannot_take_the_registered_teams_slots() -> Result<(), String> {
    let venue = start_venue("idle-sockets")?;
    // Four times max_connections in sockets that never start TLS: before
    // the fix they filled every session slot and its waiting area.
    let mut idle: Vec<TcpStream> = (0..12)
        .map(|_| TcpStream::connect(("127.0.0.1", venue.bnp_port)))
        .collect::<Result<_, _>>()
        .map_err(|error| error.to_string())?;
    let started = Instant::now();
    let team1 = venue.connect(&venue.team1, None)?;
    let team2 = venue.connect(&venue.team2, None)?;
    for (nonce, team) in [(1, &team1), (2, &team2)] {
        team.send(&ClientMessage::Ping { nonce })
            .map_err(|error| error.to_string())?;
        until(
            team,
            |message| matches!(message, ServerMessage::Pong { nonce: n, .. } if *n == nonce),
        )?;
    }
    // The venue closes each idle socket at the handshake deadline.
    let first = idle.first_mut().ok_or("no idle socket")?;
    first
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|error| error.to_string())?;
    let mut byte = [0_u8; 1];
    let read = std::io::Read::read(first, &mut byte);
    assert!(
        matches!(read, Ok(0))
            || matches!(&read, Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset),
        "idle socket not closed: {read:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(10));
    Ok(())
}

#[test]
fn a_kill_switch_cancels_and_blocks_only_its_own_team_until_reset() -> Result<(), String> {
    let venue = start_venue("kill-switch")?;
    let team1 = venue.connect(&venue.team1, None)?;
    let team2 = venue.connect(&venue.team2, None)?;
    let send = |client: &Client, message: ClientMessage| {
        client.send(&message).map_err(|error| error.to_string())
    };
    send(&team1, limit(1, Side::Buy, 1, 91))?;
    until(&team1, |message| {
        matches!(
            message,
            ServerMessage::OrderRested {
                client_order_id: 1,
                ..
            }
        )
    })?;
    send(&team1, ClientMessage::KillSwitch { request_id: 1 })?;
    let reports = until(&team1, |message| {
        matches!(message, ServerMessage::KillSwitchActivated { .. })
    })?;
    // Its own resting order is canceled in the same commit.
    let reports = [
        reports,
        until(&team1, |message| {
            matches!(message, ServerMessage::OrderCanceled { .. })
        })?,
    ]
    .concat();
    assert!(reports.iter().any(|message| matches!(
        message,
        ServerMessage::OrderCanceled {
            client_order_id: 1,
            reason: CancelReason::KillSwitch,
            ..
        }
    )));
    // Team 1 is refused; team 2 still trades.
    send(&team1, limit(2, Side::Buy, 1, 91))?;
    until(&team1, |message| {
        matches!(
            message,
            ServerMessage::OrderRejected {
                client_order_id: 2,
                reason: RejectReason::KillSwitchActive,
                ..
            }
        )
    })?;
    send(&team2, limit(5, Side::Buy, 1, 90))?;
    until(&team2, |message| {
        matches!(
            message,
            ServerMessage::OrderRested {
                client_order_id: 5,
                ..
            }
        )
    })?;
    // After a reset team 1 trades again.
    send(&team1, ClientMessage::KillSwitchReset { request_id: 1 })?;
    until(&team1, |message| {
        matches!(message, ServerMessage::KillSwitchReset { .. })
    })?;
    send(&team1, limit(3, Side::Buy, 1, 91))?;
    until(&team1, |message| {
        matches!(
            message,
            ServerMessage::OrderRested {
                client_order_id: 3,
                ..
            }
        )
    })?;
    Ok(())
}

/// The hub sits with team 2 and venue 1 is 30 ms away, so a report from
/// the hub can be due before an earlier report from the venue.
fn hub_near_team_map() -> LatencyMap {
    let at = |location: &str| location.to_owned();
    LatencyMap {
        default_location: at("two"),
        participants: vec![ParticipantPlacement {
            participant_id: ParticipantId::new(10),
            location: at("one"),
        }],
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

#[test]
fn private_reports_arrive_in_sequence_order_so_resume_never_skips_one() -> Result<(), String> {
    let venue = start_venue_with("ordering", hub_near_team_map())?;
    let mut maker = venue.maker()?;
    let team = venue.connect(&venue.team2, None)?;
    team.send(&limit(1, Side::Buy, 1, 95))
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
    // The maker fills the bid at the venue; a moment later the team's kill
    // switch commits at the hub, beside the team. The kill switch is due
    // first, but the fill was committed first.
    maker.send(order("903", "sell", 1, Some(95)))?;
    maker.wait_report("903", "F")?;
    team.send(&ClientMessage::KillSwitch { request_id: 1 })
        .map_err(|error| error.to_string())?;
    let received = until(&team, |message| {
        matches!(message, ServerMessage::KillSwitchActivated { .. })
    })?;
    let sequences: Vec<u64> = received
        .iter()
        .filter_map(|message| message.stamp().map(|stamp| stamp.sequence))
        .collect();
    assert!(
        sequences.windows(2).all(|pair| pair[0] <= pair[1]),
        "private stream out of order: {received:?}"
    );
    assert!(
        received.iter().any(|message| matches!(
            message,
            ServerMessage::Fill {
                client_order_id: 1,
                ..
            }
        )),
        "the fill must arrive before the later kill switch: {received:?}"
    );
    // Reconnecting from the cursor misses nothing: the replay is empty.
    let cursor = team.cursor();
    team.close();
    let resumed = venue.connect(&venue.team2, Some(cursor))?;
    let mut replay = until(&resumed, |message| {
        matches!(message, ServerMessage::ReplayComplete { .. })
    })?;
    // The measured access delay is a session message and may come first.
    replay.retain(|message| !matches!(message, ServerMessage::AccessDelay { .. }));
    assert!(
        matches!(replay.as_slice(), [ServerMessage::ReplayComplete { through_sequence }] if *through_sequence == cursor),
        "{replay:?}"
    );
    Ok(())
}

/// Applies every message to both books until `done` holds for them.
fn follow(
    client: &Client,
    book: &mut FeedBook,
    tape: &mut TapeBook,
    seen: &mut Vec<ServerMessage>,
    done: impl Fn(&FeedBook, &TapeBook) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + TIMEOUT;
    while !done(book, tape) {
        if Instant::now() > deadline {
            return Err(format!("timed out; received {seen:?}"));
        }
        if let Some(message) = client
            .recv_timeout(Duration::from_millis(50))
            .map_err(|error| error.to_string())?
        {
            book.apply(&message).map_err(|error| error.to_string())?;
            tape.apply(&message).map_err(|error| error.to_string())?;
            seen.push(message);
        }
    }
    Ok(())
}

#[test]
fn the_order_by_order_feed_and_the_tape_show_queues_trades_and_venues() -> Result<(), String> {
    let venue = start_venue("l3-tape")?;
    let mut maker = venue.maker()?;
    let team = venue.connect(&venue.team1, None)?;
    // The venue publishes its measurement of this connection's delay.
    until(&team, |message| {
        matches!(
            message,
            ServerMessage::AccessDelay {
                min_rtt_us: Some(_),
                ..
            }
        )
    })?;
    maker.send(order("901", "sell", 5, Some(100)))?;
    maker.wait_report("901", "0")?;
    maker.send(order("902", "sell", 3, Some(100)))?;
    maker.wait_report("902", "0")?;
    let send = |message: ClientMessage| team.send(&message).map_err(|error| error.to_string());
    send(ClientMessage::Subscribe {
        request_id: 7,
        listing: LISTING,
        flags: FeedFlags::ALL.union(FeedFlags::ORDERS),
    })?;
    // Venue 0 names the instrument's consolidated tape.
    send(ClientMessage::Subscribe {
        request_id: 8,
        listing: Listing {
            venue_id: 0,
            instrument_id: 1,
        },
        flags: FeedFlags::ALL,
    })?;
    let (mut book, mut tape) = (FeedBook::new(7), TapeBook::new(8));
    let mut seen = Vec::new();
    follow(&team, &mut book, &mut tape, &mut seen, |book, tape| {
        book.is_ready() && tape.is_ready()
    })?;
    // Two offers at 100, in time priority: the second waits behind 5.
    let queue = book.orders(Side::Sell);
    assert_eq!(
        queue
            .iter()
            .map(|order| (
                order.quantity,
                order.position_at_price,
                order.quantity_ahead
            ))
            .collect::<Vec<_>>(),
        vec![(5, 1, 0), (3, 2, 5)]
    );
    assert_eq!(
        tape.quote(1, Side::Sell)
            .map(|level| (level.price, level.quantity)),
        Some((100, 8))
    );
    // A buy of 6 takes the first offer and 1 of the second.
    send(limit(1, Side::Buy, 6, 100))?;
    seen.clear();
    follow(&team, &mut book, &mut tape, &mut seen, |book, tape| {
        book.trades().len() == 2 && tape.trades().len() == 2
    })?;
    let resting: Vec<Option<u64>> = seen
        .iter()
        .filter_map(|message| match message {
            ServerMessage::OrderUpdate { entries, .. } => Some(entries),
            _ => None,
        })
        .flatten()
        .filter_map(|entry| match entry {
            OrderEntry::Trade {
                resting_reference, ..
            } => Some(*resting_reference),
            OrderEntry::Order { .. } => None,
        })
        .collect();
    assert_eq!(
        resting,
        vec![Some(queue[0].reference), Some(queue[1].reference)],
        "each trade names the resting order it executed against"
    );
    let left = book.orders(Side::Sell);
    assert_eq!(left.len(), 1);
    assert_eq!(
        (left[0].reference, left[0].quantity, left[0].quantity_ahead),
        (queue[1].reference, 2, 0)
    );
    assert!(tape.trades().iter().all(|trade| trade.venue_id == 1));
    assert_eq!(
        tape.quote(1, Side::Sell)
            .map(|level| (level.price, level.quantity)),
        Some((100, 2))
    );
    Ok(())
}

#[test]
fn an_instructor_runs_the_round_and_teams_see_it_in_their_stream() -> Result<(), String> {
    let venue = start_venue("operator")?;
    let instructor = venue.connect(&venue.instructor, None)?;
    let team = venue.connect(&venue.team1, None)?;
    let command = |request_id, action| {
        instructor
            .send(&ClientMessage::OperatorCommand { request_id, action })
            .map_err(|error| error.to_string())?;
        until(&instructor, |message| {
            matches!(message, ServerMessage::OperatorDone { request_id: done, .. } if *done == request_id)
        })
        .map(|_| ())
    };
    command(1, OperatorAction::Pause)?;
    command(2, OperatorAction::Resume)?;
    command(
        3,
        OperatorAction::Halt {
            instrument_id: 1,
            halted: true,
            reason: "pending news".to_owned(),
        },
    )?;
    command(
        4,
        OperatorAction::PublishNews {
            news_id: 1,
            audience: NewsAudience::Participant(2),
            headline: "for team 2 only".to_owned(),
            body: String::new(),
        },
    )?;
    command(
        5,
        OperatorAction::PublishNews {
            news_id: 2,
            audience: NewsAudience::Public,
            headline: "BNT earnings".to_owned(),
            body: "Revenue up 4%".to_owned(),
        },
    )?;
    command(
        6,
        OperatorAction::Fine {
            participant_id: 1,
            currency_id: 1,
            amount: 250,
            reason: "late report".to_owned(),
        },
    )?;
    // The team sees each public fact in commit order, and not news meant
    // for another team.
    let stream = until(&team, |message| {
        matches!(message, ServerMessage::News { news_id: 2, .. })
    })?;
    let run: Vec<&ServerMessage> = stream
        .iter()
        .filter(|message| {
            matches!(
                message,
                ServerMessage::RunStatus { .. }
                    | ServerMessage::ListingHalt { .. }
                    | ServerMessage::News { .. }
            )
        })
        .collect();
    assert!(
        matches!(
            run.as_slice(),
            [
                ServerMessage::RunStatus { status: paused, .. },
                ServerMessage::RunStatus { status: active, .. },
                ServerMessage::ListingHalt {
                    instrument_id: 1,
                    halted: true,
                    ..
                },
                ServerMessage::News { headline, body, .. },
            ] if paused == "paused" && active == "active" && headline == "BNT earnings" && body == "Revenue up 4%"
        ),
        "{run:?}"
    );
    let sequences = run.iter().filter_map(|message| message.stamp());
    assert!(
        sequences.is_sorted_by_key(|stamp| stamp.sequence),
        "{run:?}"
    );
    // The fine is in the team's ledger.
    team.send(&ClientMessage::AccountRequest { request_id: 1 })
        .map_err(|error| error.to_string())?;
    let account = until(&team, |message| {
        matches!(message, ServerMessage::Account { .. })
    })?;
    assert!(
        account.iter().any(|message| matches!(
            message,
            ServerMessage::Account { cash, .. }
                if cash.iter().any(|balance| balance.currency_id == 1 && balance.balance == 1_000_000_000 - 250)
        )),
        "{account:?}"
    );
    Ok(())
}

#[test]
fn a_team_cannot_run_the_round_and_an_instructor_cannot_trade() -> Result<(), String> {
    let venue = start_venue("operator-refusals")?;
    let instructor = venue.connect(&venue.instructor, None)?;
    let team = venue.connect(&venue.team1, None)?;
    team.send(&ClientMessage::OperatorCommand {
        request_id: 7,
        action: OperatorAction::Pause,
    })
    .map_err(|error| error.to_string())?;
    until(&team, |message| {
        matches!(
            message,
            ServerMessage::Reject { request_type, reference: 7, .. }
                if *request_type == msg_type::OPERATOR_COMMAND
        )
    })?;
    instructor
        .send(&limit(1, Side::Buy, 1, 90))
        .map_err(|error| error.to_string())?;
    until(&instructor, |message| {
        matches!(
            message,
            ServerMessage::Reject { request_type, reason, .. }
                if *request_type == msg_type::NEW_ORDER && reason.contains("do not trade")
        )
    })?;
    Ok(())
}

#[test]
fn a_resting_report_names_the_orders_place_on_the_order_by_order_feed() -> Result<(), String> {
    let venue = start_venue("own-reference")?;
    let mut maker = venue.maker()?;
    let team = venue.connect(&venue.team1, None)?;
    team.send(&ClientMessage::Subscribe {
        request_id: 7,
        listing: LISTING,
        flags: FeedFlags::ALL.union(FeedFlags::ORDERS),
    })
    .map_err(|error| error.to_string())?;
    let (mut book, mut tape) = (FeedBook::new(7), TapeBook::new(8));
    let mut seen = Vec::new();
    follow(&team, &mut book, &mut tape, &mut seen, |book, _| {
        book.is_ready()
    })?;
    maker.send(order("901", "buy", 5, Some(99)))?;
    maker.wait_report("901", "0")?;
    team.send(&limit(1, Side::Buy, 3, 99))
        .map_err(|error| error.to_string())?;
    // The report names the order's public reference, so the team finds
    // its own place in the queue without guessing.
    let mut reference = None;
    let deadline = Instant::now() + TIMEOUT;
    while reference.is_none_or(|reference| book.order(reference).is_none()) {
        assert!(Instant::now() < deadline, "received {seen:?}");
        if let Some(message) = team
            .recv_timeout(Duration::from_millis(50))
            .map_err(|error| error.to_string())?
        {
            book.apply(&message).map_err(|error| error.to_string())?;
            if let ServerMessage::OrderRested {
                client_order_id: 1,
                reference: rested,
                ..
            } = &message
            {
                reference = *rested;
            }
            seen.push(message);
        }
    }
    let place = reference
        .and_then(|reference| book.order(reference))
        .ok_or("no queue place")?;
    assert_eq!(
        (
            place.quantity,
            place.position_at_price,
            place.quantity_ahead
        ),
        (3, 2, 5)
    );
    Ok(())
}
