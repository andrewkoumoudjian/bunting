//! The terminal loop: venue events into the model, keys into requests,
//! and a redraw whenever something changed.

use crate::connect::Target;
use crate::ui::{Effect, Ui};
use crate::view::{Screen, render};
use bunting_trading_state::bunting_client::bnp_wire::ClientMessage;
use bunting_trading_state::{Model, Session};
use ratatui::crossterm::event::{self, Event};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Venue events applied before the screen is drawn again.
const EVENTS_PER_FRAME: usize = 4_096;
/// Longest wait for a key before applying venue events again.
const FRAME: Duration = Duration::from_millis(10);
/// Least time between two redraws.
const REDRAW: Duration = Duration::from_millis(16);

pub fn run(target: Target) -> Result<(), String> {
    let Target {
        client,
        label,
        local,
    } = target;
    let session = Session::start(client).map_err(|error| error.to_string())?;
    let mut terminal = ratatui::try_init().map_err(|error| error.to_string())?;
    let result = drive(&mut terminal, &session, &label);
    ratatui::restore();
    // Log out before the local venue (if any) stops.
    drop(session);
    drop(local);
    result
}

fn drive(
    terminal: &mut ratatui::DefaultTerminal,
    session: &Session,
    label: &str,
) -> Result<(), String> {
    let started = Instant::now();
    let now = || u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    // Client order IDs must never repeat within a run, across restarts:
    // start from the wall clock in microseconds, as bunting-trader does.
    let first_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(1, |elapsed| {
            u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
        });
    let mut model = Model::new(first_id);
    let mut ui = Ui::default();
    let mut dirty = true;
    let mut last_draw = Instant::now();
    loop {
        for _ in 0..EVENTS_PER_FRAME {
            let Some((event, received)) = session.try_event() else {
                break;
            };
            let at = u64::try_from(received.saturating_duration_since(started).as_micros())
                .unwrap_or(u64::MAX);
            let requests = model.apply(event, at);
            send_all(session, &mut model, requests, now());
            dirty = true;
        }
        let requests = model.tick(now());
        send_all(session, &mut model, requests, now());
        let since = last_draw.elapsed();
        if (dirty && since >= REDRAW) || since >= Duration::from_millis(250) {
            let screen = Screen {
                model: &model,
                ui: &ui,
                label,
                now_us: now(),
            };
            terminal
                .draw(|frame| render(frame, &screen))
                .map_err(|error| error.to_string())?;
            dirty = false;
            last_draw = Instant::now();
        }
        if event::poll(FRAME).map_err(|error| error.to_string())? {
            match event::read().map_err(|error| error.to_string())? {
                Event::Key(key) => {
                    for effect in ui.handle(key, &mut model, now()) {
                        match effect {
                            Effect::Quit => return Ok(()),
                            Effect::Send(message) => send(session, &mut model, &message, now()),
                        }
                    }
                    dirty = true;
                }
                Event::Resize(..) => dirty = true,
                _ => {}
            }
        }
    }
}

fn send(session: &Session, model: &mut Model, message: &ClientMessage, now_us: u64) {
    if let Err(error) = session.send(message) {
        model.send_failed(message, &error.to_string(), now_us);
    }
}

fn send_all(session: &Session, model: &mut Model, messages: Vec<ClientMessage>, now_us: u64) {
    for message in messages {
        send(session, model, &message, now_us);
    }
}
