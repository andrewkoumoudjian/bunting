//! Drives one `bunting_client::Client` for an app: connects, reconnects
//! with the private-stream cursor, pings for round-trip time and hands
//! every venue message to the app as a [`SessionEvent`].
//!
//! Requests are written on the caller's thread the moment it calls
//! [`Session::send`]: nothing waits for a poll interval (ADR 0035, a good
//! client never adds delay).

use crate::model::SessionEvent;
use bunting_client::bnp_wire::{ClientMessage, ServerMessage};
use bunting_client::{Client, ClientConfig, ClientError, ClientSender};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Events queued for the app before the runner stops reading the client
/// (and, behind it, TCP): ordinary backpressure.
const EVENT_CAPACITY: usize = 65_536;
/// Interval between `Ping`s.
const PING_INTERVAL: Duration = Duration::from_secs(1);
/// Pings awaiting their `Pong`.
const MAX_PINGS: usize = 16;
/// How long the runner waits on the client before checking for shutdown
/// and pings. It only bounds reaction to shutdown: messages are handed on
/// the moment they arrive.
const RECEIVE_WAIT: Duration = Duration::from_millis(20);
const MIN_BACKOFF: Duration = Duration::from_millis(250);
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Why a request was not written.
#[derive(Debug)]
pub enum SendError {
    NotConnected,
    Client(ClientError),
}

impl std::fmt::Display for SendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotConnected => formatter.write_str("not connected"),
            Self::Client(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SendError {}

struct Shared {
    client: Mutex<Option<ClientSender>>,
    stop: AtomicBool,
}

/// A venue session that stays connected until dropped.
pub struct Session {
    shared: Arc<Shared>,
    events: Receiver<(SessionEvent, Instant)>,
    runner: Option<JoinHandle<()>>,
}

impl Session {
    /// Starts connecting in the background. `config.resume_after` is the
    /// cursor of an earlier session, if any.
    ///
    /// # Errors
    /// Returns an error when the runner thread cannot be started.
    pub fn start(config: ClientConfig) -> std::io::Result<Self> {
        let shared = Arc::new(Shared {
            client: Mutex::new(None),
            stop: AtomicBool::new(false),
        });
        let (sender, events) = sync_channel(EVENT_CAPACITY);
        let runner = Runner {
            config,
            shared: shared.clone(),
            events: sender,
            pings: VecDeque::new(),
            next_nonce: 1,
        };
        let runner = std::thread::Builder::new()
            .name("bunting-session".to_owned())
            .spawn(move || runner.run())?;
        Ok(Self {
            shared,
            events,
            runner: Some(runner),
        })
    }

    /// Writes one request to the venue now.
    ///
    /// # Errors
    /// Returns [`SendError::NotConnected`] between connections.
    pub fn send(&self, message: &ClientMessage) -> Result<(), SendError> {
        let client = self
            .shared
            .client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or(SendError::NotConnected)?;
        client.send(message).map_err(SendError::Client)
    }

    /// The next event, if one is waiting, with the instant the runner
    /// received it from the client. Pass that instant (not the time the app
    /// got round to it) to [`crate::Model::apply`], so round trips measure
    /// the network and the venue rather than the app's own loop.
    #[must_use]
    pub fn try_event(&self) -> Option<(SessionEvent, Instant)> {
        self.events.try_recv().ok()
    }

    /// The next event and when it was received, waiting at most `timeout`.
    #[must_use]
    pub fn event_timeout(&self, timeout: Duration) -> Option<(SessionEvent, Instant)> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        let client = self
            .shared
            .client
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(client) = client {
            let _ = client.send(&ClientMessage::Logout {
                reason: "client closed".to_owned(),
            });
        }
        // Unblock a runner waiting to hand over an event.
        while self.events.try_recv().is_ok() {}
        if let Some(runner) = self.runner.take() {
            let _ = runner.join();
        }
    }
}

struct Runner {
    config: ClientConfig,
    shared: Arc<Shared>,
    events: SyncSender<(SessionEvent, Instant)>,
    pings: VecDeque<(u64, Instant)>,
    next_nonce: u64,
}

impl Runner {
    fn stopping(&self) -> bool {
        self.shared.stop.load(Ordering::Acquire)
    }

    /// Hands an event to the app; `false` once the app has gone.
    fn emit(&self, event: SessionEvent) -> bool {
        let mut event = (event, Instant::now());
        loop {
            if self.stopping() {
                return false;
            }
            match self.events.try_send(event) {
                Ok(()) => return true,
                Err(std::sync::mpsc::TrySendError::Full(back)) => {
                    event = back;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => return false,
            }
        }
    }

    fn run(mut self) {
        let mut attempt = 0_u32;
        let mut backoff = MIN_BACKOFF;
        while !self.stopping() {
            attempt = attempt.saturating_add(1);
            if !self.emit(SessionEvent::Connecting { attempt }) {
                return;
            }
            match Client::connect(&self.config) {
                Ok(client) => {
                    attempt = 0;
                    backoff = MIN_BACKOFF;
                    let welcome = client.welcome().clone();
                    *self
                        .shared
                        .client
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner) = Some(client.sender());
                    if !self.emit(SessionEvent::Connected(welcome)) {
                        return;
                    }
                    let reason = self.serve(&client);
                    self.shared
                        .client
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take();
                    // Resume after the last report handed to the app.
                    self.config.resume_after = Some(client.cursor());
                    client.close();
                    self.pings.clear();
                    if self.stopping() || !self.emit(SessionEvent::Disconnected { reason }) {
                        return;
                    }
                }
                Err(error) => {
                    if !self.emit(SessionEvent::Disconnected {
                        reason: error.to_string(),
                    }) {
                        return;
                    }
                }
            }
            let resume = Instant::now() + backoff;
            while Instant::now() < resume {
                if self.stopping() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }

    /// Forwards messages until the connection ends; returns why.
    fn serve(&mut self, client: &Client) -> String {
        let mut next_ping = Instant::now();
        loop {
            if self.stopping() {
                return "client closed".to_owned();
            }
            if Instant::now() >= next_ping {
                next_ping = Instant::now() + PING_INTERVAL;
                let nonce = self.next_nonce;
                self.next_nonce = self.next_nonce.wrapping_add(1);
                if self.pings.len() == MAX_PINGS {
                    self.pings.pop_front();
                }
                self.pings.push_back((nonce, Instant::now()));
                if let Err(error) = client.send(&ClientMessage::Ping { nonce }) {
                    return error.to_string();
                }
            }
            match client.recv_timeout(RECEIVE_WAIT) {
                Ok(Some(ServerMessage::Pong { nonce, .. })) => {
                    if let Some(index) = self.pings.iter().position(|(sent, _)| *sent == nonce) {
                        let (_, sent_at) =
                            self.pings.remove(index).unwrap_or((nonce, Instant::now()));
                        let micros =
                            u64::try_from(sent_at.elapsed().as_micros()).unwrap_or(u64::MAX);
                        if !self.emit(SessionEvent::RoundTrip { micros }) {
                            return "client closed".to_owned();
                        }
                    }
                }
                Ok(Some(message)) => {
                    if !self.emit(SessionEvent::Message(message)) {
                        return "client closed".to_owned();
                    }
                }
                Ok(None) => {}
                Err(error) => return error.to_string(),
            }
        }
    }
}
