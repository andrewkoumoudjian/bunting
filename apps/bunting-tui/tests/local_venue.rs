//! The terminal's state against a real venue: the local practice venue's
//! generated certificates and configuration, served by the real server,
//! with built-in agents quoting BNT on venue 1, and QRC listed on venue 1
//! and on venue 2 two milliseconds away. Orders go to the venue the trader
//! chooses and the far venue answers later; order-by-order books, each
//! instrument's tape, the venue's measured access delay, fills, cancels,
//! the account and the kill switch come back over BNP.

use bunting_server::config::ServerConfig;
use bunting_trading_state::bunting_client::bnp_wire::{CancelReason, ClientMessage, Listing, Side};
use bunting_trading_state::local_venue::{self, FAR_VENUE_US};
use bunting_trading_state::{FeedState, Model, OrderStatus, OrderTicket, Session, TicketTif};
use std::net::TcpStream;
use std::time::{Duration, Instant};

const BNT: Listing = Listing {
    venue_id: 1,
    instrument_id: 1,
};
const QRC_NEAR: Listing = Listing {
    venue_id: 1,
    instrument_id: 2,
};
const QRC_FAR: Listing = Listing {
    venue_id: 2,
    instrument_id: 2,
};
const TIMEOUT: Duration = Duration::from_secs(20);

struct Trader {
    session: Session,
    model: Model,
    started: Instant,
}

impl Trader {
    fn now(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_micros()).unwrap_or(u64::MAX)
    }

    fn send(&mut self, message: &ClientMessage) {
        if let Err(error) = self.session.send(message) {
            let now = self.now();
            self.model.send_failed(message, &error.to_string(), now);
        }
    }

    /// Applies venue events until `done` holds.
    fn until(&mut self, what: &str, done: impl Fn(&Model) -> bool) -> Result<(), String> {
        let deadline = Instant::now() + TIMEOUT;
        while !done(&self.model) {
            if Instant::now() > deadline {
                return Err(format!(
                    "timed out waiting for {what}; orders {:?}; notices {:?}",
                    self.model.orders(),
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

fn status(model: &Model, client_order_id: u64) -> Option<OrderStatus> {
    model
        .orders()
        .iter()
        .find(|row| row.client_order_id == client_order_id)
        .map(|row| row.status.clone())
}

#[test]
fn the_terminal_trades_on_the_venue_it_chooses_over_bnp() -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("bunting-tui-local-{}", std::process::id()));
    let prepared = local_venue::prepare(&directory).map_err(|e| e.to_string())?;
    let config = ServerConfig::from_file(&prepared.config_path).map_err(|e| e.to_string())?;
    let server = std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", prepared.bnp_port)).is_err() {
        if server.is_finished() {
            return Err(format!("venue stopped: {:?}", server.join()));
        }
        if Instant::now() > deadline {
            return Err("venue did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut trader = Trader {
        session: Session::start(prepared.client.clone()).map_err(|e| e.to_string())?,
        model: Model::new(1),
        started: Instant::now(),
    };
    trader.until("every listing's book and an agent's ask", |model| {
        model.is_connected()
            && [QRC_NEAR, QRC_FAR].iter().all(|&listing| {
                model
                    .feed(listing)
                    .is_some_and(|f| f.state == FeedState::Live)
            })
            && model.feed(BNT).and_then(|f| f.book.best_ask()).is_some()
    })?;
    // Venue feeds are order by order: an agent's ask is a queued order.
    let bnt = trader.model.feed(BNT).ok_or("BNT feed")?;
    assert!(bnt.order_by_order, "venue feeds are order by order");
    assert!(!bnt.book.orders(Side::Sell).is_empty(), "agents' asks");
    trader.until("each instrument's tape", |model| {
        [1, 2].iter().all(|&instrument| {
            model
                .tape(instrument)
                .is_some_and(|t| t.state == FeedState::Live)
        })
    })?;
    trader.until("the venue's access delay", |model| {
        model.access_delay().is_some_and(|d| d.min_rtt_us.is_some())
    })?;

    // A marketable IOC buy on venue 1 against an agent's ask.
    let ask = trader
        .model
        .feed(BNT)
        .and_then(|f| f.book.best_ask())
        .ok_or("ask")?;
    let mut buy = OrderTicket::new(Some(BNT), Side::Buy);
    buy.price = ask.price + 5;
    buy.tif = TicketTif::Ioc;
    let taker = trader.place(&buy)?;
    trader.until("the fill", |model| {
        model
            .fills()
            .iter()
            .any(|fill| fill.client_order_id == taker)
    })?;

    // The same bid on each venue: venue 2 is two milliseconds away each way.
    let mut bid = OrderTicket::new(Some(QRC_NEAR), Side::Buy);
    bid.price = 1;
    let near_bid = trader.place(&bid)?;
    bid.listing = Some(QRC_FAR);
    let resting = trader.place(&bid)?;
    trader.until("both bids to rest", |model| {
        status(model, near_bid) == Some(OrderStatus::Resting)
            && status(model, resting) == Some(OrderStatus::Resting)
    })?;
    let trips = trader.model.venue_round_trips();
    let near = trips.get(&1).map(|t| t.last_us).ok_or("near round trip")?;
    let far = trips.get(&2).map(|t| t.last_us).ok_or("far round trip")?;
    assert!(
        far >= 2 * FAR_VENUE_US && far > near,
        "venue 2 answers after the distance both ways: near {near} µs, far {far} µs"
    );
    trader.until("the far order on venue 2's book", |model| {
        model
            .feed(QRC_FAR)
            .and_then(|f| f.book.best_bid())
            .is_some_and(|level| level.price == 1)
    })?;
    // The only bid on venue 1's QRC book is first in its queue, and the
    // tape shows a bid of 1 on both venues.
    trader.until("the near order first in its queue", |model| {
        model
            .feed(QRC_NEAR)
            .map(|f| f.book.orders(Side::Buy))
            .is_some_and(|orders| {
                orders
                    .iter()
                    .any(|o| o.price == 1 && o.position_at_price == 1 && o.quantity_ahead == 0)
            })
    })?;
    trader.until("both bids on the tape", |model| {
        model.tape(2).is_some_and(|tape| {
            [1, 2].iter().all(|&venue| {
                tape.book
                    .quote(venue, Side::Buy)
                    .is_some_and(|level| level.price == 1)
            })
        })
    })?;

    let cancel = trader.model.cancel(resting).ok_or("cancelable")?;
    trader.send(&cancel);
    trader.until("the cancel", |model| {
        status(model, resting) == Some(OrderStatus::Canceled(CancelReason::Requested))
    })?;

    trader.until("the account to show the bought lot", |model| {
        model.account().is_some_and(|account| {
            account
                .positions
                .iter()
                .any(|position| position.instrument_id == 1 && position.position >= 1)
        })
    })?;

    let now = trader.now();
    let kill = trader.model.kill_switch(now);
    trader.send(&kill);
    // The venue reports the kill switch; the model shows it from that
    // report, never from having sent it.
    assert!(!trader.model.kill_switch_active());
    trader.until("the kill switch", Model::kill_switch_active)?;
    // It cancels this participant's resting orders.
    trader.until("the kill switch cancel", |model| {
        status(model, near_bid) == Some(OrderStatus::Canceled(CancelReason::KillSwitch))
    })?;
    let now = trader.now();
    let reset = trader.model.reset_kill_switch(now);
    trader.send(&reset);
    trader.until("the kill switch reset", |model| !model.kill_switch_active())?;
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}
