//! The participant's view of one venue session, rebuilt from what the venue
//! sent: listings, every direct feed's order-by-order book, each
//! instrument's consolidated tape, the trades those feeds carried, the
//! participant's orders and fills, the account, the access delay the venue
//! measured and the connection with its client-measured round trips.
//!
//! The model is pure. [`Model::apply`] takes one [`SessionEvent`] and the
//! caller's clock in microseconds and returns the BNP requests that event
//! calls for (subscribing to new listings, resubscribing a feed with a gap,
//! reloading orders and the account after a reconnect). The caller sends
//! them; nothing here performs I/O or reads a clock.

use crate::ticket::{OrderTicket, TicketError};
use bunting_client::bnp_wire::{
    CancelReason, CashBalance, ClientMessage, DelaySource, EntryKind, FeedFlags, Liquidity,
    Listing, ListingInfo, OrderEntry, Position, RejectReason, ResumeStatus, ServerMessage, Side,
    Welcome, msg_type,
};
use bunting_client::{FeedBook, TapeBook};
use std::collections::{BTreeMap, VecDeque};

/// Subscriptions a BNP session may hold, direct feeds and tapes together
/// (docs/specs/bnp-v1.md).
pub const MAX_FEEDS: usize = 32;
/// Trades kept in each blotter (direct feeds, tape).
pub const MAX_PRINTS: usize = 512;
/// Orders kept; finished orders are dropped first.
pub const MAX_ORDERS: usize = 1_024;
/// Fills kept.
pub const MAX_FILLS: usize = 1_024;
/// Notices kept.
pub const MAX_NOTICES: usize = 256;
/// Round-trip samples averaged.
pub const ROUND_TRIP_WINDOW: usize = 32;
/// Least time between two account reloads, in microseconds.
const ACCOUNT_REFRESH_US: u64 = 250_000;
/// An account request unanswered this long is given up, in microseconds.
const ACCOUNT_TIMEOUT_US: u64 = 5_000_000;

/// What the session runner reports.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionEvent {
    /// A connection attempt started (1 for the first).
    Connecting { attempt: u32 },
    /// The venue accepted the session.
    Connected(Welcome),
    /// The session ended; the runner will try again.
    Disconnected { reason: String },
    /// A message from the venue. Probes and heartbeats never appear here.
    Message(ServerMessage),
    /// One `Ping` round trip, measured by the client, in microseconds.
    RoundTrip { micros: u64 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Connection {
    Idle,
    Connecting { attempt: u32 },
    Connected { welcome: Welcome, since_us: u64 },
    Disconnected { reason: String, since_us: u64 },
}

/// Client-measured `Ping` round trips: the real network and both stacks,
/// with no virtual distance (session messages are never delayed).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoundTrips {
    window: VecDeque<u64>,
    pub last_us: Option<u64>,
    pub min_us: Option<u64>,
    pub max_us: Option<u64>,
}

impl RoundTrips {
    fn record(&mut self, micros: u64) {
        if self.window.len() == ROUND_TRIP_WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(micros);
        self.last_us = Some(micros);
        self.min_us = Some(self.min_us.map_or(micros, |min| min.min(micros)));
        self.max_us = Some(self.max_us.map_or(micros, |max| max.max(micros)));
    }

    /// Mean of the latest samples.
    #[must_use]
    pub fn mean_us(&self) -> Option<u64> {
        let count = u64::try_from(self.window.len()).ok().filter(|&n| n > 0)?;
        Some(self.window.iter().sum::<u64>() / count)
    }

    #[must_use]
    pub fn samples(&self) -> usize {
        self.window.len()
    }
}

/// Time from sending an order to the venue's answer, per venue: the real
/// network both ways plus the virtual distance both ways plus the queue.
/// Measured on this client; never sent anywhere.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct VenueRoundTrip {
    pub last_us: u64,
    pub min_us: u64,
    pub samples: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeedState {
    /// Subscribed; the snapshot has not arrived.
    Requested,
    Live,
    /// The venue refused the subscription.
    Refused(String),
}

/// One direct venue feed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Feed {
    pub request_id: u32,
    pub book: FeedBook,
    /// Order by order (every displayed order and its queue place), as
    /// requested and then as the venue's snapshot showed; false when the
    /// venue sent price levels or refused order data.
    pub order_by_order: bool,
    pub state: FeedState,
    /// Updates applied since the snapshot.
    pub updates: u64,
    /// Gaps detected (each one resubscribed).
    pub gaps: u64,
    pub last_message_us: Option<u64>,
}

/// One instrument's consolidated tape: every venue's best bid and offer
/// and trades as the processor at the hub publishes them. It crosses the
/// distance from the hub, so near a venue it lags that venue's direct feed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tape {
    pub request_id: u32,
    pub book: TapeBook,
    pub state: FeedState,
    /// Updates applied since the snapshot.
    pub updates: u64,
    /// Gaps detected (each one resubscribed).
    pub gaps: u64,
    pub last_message_us: Option<u64>,
}

/// One trade, with the time this client received it. From a direct feed
/// `listing` is that venue's; from the tape it names the venue that traded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TradePrint {
    pub listing: Listing,
    pub price: i64,
    pub quantity: i64,
    pub received_us: u64,
    /// Broker identifiers, on order-by-order feeds of venues that publish
    /// them.
    pub buyer: Option<u128>,
    pub seller: Option<u128>,
}

