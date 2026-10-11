//! The app's state: the shared trading [`Model`] plus what only this window
//! needs (the selected instrument and venue). Panels read it and call its
//! actions; nothing here renders.
//!
//! Requests leave the moment an action runs: [`Desk::place`] and the other
//! actions write to the venue on the UI thread through the session, with no
//! queue or batching in between (ADR 0035, ADR 0050 decision 7). Venue
//! events are applied by [`Desk::pump`] with the instant the session
//! received them, so measured times are the network's and the venue's, not
//! this app's frame rate.

use bunting_trading_state::bunting_client::bnp_wire::{ClientMessage, Listing};
use bunting_trading_state::local_venue::LocalVenue;
use bunting_trading_state::{Instrument, Model, OrderTicket, Session, SessionEvent, TicketError};
use gpui_kit::{AsyncApp, Context, WeakEntity};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Venue events applied before the window is told to redraw.
const EVENTS_PER_PUMP: usize = 4_096;
/// How often queued venue events are applied. Orders never wait on this:
/// they are written when the user acts.
pub const PUMP_INTERVAL: Duration = Duration::from_millis(4);
/// Least time between two redraws while events keep arriving.
const REDRAW_INTERVAL: Duration = Duration::from_millis(16);
/// Redraw at least this often so ages and clocks move.
const IDLE_REDRAW: Duration = Duration::from_millis(250);

/// Where requests go when there is no session: tests record them.
pub trait Outbox {
    /// Writes one request now.
    ///
    /// # Errors
    /// Returns why the request was not written.
    fn send(&self, message: &ClientMessage) -> Result<(), String>;
}

enum Link {
    None,
    Session(Session),
    Outbox(Box<dyn Outbox>),
}

/// The venue this window is attached to.
pub struct Venue {
    pub session: Session,
    /// A local practice venue this app started; dropped after the session.
    pub local: Option<LocalVenue>,
}

pub struct Desk {
    model: Model,
    link: Link,
    /// Kept so a local venue outlives the session that logs out of it.
    local: Option<LocalVenue>,
    label: String,
    started: Instant,
    instrument: Option<u128>,
    listing: Option<Listing>,
    /// A price picked from a book for the ticket, numbered so the ticket
    /// takes each pick once.
    price_hint: (u64, Option<i64>),
    dirty: bool,
    last_redraw: Instant,
}

impl Drop for Desk {
    fn drop(&mut self) {
        // Log out before the local venue (if any) stops.
        self.link = Link::None;
        self.local = None;
    }
}

impl Desk {
    /// A desk attached to a venue session.
    #[must_use]
    pub fn connected(venue: Venue, label: String) -> Self {
        let Venue { session, local } = venue;
        let mut desk = Self::new(Link::Session(session), label);
        desk.local = local;
        desk
    }

    /// A desk writing to `outbox` instead of a venue (tests).
    #[must_use]
    pub fn with_outbox(outbox: Box<dyn Outbox>, label: String) -> Self {
        Self::new(Link::Outbox(outbox), label)
    }

    /// A desk with no venue.
    #[must_use]
    pub fn offline(label: String) -> Self {
        Self::new(Link::None, label)
    }

    fn new(link: Link, label: String) -> Self {
        // Client order IDs must never repeat within a run, across restarts:
        // start from the wall clock in microseconds, as the terminal does.
        let first_id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(1, |elapsed| {
                u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
            });
        Self {
            model: Model::new(first_id),
            link,
            local: None,
            label,
            started: Instant::now(),
            instrument: None,
            listing: None,
            price_hint: (0, None),
            dirty: true,
            last_redraw: Instant::now(),
        }
    }

