//! The shared local venue through its join-or-start path, against the
//! real `bunting` server binary: the first client starts one market of
//! eight participants, a second joins it as a different participant, a
//! resting order from one is visible in the other's public book, the
//! market survives the starter leaving, and the last leaver stops the
//! server.
//!
//! The server binary is the workspace's own `bunting` (or
//! `bunting-server`) build next to this test executable, or
//! `$BUNTING_TEST_SERVER_BIN`. Without one (a `bunting-tui`-only build)
//! the test reports why and skips; the workspace test gate builds it.

use bunting_trading_state::bunting_client::bnp_wire::{ClientMessage, Listing, Side};
use bunting_trading_state::local_venue::{self, ServerProgram, SharedVenue};
use bunting_trading_state::{Connection, FeedState, Model, OrderStatus, OrderTicket, Session};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const QRC_NEAR: Listing = Listing {
    venue_id: 1,
    instrument_id: 2,
};
const TIMEOUT: Duration = Duration::from_secs(20);

fn server_program() -> Option<ServerProgram> {
    let mut candidates: Vec<PathBuf> = std::env::var_os("BUNTING_TEST_SERVER_BIN")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    if let Some(binaries) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().and_then(Path::parent).map(Path::to_path_buf))
    {
        for name in ["bunting", "bunting-server"] {
            candidates.push(binaries.join(format!("{name}{}", std::env::consts::EXE_SUFFIX)));
        }
    }
    candidates
        .into_iter()
        .find(|path| path.is_file())
        .map(|program| {
            let arguments = if program
                .file_stem()
                .is_some_and(|stem| stem.eq_ignore_ascii_case("bunting"))
            {
                vec!["server".into()]
            } else {
                Vec::new()
            };
            ServerProgram { program, arguments }
        })
}

struct Trader {
    session: Session,
    model: Model,
    started: Instant,
}

impl Trader {
    fn connect(venue: &SharedVenue, first_id: u64) -> Result<Self, String> {
        Ok(Self {
            session: Session::start(venue.client.clone()).map_err(|e| e.to_string())?,
            model: Model::new(first_id),
            started: Instant::now(),
        })
    }

    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn send(&mut self, message: &ClientMessage) {
        if let Err(error) = self.session.send(message) {
            let now = self.now();
            self.model.send_failed(message, &error.to_string(), now);
        }
    }

    fn until(&mut self, what: &str, done: impl Fn(&Model) -> bool) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&self.model) {
            if Instant::now() > deadline {
                return Err(format!(
                    "timed out waiting for {what}; connection {:?}; notices {:?}",
                    self.model.connection(),
                    self.model.notices()
                ));
            }
            if let Some((event, received)) = self.session.event_timeout(Duration::from_millis(10)) {
                let at =
                    u64::try_from(received.saturating_duration_since(self.started).as_micros())
                        .unwrap_or(u64::MAX);
                for request in self.model.apply(event, at) {
                    self.send(&request);
                }
            }
            let now = self.now();
            for request in self.model.tick(now) {
                self.send(&request);
            }
        }
        Ok(())
    }

    fn participant_id(&self) -> Option<u128> {
        match self.model.connection() {
            Connection::Connected { welcome, .. } => Some(welcome.participant_id),
            _ => None,
        }
    }

    fn place(&mut self, ticket: &OrderTicket) -> Result<u64, String> {
        let now = self.now();
        let message = self.model.place(ticket, now).map_err(|e| e.to_string())?;
        let ClientMessage::NewOrder(order) = &message else {
            return Err("not an order".to_owned());
        };
        let id = order.client_order_id;
        self.send(&message);
        Ok(id)
    }
}

fn read_pid(directory: &Path) -> Result<u64, String> {
    let value: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(directory.join("venue.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    value["pid"]
        .as_u64()
        .ok_or_else(|| "venue.json pid".to_owned())
}

#[test]
fn two_clients_share_one_local_market_and_the_last_leaver_stops_it() -> Result<(), String> {
    let Some(program) = server_program() else {
        eprintln!(
            "skipping: no `bunting` server binary beside the test executable \
             (build the workspace, or set BUNTING_TEST_SERVER_BIN)"
        );
        return Ok(());
    };
    let directory =
        std::env::temp_dir().join(format!("bunting-shared-venue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);

    // First client starts the market and claims participant 1; the
    // second joins the same server as participant 2.
    let venue_a =
        local_venue::join_or_start(&directory, &program, None).map_err(|e| e.to_string())?;
    assert!(venue_a.started_server, "the first client starts the server");
    assert_eq!(venue_a.participant_id, 1);
    assert_eq!(venue_a.label(), "Participant 1 · local venue");
    let pid = read_pid(&directory)?;
    let bnp_port = venue_a.prepared.bnp_port;

    let venue_b =
        local_venue::join_or_start(&directory, &program, None).map_err(|e| e.to_string())?;
    assert!(
        !venue_b.started_server,
        "the second client joins, not starts"
    );
    assert_eq!(venue_b.participant_id, 2);
    assert_eq!(venue_b.prepared.bnp_port, bnp_port, "one shared server");
    assert_eq!(read_pid(&directory)?, pid, "the same server process");

    // Both connect over BNP as different participants.
    let mut trader_a = Trader::connect(&venue_a, 1_000_000)?;
    let mut trader_b = Trader::connect(&venue_b, 2_000_000)?;
    trader_a.until(
        "participant 1 connected",
        bunting_trading_state::Model::is_connected,
    )?;
    trader_b.until("participant 2 connected with a live QRC feed", |model| {
        model.is_connected()
            && model
                .feed(QRC_NEAR)
                .is_some_and(|feed| feed.state == FeedState::Live)
    })?;
    assert_eq!(trader_a.participant_id(), Some(1));
    assert_eq!(trader_b.participant_id(), Some(2));

    // A resting bid from participant 1 appears in participant 2's
    // public book. It sits well below the agents' quotes, so it rests.
    let mut bid = OrderTicket::new(Some(QRC_NEAR), Side::Buy);
    bid.price = 90;
    bid.quantity = 3;
    let resting = trader_a.place(&bid)?;
    trader_a.until("participant 1's bid resting", |model| {
        model
            .orders()
            .iter()
            .any(|row| row.client_order_id == resting && row.status == OrderStatus::Resting)
    })?;
    trader_b.until("the bid in participant 2's public book", |model| {
        model
            .feed(QRC_NEAR)
            .is_some_and(|feed| feed.book.bids().iter().any(|level| level.price == 90))
    })?;

    // The starter leaving must not kill the market for the other.
    drop(trader_a);
    drop(venue_a);
    assert!(
        TcpStream::connect(("127.0.0.1", bnp_port)).is_ok(),
        "the server outlives the client that started it"
    );
    let mut own_bid = OrderTicket::new(Some(QRC_NEAR), Side::Buy);
    own_bid.price = 91;
    let own = trader_b.place(&own_bid)?;
    trader_b.until(
        "participant 2 still trading after the starter left",
        |model| {
            model
                .orders()
                .iter()
                .any(|row| row.client_order_id == own && row.status == OrderStatus::Resting)
        },
    )?;

    // The last client leaving stops the server.
    drop(trader_b);
    drop(venue_b);
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", bnp_port)).is_ok() {
        if Instant::now() > deadline {
            return Err("the server kept running after its last client left".to_owned());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !directory.join("venue.json").exists(),
        "the venue record is cleared when the server stops"
    );
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}