/// The venue's measurement of this connection (ADR 0035): published to the
/// participant, never used to order commands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessDelay {
    pub source: DelaySource,
    /// Smallest round trip the venue has seen.
    pub min_rtt_us: Option<u64>,
    /// Half of it: the venue's estimate of one way.
    pub one_way_us: Option<u64>,
    pub probe_samples: u64,
    pub received_us: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OrderStatus {
    /// Sent; the venue has not answered.
    PendingNew,
    /// Accepted; not yet resting or done.
    New,
    Resting,
    /// Cancel sent; the venue has not answered.
    PendingCancel,
    Filled,
    Canceled(CancelReason),
    Rejected(String),
    /// Absent from an open-orders snapshot taken after its last report.
    Closed,
}

impl OrderStatus {
    /// Whether the order may still trade.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(
            self,
            Self::PendingNew | Self::New | Self::Resting | Self::PendingCancel
        )
    }

    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::PendingNew => "sent".to_owned(),
            Self::New => "accepted".to_owned(),
            Self::Resting => "resting".to_owned(),
            Self::PendingCancel => "cancel sent".to_owned(),
            Self::Filled => "filled".to_owned(),
            Self::Canceled(reason) => format!("canceled ({})", words(&format!("{reason:?}"))),
            Self::Rejected(reason) => format!("rejected: {reason}"),
            Self::Closed => "closed".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrderRow {
    /// 0 for an order not entered over BNP.
    pub client_order_id: u64,
    pub order_id: Option<u128>,
    pub listing: Listing,
    pub side: Side,
    pub quantity: i64,
    /// `None` for a market order.
    pub price: Option<i64>,
    pub remaining: i64,
    pub filled: i64,
    pub status: OrderStatus,
    /// When this client sent it, if it did in this process.
    pub sent_us: Option<u64>,
    /// Send to the venue's first answer.
    pub answer_us: Option<u64>,
    /// Highest committed sequence applied to this row.
    pub sequence: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FillRow {
    pub sequence: u64,
    pub logical_time_ns: u64,
    pub client_order_id: u64,
    pub order_id: u128,
    pub listing: Listing,
    pub side: Side,
    pub price: i64,
    pub quantity: i64,
    pub fee: i128,
    pub liquidity: Liquidity,
    pub received_us: u64,
}

/// The venue's last account answer.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AccountView {
    pub run_sequence: u64,
    pub cash: Vec<CashBalance>,
    pub positions: Vec<Position>,
    pub received_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum NoticeLevel {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notice {
    pub at_us: u64,
    pub level: NoticeLevel,
    pub text: String,
}

/// One instrument and the venues that list it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Instrument {
    pub instrument_id: u128,
    pub symbol: String,
    /// Listings in venue order.
    pub listings: Vec<ListingInfo>,
}

#[derive(Clone, Debug)]
pub struct Model {
    connection: Connection,
    round_trips: RoundTrips,
    venue_round_trips: BTreeMap<u128, VenueRoundTrip>,
    listings: Vec<ListingInfo>,
    feeds: BTreeMap<Listing, Feed>,
    feed_requests: BTreeMap<u32, Listing>,
    tapes: BTreeMap<u128, Tape>,
    tape_requests: BTreeMap<u32, u128>,
    prints: VecDeque<TradePrint>,
    tape_prints: VecDeque<TradePrint>,
    access_delay: Option<AccessDelay>,
    orders: VecDeque<OrderRow>,
    fills: VecDeque<FillRow>,
    account: Option<AccountView>,
    account_dirty: bool,
    account_request: Option<(u32, u64)>,
    last_account_request_us: Option<u64>,
    kill_switch: bool,
    notices: VecDeque<Notice>,
    last_sequence: u64,
    next_request_id: u32,
    next_client_order_id: u64,
}

impl Model {
    /// `first_client_order_id` must not repeat an ID used earlier in the run
    /// (the venue refuses reuse, as on an OUCH session); microseconds since
    /// the Unix epoch at start-up is a sound choice.
    #[must_use]
    pub fn new(first_client_order_id: u64) -> Self {
        Self {
            connection: Connection::Idle,
            round_trips: RoundTrips::default(),
            venue_round_trips: BTreeMap::new(),
            listings: Vec::new(),
            feeds: BTreeMap::new(),
            feed_requests: BTreeMap::new(),
            tapes: BTreeMap::new(),
            tape_requests: BTreeMap::new(),
            prints: VecDeque::new(),
            tape_prints: VecDeque::new(),
            access_delay: None,
            orders: VecDeque::new(),
            fills: VecDeque::new(),
            account: None,
            account_dirty: false,
            account_request: None,
            last_account_request_us: None,
            kill_switch: false,
            notices: VecDeque::new(),
            last_sequence: 0,
            next_request_id: 1,
            next_client_order_id: first_client_order_id.max(1),
        }
    }

    // ---- reading -------------------------------------------------------

    #[must_use]
    pub const fn connection(&self) -> &Connection {
        &self.connection
    }

    #[must_use]
    pub const fn is_connected(&self) -> bool {
        matches!(self.connection, Connection::Connected { .. })
    }

    #[must_use]
    pub const fn round_trips(&self) -> &RoundTrips {
        &self.round_trips
    }

    /// Order round trips per venue, measured on this client.
    #[must_use]
    pub const fn venue_round_trips(&self) -> &BTreeMap<u128, VenueRoundTrip> {
        &self.venue_round_trips
    }

    #[must_use]
    pub fn listings(&self) -> &[ListingInfo] {
        &self.listings
    }

    /// Instruments in ID order, each with the venues listing it.
    #[must_use]
    pub fn instruments(&self) -> Vec<Instrument> {
        let mut instruments: BTreeMap<u128, Instrument> = BTreeMap::new();
        for info in &self.listings {
            instruments
                .entry(info.listing.instrument_id)
                .or_insert_with(|| Instrument {
                    instrument_id: info.listing.instrument_id,
                    symbol: info.symbol.clone(),
                    listings: Vec::new(),
                })
                .listings
                .push(info.clone());
        }
        let mut instruments: Vec<Instrument> = instruments.into_values().collect();
        for instrument in &mut instruments {
            instrument
                .listings
                .sort_by_key(|info| info.listing.venue_id);
        }
        instruments
    }

    #[must_use]
    pub fn symbol(&self, listing: Listing) -> Option<&str> {
        self.listings
            .iter()
            .find(|info| info.listing == listing)
            .map(|info| info.symbol.as_str())
    }

    #[must_use]
    pub fn feed(&self, listing: Listing) -> Option<&Feed> {
        self.feeds.get(&listing)
    }

    /// Trades from every direct feed, newest last, in the order this client
    /// received them. Each venue's trades arrive after that venue's own
    /// distance, so this is not a consolidated tape.
    #[must_use]
    pub const fn prints(&self) -> &VecDeque<TradePrint> {
        &self.prints
    }

    /// One instrument's consolidated tape, if subscribed.
    #[must_use]
    pub fn tape(&self, instrument_id: u128) -> Option<&Tape> {
        self.tapes.get(&instrument_id)
    }

    /// Trades from every instrument's consolidated tape, newest last, in the
    /// order this client received them.
    #[must_use]
    pub const fn tape_prints(&self) -> &VecDeque<TradePrint> {
        &self.tape_prints
    }

    /// The access delay the venue last published for this connection.
    #[must_use]
    pub const fn access_delay(&self) -> Option<&AccessDelay> {
        self.access_delay.as_ref()
    }

    /// Orders, oldest first.
    #[must_use]
    pub const fn orders(&self) -> &VecDeque<OrderRow> {
        &self.orders
    }

    pub fn live_orders(&self) -> impl Iterator<Item = &OrderRow> {
        self.orders.iter().filter(|order| order.status.is_live())
    }

    /// Fills, oldest first.
    #[must_use]
    pub const fn fills(&self) -> &VecDeque<FillRow> {
        &self.fills
    }

    #[must_use]
    pub const fn account(&self) -> Option<&AccountView> {
        self.account.as_ref()
    }

    /// Lots bought minus sold in fills after the last account answer: what
    /// the position should have moved by since then.
    #[must_use]
    pub fn unreported_position(&self, instrument_id: u128) -> i64 {
        let after = self
            .account
            .as_ref()
            .map_or(0, |account| account.run_sequence);
        self.fills
            .iter()
            .filter(|fill| fill.listing.instrument_id == instrument_id && fill.sequence > after)
            .map(|fill| match fill.side {
                Side::Buy => fill.quantity,
                Side::Sell => -fill.quantity,
            })
            .sum()
    }

    #[must_use]
    pub const fn kill_switch_active(&self) -> bool {
        self.kill_switch
    }

    /// Notices, oldest first.
    #[must_use]
    pub const fn notices(&self) -> &VecDeque<Notice> {
        &self.notices
    }

    /// Highest committed sequence of any private report applied.
    #[must_use]
    pub const fn last_sequence(&self) -> u64 {
        self.last_sequence
    }

    // ---- trading -------------------------------------------------------

    /// Records a new order from `ticket` and returns the message to send.
    ///
    /// # Errors
    /// Returns the ticket's first invalid field; nothing is recorded.
    pub fn place(
        &mut self,
        ticket: &OrderTicket,
        now_us: u64,
    ) -> Result<ClientMessage, TicketError> {
        let client_order_id = self.next_client_order_id;
        let order = ticket.to_order(client_order_id)?;
        self.next_client_order_id = self.next_client_order_id.saturating_add(1);
        self.push_order(OrderRow {
            client_order_id,
            order_id: None,
            listing: order.listing,
            side: order.side,
            quantity: order.quantity,
            price: match order.order_type {
                bunting_client::bnp_wire::OrderType::Limit { price } => Some(price),
                bunting_client::bnp_wire::OrderType::Market => None,
            },
            remaining: order.quantity,
            filled: 0,
            status: OrderStatus::PendingNew,
            sent_us: Some(now_us),
            answer_us: None,
            sequence: 0,
        });
        Ok(ClientMessage::NewOrder(order))
    }

    /// The cancel for one live order entered over BNP, if it can be
    /// canceled from here.
    pub fn cancel(&mut self, client_order_id: u64) -> Option<ClientMessage> {
        if client_order_id == 0 {
            return None;
        }
        let order = self
            .orders
            .iter_mut()
            .find(|order| order.client_order_id == client_order_id)?;
        if !matches!(order.status, OrderStatus::New | OrderStatus::Resting) {
            return None;
        }
        order.status = OrderStatus::PendingCancel;
        Some(ClientMessage::CancelOrder { client_order_id })
    }

    /// Cancels for every live order entered over BNP.
    pub fn cancel_all(&mut self) -> Vec<ClientMessage> {
        let ids: Vec<u64> = self
            .orders
            .iter()
            .filter(|order| matches!(order.status, OrderStatus::New | OrderStatus::Resting))
            .map(|order| order.client_order_id)
            .collect();
        ids.into_iter().filter_map(|id| self.cancel(id)).collect()
    }

    /// The kill switch: the venue cancels every live order of the
    /// participant, including FIX ones, and blocks new orders.
    pub fn kill_switch(&mut self, now_us: u64) -> ClientMessage {
        self.notice(now_us, NoticeLevel::Warning, "kill switch sent".to_owned());
        ClientMessage::KillSwitch {
            request_id: u64::from(self.request_id()),
        }
    }

    /// Lifts this participant's kill switch; the venue confirms with
    /// `KillSwitchReset`.
    pub fn reset_kill_switch(&mut self, now_us: u64) -> ClientMessage {
        self.notice(
            now_us,
            NoticeLevel::Info,
            "kill switch reset sent".to_owned(),
        );
        ClientMessage::KillSwitchReset {
            request_id: u64::from(self.request_id()),
        }
    }

    /// Reloads open orders and the account.
    pub fn refresh(&mut self, now_us: u64) -> Vec<ClientMessage> {
        let mut messages = vec![ClientMessage::OpenOrdersRequest {
            request_id: self.request_id(),
        }];
        messages.extend(self.request_account(now_us));
        messages
    }

    /// Records that `message` could not be written.
    pub fn send_failed(&mut self, message: &ClientMessage, error: &str, now_us: u64) {
        match message {
            ClientMessage::NewOrder(order) => {
                if let Some(row) = self.order_mut(order.client_order_id) {
                    row.status = OrderStatus::Rejected(format!("not sent: {error}"));
                }
            }
            ClientMessage::CancelOrder { client_order_id } => {
                if let Some(row) = self.order_mut(*client_order_id)
                    && row.status == OrderStatus::PendingCancel
                {
                    row.status = OrderStatus::Resting;
                }
            }
            _ => {}
        }
        self.notice(
            now_us,
            NoticeLevel::Error,
            format!("not sent ({}): {error}", request_name(message.msg_type())),
        );
    }

    /// Periodic work: reloads the account after fills, at most every
    /// 250 ms.
    pub fn tick(&mut self, now_us: u64) -> Vec<ClientMessage> {
        if let Some((_, sent)) = self.account_request
            && now_us.saturating_sub(sent) > ACCOUNT_TIMEOUT_US
        {
            self.account_request = None;
        }
        let due = self
            .last_account_request_us
            .is_none_or(|last| now_us.saturating_sub(last) >= ACCOUNT_REFRESH_US);
        if self.account_dirty && self.account_request.is_none() && due && self.is_connected() {
            return self.request_account(now_us);
        }
        Vec::new()
    }

    // ---- applying session events ----------------------------------------

    /// Applies one event and returns the requests it calls for.
    pub fn apply(&mut self, event: SessionEvent, now_us: u64) -> Vec<ClientMessage> {
        match event {
            SessionEvent::Connecting { attempt } => {
                self.connection = Connection::Connecting { attempt };
                Vec::new()
            }
            SessionEvent::Connected(welcome) => self.connected(welcome, now_us),
            SessionEvent::Disconnected { reason } => {
                self.notice(
                    now_us,
                    NoticeLevel::Error,
                    format!("disconnected: {reason}"),
                );
                self.connection = Connection::Disconnected {
                    reason,
                    since_us: now_us,
                };
                self.account_request = None;
                Vec::new()
            }
            SessionEvent::RoundTrip { micros } => {
                self.round_trips.record(micros);
                Vec::new()
            }
            SessionEvent::Message(message) => self.message(message, now_us),
        }
    }

    fn connected(&mut self, welcome: Welcome, now_us: u64) -> Vec<ClientMessage> {
        self.notice(
            now_us,
            NoticeLevel::Info,
            format!(
                "connected as participant {} to run {} (private stream {})",
                welcome.participant_id,
                welcome.run_id,
                match welcome.resume {
                    ResumeStatus::Live => "live",
                    ResumeStatus::Replaying => "replaying missed reports",
                    ResumeStatus::Gap => "had a gap; reloading orders and account",
                }
            ),
        );
        self.connection = Connection::Connected {
            welcome,
            since_us: now_us,
        };
        // Feeds belong to a connection: subscribe again for fresh snapshots.
        self.feeds.clear();
        self.feed_requests.clear();
        self.tapes.clear();
        self.tape_requests.clear();
        self.access_delay = None;
        self.account_request = None;
        let mut messages = vec![ClientMessage::ListingsRequest {
            request_id: self.request_id(),
        }];
        messages.extend(self.refresh(now_us));
        messages
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one arm per BNP server message, each a few lines"
    )]
    fn message(&mut self, message: ServerMessage, now_us: u64) -> Vec<ClientMessage> {
        if let Some(stamp) = message.stamp() {
            self.last_sequence = self.last_sequence.max(stamp.sequence);
        }
        match message {
            ServerMessage::Listings { listings, .. } => {
                return self.apply_listings(listings, now_us);
            }
            ServerMessage::MarketSnapshot { request_id, .. }
            | ServerMessage::MarketUpdate { request_id, .. }
            | ServerMessage::OrderSnapshot { request_id, .. }
            | ServerMessage::OrderUpdate { request_id, .. } => {
                return self.feed_message(request_id, &message, now_us);
            }
            ServerMessage::TapeSnapshot { request_id, .. }
            | ServerMessage::TapeUpdate { request_id, .. } => {
                return self.tape_message(request_id, &message, now_us);
            }
            ServerMessage::AccessDelay {
                source,
                min_rtt_us,
                one_way_us,
                probe_samples,
            } => {
                self.access_delay = Some(AccessDelay {
                    source,
                    min_rtt_us,
                    one_way_us,
                    probe_samples,
                    received_us: now_us,
                });
            }
            ServerMessage::Reject {
                request_type,
                reference,
                reason,
            } => return self.reject(request_type, reference, &reason, now_us),
            ServerMessage::OrderAccepted {
                stamp,
                client_order_id,
                order_id,
                listing,
                side,
                quantity,
                price,
            } => {
                let row = self.order_entry(client_order_id, order_id, listing, side, quantity);
                row.order_id = Some(order_id);
                row.price = price;
                row.sequence = row.sequence.max(stamp.sequence);
                if row.status == OrderStatus::PendingNew {
                    row.status = OrderStatus::New;
                }
                self.answered(client_order_id, now_us);
            }
            ServerMessage::OrderRejected {
                stamp,
                client_order_id,
                reason,
            } => {
                let text = reject_text(reason);
                if let Some(row) = self.order_mut(client_order_id) {
                    row.status = OrderStatus::Rejected(text.clone());
                    row.remaining = 0;
                    row.sequence = row.sequence.max(stamp.sequence);
                }
                self.answered(client_order_id, now_us);
                self.notice(
                    now_us,
                    NoticeLevel::Warning,
                    format!("order rejected: {text}"),
                );
            }
            ServerMessage::OrderRested {
                stamp,
                client_order_id,
                order_id,
                price,
                remaining,
                reference: _,
            } => {
                if let Some(row) = self.order_by(client_order_id, order_id) {
                    if row.status.is_live() && row.status != OrderStatus::PendingCancel {
                        row.status = OrderStatus::Resting;
                    }
                    row.price = Some(price);
                    row.remaining = remaining;
                    row.sequence = row.sequence.max(stamp.sequence);
                }
            }
            ServerMessage::OrderReduced {
                stamp,
                client_order_id,
                order_id,
                remaining,
                reference: _,
            } => {
                if let Some(row) = self.order_by(client_order_id, order_id) {
                    row.remaining = remaining;
                    row.sequence = row.sequence.max(stamp.sequence);
                }
            }
            ServerMessage::Fill {
                stamp,
                client_order_id,
                order_id,
                listing,
                side,
                price,
                quantity,
                fee,
                liquidity,
            } => {
                if let Some(row) = self.order_by(client_order_id, order_id) {
                    row.filled = row.filled.saturating_add(quantity);
                    row.remaining = row.remaining.saturating_sub(quantity).max(0);
                    row.sequence = row.sequence.max(stamp.sequence);
                }
                if self.fills.len() == MAX_FILLS {
                    self.fills.pop_front();
                }
                self.fills.push_back(FillRow {
                    sequence: stamp.sequence,
                    logical_time_ns: stamp.logical_time_ns,
                    client_order_id,
                    order_id,
                    listing,
                    side,
                    price,
                    quantity,
                    fee,
                    liquidity,
                    received_us: now_us,
                });
                self.account_dirty = true;
            }
            ServerMessage::OrderDone {
                stamp,
                client_order_id,
                order_id,
            } => {
                if let Some(row) = self.order_by(client_order_id, order_id) {
                    row.status = OrderStatus::Filled;
                    row.remaining = 0;
                    row.sequence = row.sequence.max(stamp.sequence);
                }
                self.account_dirty = true;
            }
            ServerMessage::OrderCanceled {
                stamp,
                client_order_id,
                order_id,
                remaining,
                reason,
            } => {
                if let Some(row) = self.order_by(client_order_id, order_id) {
                    row.status = OrderStatus::Canceled(reason);
                    row.remaining = remaining;
                    row.sequence = row.sequence.max(stamp.sequence);
                }
                self.account_dirty = true;
            }
            ServerMessage::CancelRejected {
                client_order_id,
                reason,
                ..
            } => {
                if let Some(row) = self.order_mut(client_order_id)
                    && row.status == OrderStatus::PendingCancel
                {
                    row.status = OrderStatus::Resting;
                }
                self.notice(
                    now_us,
                    NoticeLevel::Warning,
                    format!("cancel rejected: {}", reject_text(reason)),
                );
            }
            ServerMessage::KillSwitchActivated { .. } => {
                self.kill_switch = true;
                self.notice(
                    now_us,
                    NoticeLevel::Warning,
                    "kill switch active: every order canceled, new orders refused".to_owned(),
                );
            }
            ServerMessage::KillSwitchReset { .. } => {
                self.kill_switch = false;
                self.notice(
                    now_us,
                    NoticeLevel::Info,
                    "kill switch reset: new orders accepted again".to_owned(),
                );
            }
            ServerMessage::PositionChanged { .. } | ServerMessage::BalanceChanged { .. } => {
                self.account_dirty = true;
            }
            ServerMessage::OpenOrders {
                run_sequence,
                orders,
                ..
            } => self.open_orders(run_sequence, &orders),
            ServerMessage::Account {
                request_id,
                run_sequence,
                cash,
                positions,
            } => {
                if self
                    .account_request
                    .is_some_and(|(pending, _)| pending == request_id)
                {
                    self.account_request = None;
                }
                // Fills applied after this answer still need a reload.
                self.account_dirty = self
                    .fills
                    .back()
                    .is_some_and(|fill| fill.sequence > run_sequence);
                self.account = Some(AccountView {
                    run_sequence,
                    cash,
                    positions,
                    received_us: now_us,
                });
            }
            ServerMessage::ReplayComplete { through_sequence } => self.notice(
                now_us,
                NoticeLevel::Info,
                format!("missed reports replayed through sequence {through_sequence}"),
            ),
            ServerMessage::Welcome(_)
            | ServerMessage::Heartbeat
            | ServerMessage::Probe { .. }
            | ServerMessage::Pong { .. }
            | ServerMessage::Logout { .. }
            // The run stream (ADR 0042): not shown by this model yet.
            | ServerMessage::OperatorDone { .. }
            | ServerMessage::RunStatus { .. }
            | ServerMessage::ListingHalt { .. }
            | ServerMessage::News { .. } => {}
        }
        Vec::new()
    }

    /// Subscribes every listing's direct feed, order by order, then every
    /// instrument's tape, within the session's subscription limit.
    fn apply_listings(&mut self, listings: Vec<ListingInfo>, now_us: u64) -> Vec<ClientMessage> {
        self.listings = listings;
        self.listings.sort_by_key(|info| info.listing);
        let mut messages = Vec::new();
        let wanted: Vec<Listing> = self.listings.iter().map(|info| info.listing).collect();
        let mut instruments: Vec<u128> = wanted.iter().map(|l| l.instrument_id).collect();
        instruments.dedup();
        let mut full = false;
        for listing in wanted {
            if self.feeds.contains_key(&listing) {
                continue;
            }
            if self.subscriptions() == MAX_FEEDS {
                full = true;
                break;
            }
            messages.push(self.subscribe(listing, true));
        }
        for instrument_id in instruments {
            if self.tapes.contains_key(&instrument_id) {
                continue;
            }
            if self.subscriptions() == MAX_FEEDS {
                full = true;
                break;
            }
            messages.push(self.subscribe_tape(instrument_id));
        }
        if full {
            self.notice(
                now_us,
                NoticeLevel::Warning,
                format!(
                    "the venue allows {MAX_FEEDS} subscriptions; some books or tapes are not shown"
                ),
            );
        }
        messages
    }

    fn subscriptions(&self) -> usize {
        self.feeds.len() + self.tapes.len()
    }

    fn subscribe(&mut self, listing: Listing, order_by_order: bool) -> ClientMessage {
        let request_id = self.request_id();
        self.feeds.insert(
            listing,
            Feed {
                request_id,
                book: FeedBook::new(request_id),
                order_by_order,
                state: FeedState::Requested,
                updates: 0,
                gaps: 0,
                last_message_us: None,
            },
        );
        self.feed_requests.insert(request_id, listing);
        ClientMessage::Subscribe {
            request_id,
            listing,
            flags: if order_by_order {
                FeedFlags::ALL.union(FeedFlags::ORDERS)
            } else {
                FeedFlags::ALL
            },
        }
    }

    /// A tape is a subscription to venue 0 (docs/specs/bnp-v1.md).
    fn subscribe_tape(&mut self, instrument_id: u128) -> ClientMessage {
        let request_id = self.request_id();
        self.tapes.insert(
            instrument_id,
            Tape {
                request_id,
                book: TapeBook::new(request_id),
                state: FeedState::Requested,
                updates: 0,
                gaps: 0,
                last_message_us: None,
            },
        );
        self.tape_requests.insert(request_id, instrument_id);
        ClientMessage::Subscribe {
            request_id,
            listing: Listing {
                venue_id: 0,
                instrument_id,
            },
            flags: FeedFlags::ALL,
        }
    }

    fn feed_message(
        &mut self,
        request_id: u32,
        message: &ServerMessage,
        now_us: u64,
    ) -> Vec<ClientMessage> {
        let Some(&listing) = self.feed_requests.get(&request_id) else {
            return Vec::new();
        };
        let Some(feed) = self.feeds.get_mut(&listing) else {
            return Vec::new();
        };
        match feed.book.apply(message) {
            Ok(_) => {
                feed.last_message_us = Some(now_us);
                match message {
                    // What the venue sent decides the feed's kind.
                    ServerMessage::MarketSnapshot { .. } => {
                        feed.state = FeedState::Live;
                        feed.order_by_order = false;
                    }
                    ServerMessage::OrderSnapshot { complete: true, .. } => {
                        feed.state = FeedState::Live;
                        feed.order_by_order = true;
                    }
                    ServerMessage::MarketUpdate { entries, .. } => {
                        feed.updates = feed.updates.saturating_add(1);
                        for entry in entries.iter().filter(|e| e.kind == EntryKind::Trade) {
                            push_print(
                                &mut self.prints,
                                TradePrint {
                                    listing,
                                    price: entry.price,
                                    quantity: entry.quantity,
                                    received_us: now_us,
                                    buyer: None,
                                    seller: None,
                                },
                            );
                        }
                    }
                    ServerMessage::OrderUpdate { entries, .. } => {
                        feed.updates = feed.updates.saturating_add(1);
                        for entry in entries {
                            if let OrderEntry::Trade {
                                price,
                                quantity,
                                buyer,
                                seller,
                                ..
                            } = *entry
                            {
                                push_print(
                                    &mut self.prints,
                                    TradePrint {
                                        listing,
                                        price,
                                        quantity,
                                        received_us: now_us,
                                        buyer,
                                        seller,
                                    },
                                );
                            }
                        }
                    }
                    _ => {}
                }
                Vec::new()
            }
            Err(error) => {
                // The book is stale: start the feed again from a snapshot.
                let gaps = feed.gaps.saturating_add(1);
                let order_by_order = feed.order_by_order;
                self.feed_requests.remove(&request_id);
                self.feeds.remove(&listing);
                self.notice(
                    now_us,
                    NoticeLevel::Warning,
                    format!(
                        "feed {}:{} {error}; resubscribing",
                        listing.venue_id, listing.instrument_id
                    ),
                );
                let subscribe = self.subscribe(listing, order_by_order);
                if let Some(feed) = self.feeds.get_mut(&listing) {
                    feed.gaps = gaps;
                }
                vec![ClientMessage::Unsubscribe { request_id }, subscribe]
            }
        }
    }

    fn tape_message(
        &mut self,
        request_id: u32,
        message: &ServerMessage,
        now_us: u64,
    ) -> Vec<ClientMessage> {
        let Some(&instrument_id) = self.tape_requests.get(&request_id) else {
            return Vec::new();
        };
        let Some(tape) = self.tapes.get_mut(&instrument_id) else {
            return Vec::new();
        };
        match tape.book.apply(message) {
            Ok(_) => {
                tape.last_message_us = Some(now_us);
                match message {
                    ServerMessage::TapeSnapshot { .. } => tape.state = FeedState::Live,
                    ServerMessage::TapeUpdate { entries, .. } => {
                        tape.updates = tape.updates.saturating_add(1);
                        for entry in entries.iter().filter(|e| e.kind == EntryKind::Trade) {
                            push_print(
                                &mut self.tape_prints,
                                TradePrint {
                                    listing: Listing {
                                        venue_id: entry.venue_id,
                                        instrument_id,
                                    },
                                    price: entry.price,
                                    quantity: entry.quantity,
                                    received_us: now_us,
                                    buyer: None,
                                    seller: None,
                                },
                            );
                        }
                    }
                    _ => {}
                }
                Vec::new()
            }
            Err(error) => {
                let gaps = tape.gaps.saturating_add(1);
                self.tape_requests.remove(&request_id);
                self.tapes.remove(&instrument_id);
                self.notice(
                    now_us,
                    NoticeLevel::Warning,
                    format!("tape for instrument {instrument_id} {error}; resubscribing"),
                );
                let subscribe = self.subscribe_tape(instrument_id);
                if let Some(tape) = self.tapes.get_mut(&instrument_id) {
                    tape.gaps = gaps;
                }
                vec![ClientMessage::Unsubscribe { request_id }, subscribe]
            }
        }
    }

    fn reject(
        &mut self,
        request_type: u8,
        reference: u64,
        reason: &str,
        now_us: u64,
    ) -> Vec<ClientMessage> {
        let mut messages = Vec::new();
        match request_type {
            msg_type::NEW_ORDER => {
                if let Some(row) = self.order_mut(reference) {
                    row.status = OrderStatus::Rejected(reason.to_owned());
                    row.remaining = 0;
                }
                self.answered(reference, now_us);
            }
            msg_type::CANCEL_ORDER => {
                if let Some(row) = self.order_mut(reference)
                    && row.status == OrderStatus::PendingCancel
                {
                    row.status = OrderStatus::Resting;
                }
            }
            msg_type::SUBSCRIBE => {
                let request_id = u32::try_from(reference).ok();
                if let Some(&listing) = request_id.and_then(|id| self.feed_requests.get(&id))
                    && let Some(feed) = self.feeds.get_mut(&listing)
                {
                    if feed.order_by_order {
                        // A venue may not offer order-by-order data: fall
                        // back to its price levels.
                        self.feed_requests.remove(&feed.request_id);
                        self.feeds.remove(&listing);
                        messages.push(self.subscribe(listing, false));
                    } else {
                        feed.state = FeedState::Refused(reason.to_owned());
                    }
                }
                if let Some(&instrument_id) = request_id.and_then(|id| self.tape_requests.get(&id))
                    && let Some(tape) = self.tapes.get_mut(&instrument_id)
                {
                    tape.state = FeedState::Refused(reason.to_owned());
                }
            }
            msg_type::ACCOUNT_REQUEST => {
                if self
                    .account_request
                    .is_some_and(|(id, _)| u64::from(id) == reference)
                {
                    self.account_request = None;
                }
            }
            _ => {}
        }
        self.notice(
            now_us,
            NoticeLevel::Warning,
            format!("{} refused: {reason}", request_name(request_type)),
        );
        messages
    }

    fn open_orders(&mut self, run_sequence: u64, orders: &[bunting_client::bnp_wire::OpenOrder]) {
        for open in orders {
            let row = self.order_entry(
                open.client_order_id,
                open.order_id,
                open.listing,
                open.side,
                open.original_quantity,
            );
            row.order_id = Some(open.order_id);
            row.price = Some(open.price);
            row.remaining = open.remaining_quantity;
            row.filled = open.original_quantity - open.remaining_quantity;
            if matches!(row.status, OrderStatus::PendingNew | OrderStatus::New)
                || !row.status.is_live()
            {
                row.status = OrderStatus::Resting;
            }
            row.sequence = row.sequence.max(run_sequence);
        }
        // Orders this client thought live but the venue no longer lists,
        // with nothing newer than the snapshot, have ended.
        for row in &mut self.orders {
            let listed = orders
                .iter()
                .any(|open| Some(open.order_id) == row.order_id);
            if !listed
                && row.order_id.is_some()
                && row.status.is_live()
                && row.sequence <= run_sequence
            {
                row.status = OrderStatus::Closed;
            }
        }
    }

    // ---- helpers -------------------------------------------------------

    fn request_id(&mut self) -> u32 {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).unwrap_or(1);
        id
    }

    fn request_account(&mut self, now_us: u64) -> Vec<ClientMessage> {
        if self.account_request.is_some() {
            return Vec::new();
        }
        let request_id = self.request_id();
        self.account_request = Some((request_id, now_us));
        self.last_account_request_us = Some(now_us);
        self.account_dirty = false;
        vec![ClientMessage::AccountRequest { request_id }]
    }

    fn answered(&mut self, client_order_id: u64, now_us: u64) {
        let Some(row) = self.order_mut(client_order_id) else {
            return;
        };
        let (Some(sent), None) = (row.sent_us, row.answer_us) else {
            return;
        };
        let elapsed = now_us.saturating_sub(sent);
        row.answer_us = Some(elapsed);
        let venue = row.listing.venue_id;
        let entry = self.venue_round_trips.entry(venue).or_default();
        entry.min_us = if entry.samples == 0 {
            elapsed
        } else {
            entry.min_us.min(elapsed)
        };
        entry.last_us = elapsed;
        entry.samples = entry.samples.saturating_add(1);
    }

    fn order_mut(&mut self, client_order_id: u64) -> Option<&mut OrderRow> {
        if client_order_id == 0 {
            return None;
        }
        self.orders
            .iter_mut()
            .rev()
            .find(|order| order.client_order_id == client_order_id)
    }

    /// A row by client order ID, or by venue order ID for orders not
    /// entered over BNP.
    fn order_by(&mut self, client_order_id: u64, order_id: u128) -> Option<&mut OrderRow> {
        self.orders.iter_mut().rev().find(|order| {
            (client_order_id != 0 && order.client_order_id == client_order_id)
                || order.order_id == Some(order_id)
        })
    }

    /// The row for an order, created when this client did not send it in
    /// this process (a replayed report or an open-orders answer).
    fn order_entry(
        &mut self,
        client_order_id: u64,
        order_id: u128,
        listing: Listing,
        side: Side,
        quantity: i64,
    ) -> &mut OrderRow {
        let position = self.orders.iter().rposition(|order| {
            (client_order_id != 0 && order.client_order_id == client_order_id)
                || order.order_id == Some(order_id)
        });
        let index = if let Some(index) = position {
            index
        } else {
            self.push_order(OrderRow {
                client_order_id,
                order_id: Some(order_id),
                listing,
                side,
                quantity,
                price: None,
                remaining: quantity,
                filled: 0,
                status: OrderStatus::New,
                sent_us: None,
                answer_us: None,
                sequence: 0,
            });
            self.orders.len() - 1
        };
        &mut self.orders[index]
    }

    fn push_order(&mut self, row: OrderRow) {
        if self.orders.len() >= MAX_ORDERS {
            let oldest_finished = self
                .orders
                .iter()
                .position(|order| !order.status.is_live())
                .unwrap_or(0);
            self.orders.remove(oldest_finished);
        }
        self.orders.push_back(row);
    }

    fn notice(&mut self, at_us: u64, level: NoticeLevel, text: String) {
        if self.notices.len() == MAX_NOTICES {
            self.notices.pop_front();
        }
        self.notices.push_back(Notice { at_us, level, text });
    }
}

