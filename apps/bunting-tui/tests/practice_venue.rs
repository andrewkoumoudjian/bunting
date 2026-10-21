//! The local practice venue keeps running while its built-in agents quote
//! for thousands of wakes. Before slice 51 the agents woke every server
//! tick instead of on the run clock and never retired an order, so each
//! one piled up good-till-cancel orders until its own open-order limit
//! (1,024) refused one, and that refusal ended the whole venue after about
//! a minute (`deterministic runtime failed: Agent("wake:
//! Execution(OpenOrderLimit)")`).
//!
//! The venue here is the practice venue `bunting tui --local` and
//! `bunting-desk --local` serve, with its clock paced at 20 times real time
//! and a 5 ms agent tick: each agent wakes every 25 ms (half a second of
//! run time), so the market maker places more than the old limit of
//! orders, and several times its QUARCC state's order memory, during the
//! run. The old scheduler would have woken every tick and stopped within
//! about three seconds.

use bunting_server::config::ServerConfig;
use bunting_trading_state::bunting_client::bnp_wire::{Listing, Side};
use bunting_trading_state::local_venue;
use bunting_trading_state::{FeedState, Model, Session};
use serde_json::Value;
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

const BNT: Listing = Listing {
    venue_id: 1,
    instrument_id: 1,
};
/// Run time per venue time: 20x.
const PACE: u64 = 20;
/// How long the agents run: about 600 wakes each.
const RUN: Duration = Duration::from_secs(15);
const TIMEOUT: Duration = Duration::from_secs(20);

fn edit(path: &Path, change: impl FnOnce(&mut Value)) -> Result<(), String> {
    let mut value: Value = serde_json::from_str(
        &std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?,
    )
    .map_err(|e| e.to_string())?;
    change(&mut value);
    std::fs::write(path, value.to_string()).map_err(|e| e.to_string())
}

fn pump(session: &Session, model: &mut Model, started: Instant) {
    while let Some((event, received)) = session.event_timeout(Duration::from_millis(5)) {
        let at = u64::try_from(received.saturating_duration_since(started).as_micros())
            .unwrap_or(u64::MAX);
        for request in model.apply(event, at) {
            let _ = session.send(&request);
        }
    }
    let now = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    for request in model.tick(now) {
        let _ = session.send(&request);
    }
}

#[test]
fn the_practice_venue_keeps_running_while_its_agents_quote() -> Result<(), String> {
    let directory =
        std::env::temp_dir().join(format!("bunting-practice-endurance-{}", std::process::id()));
    let prepared = local_venue::prepare(&directory).map_err(|e| e.to_string())?;
    edit(&directory.join("scenario.json"), |scenario| {
        scenario["simulation"]["clock"]["step_ns"] = Value::from(PACE * 1_000_000);
    })?;
    edit(&prepared.config_path, |config| {
        config["runtime"]["wall_tick_ms"] = Value::from(5);
        // This is a 20x stress of one book: keep BNT on venue 1's agents so
        // an unoptimized build can keep up with the 5 ms tick.
        if let Some(agents) = config["runtime"]["scheduler"]["agents"].as_array_mut() {
            agents.retain(|agent| agent["instrument_id"] == 1 && agent["venue_id"] == 1);
        }
        config["runtime"]["scheduler"]["other_instruments"] = Value::Array(Vec::new());
    })?;
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
    let session = Session::start(prepared.client.clone()).map_err(|e| e.to_string())?;
    let mut model = Model::new(1);
    let started = Instant::now();
    while started.elapsed() < RUN {
        if server.is_finished() {
            return Err(format!(
                "the venue stopped after {:?}: {:?}",
                started.elapsed(),
                server.join()
            ));
        }
        pump(&session, &mut model, started);
    }
    // Still quoting at the end, and only a handful of live orders: each
    // agent's order rests two of its wakes, then expires.
    let deadline = Instant::now() + TIMEOUT;
    loop {
        pump(&session, &mut model, started);
        let book = model
            .feed(BNT)
            .filter(|feed| feed.state == FeedState::Live)
            .map(|feed| &feed.book);
        if let Some(book) = book
            && book.best_bid().is_some()
            && book.best_ask().is_some()
        {
            let resting = book.orders(Side::Buy).len() + book.orders(Side::Sell).len();
            assert!(
                resting <= 24,
                "agents' orders must not pile up: {resting} resting on BNT"
            );
            break;
        }
        if Instant::now() > deadline {
            return Err(format!(
                "no live BNT quotes after the run: feed {:?}; notices {:?}",
                model.feed(BNT).map(|feed| (
                    feed.state.clone(),
                    feed.book.orders(Side::Buy).len(),
                    feed.book.orders(Side::Sell).len()
                )),
                model.notices()
            ));
        }
    }
    assert!(!server.is_finished(), "the venue is still running");
    let _ = std::fs::remove_dir_all(&directory);
    Ok(())
}