    /// Applies venue events every [`PUMP_INTERVAL`] while the desk lives.
    pub fn start_pump(cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(PUMP_INTERVAL).await;
                if this.update(cx, Self::pump).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Microseconds since this desk started: the model's clock.
    #[must_use]
    pub fn now_us(&self) -> u64 {
        micros(self.started.elapsed())
    }

    #[must_use]
    pub const fn model(&self) -> &Model {
        &self.model
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Drains queued venue events into the model and sends what they call
    /// for.
    pub fn pump(&mut self, cx: &mut Context<Self>) {
        let mut events = Vec::new();
        if let Link::Session(session) = &self.link {
            while events.len() < EVENTS_PER_PUMP {
                let Some(event) = session.try_event() else {
                    break;
                };
                events.push(event);
            }
        }
        for (event, received) in events {
            let at = micros(received.saturating_duration_since(self.started));
            self.apply(event, at);
        }
        let requests = self.model.tick(self.now_us());
        self.send_all(requests);
        let since = self.last_redraw.elapsed();
        if (self.dirty && since >= REDRAW_INTERVAL) || since >= IDLE_REDRAW {
            self.dirty = false;
            self.last_redraw = Instant::now();
            cx.notify();
        }
    }

    /// Applies one venue event received at `at_us` and sends the requests
    /// it calls for.
    pub fn apply(&mut self, event: SessionEvent, at_us: u64) {
        let requests = self.model.apply(event, at_us);
        self.send_all(requests);
        self.dirty = true;
    }

    // ---- selection -------------------------------------------------------

    #[must_use]
    pub fn instruments(&self) -> Vec<Instrument> {
        self.model.instruments()
    }

    /// The instrument shown: the chosen one, else the first listed.
    #[must_use]
    pub fn instrument(&self) -> Option<Instrument> {
        let instruments = self.model.instruments();
        self.instrument
            .and_then(|id| {
                instruments
                    .iter()
                    .find(|instrument| instrument.instrument_id == id)
                    .cloned()
            })
            .or_else(|| instruments.into_iter().next())
    }

    pub fn select_instrument(&mut self, instrument_id: u128, cx: &mut Context<Self>) {
        if self.instrument != Some(instrument_id) {
            self.instrument = Some(instrument_id);
            self.listing = None;
            cx.notify();
        }
    }

    /// The venue listing chosen for the next order. There is no default:
    /// the participant picks the venue (ADR 0050 decision 7).
    #[must_use]
    pub fn listing(&self) -> Option<Listing> {
        let instrument = self.instrument()?;
        self.listing
            .filter(|listing| listing.instrument_id == instrument.instrument_id)
    }

    pub fn select_listing(&mut self, listing: Listing, cx: &mut Context<Self>) {
        self.instrument = Some(listing.instrument_id);
        self.listing = Some(listing);
        cx.notify();
    }

    /// Picks a venue and a price from its book for the next order.
    pub fn pick_price(&mut self, listing: Listing, price: i64, cx: &mut Context<Self>) {
        self.instrument = Some(listing.instrument_id);
        self.listing = Some(listing);
        self.price_hint = (self.price_hint.0.wrapping_add(1), Some(price));
        cx.notify();
    }

    /// The latest picked price and its number.
    #[must_use]
    pub const fn price_hint(&self) -> (u64, Option<i64>) {
        self.price_hint
    }

    // ---- actions -----------------------------------------------------------

    /// Sends a new order now.
    ///
    /// # Errors
    /// Returns the ticket's first invalid field; nothing is sent.
    pub fn place(
        &mut self,
        ticket: &OrderTicket,
        cx: &mut Context<Self>,
    ) -> Result<(), TicketError> {
        let now = self.now_us();
        let message = self.model.place(ticket, now)?;
        self.send(&message);
        cx.notify();
        Ok(())
    }

    pub fn cancel(&mut self, client_order_id: u64, cx: &mut Context<Self>) {
        if let Some(message) = self.model.cancel(client_order_id) {
            self.send(&message);
            cx.notify();
        }
    }

    pub fn cancel_all(&mut self, cx: &mut Context<Self>) {
        let messages = self.model.cancel_all();
        self.send_all(messages);
        cx.notify();
    }

    /// The venue cancels every live order of the participant, FIX ones
    /// included, and refuses new ones.
    pub fn kill_switch(&mut self, cx: &mut Context<Self>) {
        let now = self.now_us();
        let message = self.model.kill_switch(now);
        self.send(&message);
        cx.notify();
    }

    /// Reloads open orders and the account.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let now = self.now_us();
        let messages = self.model.refresh(now);
        self.send_all(messages);
        cx.notify();
    }

    fn send(&mut self, message: &ClientMessage) {
        let result = match &self.link {
            Link::Session(session) => session.send(message).map_err(|error| error.to_string()),
            Link::Outbox(outbox) => outbox.send(message),
            Link::None => Err("no venue".to_owned()),
        };
        if let Err(error) = result {
            let now = self.now_us();
            self.model.send_failed(message, &error, now);
        }
        self.dirty = true;
    }

    fn send_all(&mut self, messages: Vec<ClientMessage>) {
        for message in messages {
            self.send(&message);
        }
    }
}

fn micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}