fn push_print(prints: &mut VecDeque<TradePrint>, print: TradePrint) {
    if prints.len() == MAX_PRINTS {
        prints.pop_front();
    }
    prints.push_back(print);
}

/// `InsufficientCash` → `insufficient cash`.
fn words(name: &str) -> String {
    let mut text = String::with_capacity(name.len() + 4);
    for (index, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                text.push(' ');
            }
            text.extend(character.to_lowercase());
        } else {
            text.push(character);
        }
    }
    text
}

fn reject_text(reason: RejectReason) -> String {
    words(&format!("{reason:?}"))
}

fn request_name(request_type: u8) -> &'static str {
    match request_type {
        msg_type::NEW_ORDER => "order",
        msg_type::CANCEL_ORDER => "cancel",
        msg_type::KILL_SWITCH => "kill switch",
        msg_type::KILL_SWITCH_RESET => "kill switch reset",
        msg_type::SUBSCRIBE => "subscription",
        msg_type::UNSUBSCRIBE => "unsubscribe",
        msg_type::SNAPSHOT_REQUEST => "snapshot request",
        msg_type::LISTINGS_REQUEST => "listings request",
        msg_type::OPEN_ORDERS_REQUEST => "open orders request",
        msg_type::ACCOUNT_REQUEST => "account request",
        _ => "request",
    }
}

#[cfg(test)]
mod tests;
