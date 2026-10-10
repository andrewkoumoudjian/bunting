#![forbid(unsafe_code)]
//! Bunting Native Protocol (BNP) version 1: frames and messages (ADR 0040).
//!
//! A frame is a little-endian `u32` length followed by that many bytes: one
//! message-type byte and the message body. Bodies are fixed-width
//! little-endian fields in declaration order; strings are a `u16` byte
//! count and UTF-8 bytes; repeating groups are a `u16` entry count and the
//! entries. Optional values carry a one-byte presence flag. This is the
//! layout style of exchange binary protocols (OUCH, ITCH, SBE): no
//! self-description, no field tags, nothing to negotiate per message.
//!
//! The codec is sans-I/O and bounded. It never allocates more than the
//! configured frame limit, and every malformed input is an error.

mod codec;

pub use codec::{FrameDecoder, decode_client, decode_server, encode_client, encode_server};

/// Version both peers state in `Hello` and `Welcome`.
pub const PROTOCOL_VERSION: u16 = 1;
/// First bytes of every `Hello` body, so a non-BNP peer is refused at once.
pub const MAGIC: [u8; 4] = *b"BNP1";
/// Bytes of the frame length prefix.
pub const LENGTH_BYTES: usize = 4;
/// Largest frame (type byte plus body) either peer may ever send.
pub const MAX_FRAME_BYTES: usize = 65_536;
/// Largest encoded string, in bytes.
pub const MAX_STRING_BYTES: usize = 1_024;
/// Largest repeating group, in entries.
pub const MAX_GROUP_ENTRIES: usize = 4_096;

/// Message-type bytes. Client messages are `0x01..=0x3f`, server messages
/// `0x40..=0x7f`.
pub mod msg_type {
    pub const HELLO: u8 = 0x01;
    pub const CLIENT_HEARTBEAT: u8 = 0x02;
    pub const PROBE_REPLY: u8 = 0x03;
    pub const PING: u8 = 0x04;
    pub const CLIENT_LOGOUT: u8 = 0x05;
    pub const NEW_ORDER: u8 = 0x10;
    pub const CANCEL_ORDER: u8 = 0x11;
    pub const KILL_SWITCH: u8 = 0x12;
    pub const SUBSCRIBE: u8 = 0x20;
    pub const UNSUBSCRIBE: u8 = 0x21;
    pub const SNAPSHOT_REQUEST: u8 = 0x22;
    pub const LISTINGS_REQUEST: u8 = 0x30;
    pub const OPEN_ORDERS_REQUEST: u8 = 0x31;
    pub const ACCOUNT_REQUEST: u8 = 0x32;

    pub const WELCOME: u8 = 0x41;
    pub const SERVER_HEARTBEAT: u8 = 0x42;
    pub const PROBE: u8 = 0x43;
    pub const PONG: u8 = 0x44;
    pub const REJECT: u8 = 0x45;
    pub const REPLAY_COMPLETE: u8 = 0x46;
    pub const SERVER_LOGOUT: u8 = 0x47;
    pub const ORDER_ACCEPTED: u8 = 0x50;
    pub const ORDER_REJECTED: u8 = 0x51;
    pub const ORDER_RESTED: u8 = 0x52;
    pub const FILL: u8 = 0x53;
    pub const ORDER_DONE: u8 = 0x54;
    pub const ORDER_CANCELED: u8 = 0x55;
    pub const CANCEL_REJECTED: u8 = 0x56;
    pub const POSITION_CHANGED: u8 = 0x57;
    pub const BALANCE_CHANGED: u8 = 0x58;
    pub const KILL_SWITCH_ACTIVATED: u8 = 0x59;
    pub const ORDER_REDUCED: u8 = 0x5a;
    pub const MARKET_SNAPSHOT: u8 = 0x60;
    pub const MARKET_UPDATE: u8 = 0x61;
    pub const LISTINGS: u8 = 0x70;
    pub const OPEN_ORDERS: u8 = 0x71;
    pub const ACCOUNT: u8 = 0x72;
}

/// Why a frame could not be encoded or decoded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WireError {
    /// A frame is larger than the limit (`limit`, `actual` bytes).
    FrameTooLarge {
        limit: usize,
        actual: usize,
    },
    /// A frame of zero bytes has no message type.
    EmptyFrame,
    /// The body ended before every field was read.
    Truncated,
    /// Bytes remained after the last field.
    TrailingBytes,
    UnknownMessageType(u8),
    /// A server message where a client message was expected, or the reverse.
    WrongDirection(u8),
    /// A field held a value outside its enumeration (`field`, `value`).
    InvalidValue {
        field: &'static str,
        value: u64,
    },
    StringTooLong(usize),
    InvalidUtf8,
    GroupTooLarge(usize),
    BadMagic,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FrameTooLarge { limit, actual } => {
                write!(formatter, "frame of {actual} bytes exceeds limit {limit}")
            }
            Self::EmptyFrame => formatter.write_str("empty frame"),
            Self::Truncated => formatter.write_str("truncated message"),
            Self::TrailingBytes => formatter.write_str("trailing bytes after message"),
            Self::UnknownMessageType(kind) => write!(formatter, "unknown message type {kind:#04x}"),
            Self::WrongDirection(kind) => {
                write!(
                    formatter,
                    "message type {kind:#04x} is sent by the other peer"
                )
            }
            Self::InvalidValue { field, value } => {
                write!(formatter, "invalid {field} value {value}")
            }
            Self::StringTooLong(bytes) => {
                write!(
                    formatter,
                    "string of {bytes} bytes exceeds {MAX_STRING_BYTES}"
                )
            }
            Self::InvalidUtf8 => formatter.write_str("string is not UTF-8"),
            Self::GroupTooLarge(entries) => {
                write!(
                    formatter,
                    "group of {entries} entries exceeds {MAX_GROUP_ENTRIES}"
                )
            }
            Self::BadMagic => formatter.write_str("not a BNP peer (bad Hello magic)"),
        }
    }
}

impl std::error::Error for WireError {}

/// Declares a one-byte enumeration with its wire values.
macro_rules! wire_enum {
    ($(#[$meta:meta])* $name:ident { $($(#[$vmeta:meta])* $variant:ident = $value:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
        pub enum $name {
            $($(#[$vmeta])* $variant),+
        }

        impl $name {
            /// Every value, in wire order.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            #[must_use]
            pub const fn to_wire(self) -> u8 {
                match self {
                    $(Self::$variant => $value),+
                }
            }

            /// # Errors
            /// Returns [`WireError::InvalidValue`] for a value outside the enumeration.
            pub const fn from_wire(value: u8) -> Result<Self, WireError> {
                match value {
                    $($value => Ok(Self::$variant),)+
                    other => Err(WireError::InvalidValue {
                        field: stringify!($name),
                        value: other as u64,
                    }),
                }
            }
        }
    };
}

wire_enum!(Side { Buy = 1, Sell = 2 });

wire_enum!(
    /// The authenticated actor's role, from its registered certificate.
    Role {
        Participant = 1,
        Team = 2,
        Instructor = 3,
        Administrator = 4,
        BuiltInAgent = 5,
    }
);

wire_enum!(
    /// What the server does with a `Hello`'s resume cursor.
    ResumeStatus {
        /// No cursor was given: the private stream starts now.
        Live = 0,
        /// Every private message after the cursor follows, then `ReplayComplete`.
        Replaying = 1,
        /// The cursor is older than the server retains: rebuild state with
        /// `OpenOrdersRequest` and `AccountRequest`; the stream starts now.
        Gap = 2,
    }
);

wire_enum!(
    /// Which side of a fill rested on the book.
    Liquidity { Maker = 1, Taker = 2 }
);

wire_enum!(
    /// One public market data entry.
    EntryKind {
        Trade = 0,
        Bid = 1,
        Ask = 2,
    }
);

wire_enum!(
    /// Engine reject reasons, as published in `OrderRejected` and
    /// `CancelRejected`.
    RejectReason {
        DuplicateOrderId = 1,
        InvalidOrderId = 2,
        UnknownOrder = 3,
        NotOrderOwner = 4,
        KillSwitchActive = 5,
        RunNotActive = 6,
        ListingHalted = 7,
        ParticipantDisabled = 8,
        InvalidQuantity = 9,
        InvalidInstrument = 10,
        PriceOutOfBounds = 11,
        MaxOrderQuantity = 12,
        MaxOpenOrderQuantity = 13,
        MaxLiveOrders = 14,
        PositionLimit = 15,
        InsufficientCash = 16,
        InsufficientInventory = 17,
        InsufficientLiquidity = 18,
        PostOnlyWouldCross = 19,
        InvalidTimeInForce = 20,
        LogicalTimeRegression = 21,
        SequenceConflict = 22,
        ArithmeticOverflow = 23,
        UnknownListing = 24,
    }
);

wire_enum!(
    /// Why a resting order left the book without trading.
    CancelReason {
        Requested = 1,
        KillSwitch = 2,
        MarketRemainder = 3,
        MassCancel = 4,
        Expired = 5,
        Halt = 6,
    }
);

/// One tradable listing: an instrument on one venue.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Listing {
    pub venue_id: u128,
    pub instrument_id: u128,
}

/// Which public entries a feed carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeedFlags(u8);

impl FeedFlags {
    pub const BIDS: Self = Self(1);
    pub const OFFERS: Self = Self(2);
    pub const TRADES: Self = Self(4);
    pub const ALL: Self = Self(7);

    /// # Errors
    /// Returns [`WireError::InvalidValue`] for unknown bits.
    pub const fn from_wire(value: u8) -> Result<Self, WireError> {
        if value & !Self::ALL.0 != 0 {
            return Err(WireError::InvalidValue {
                field: "FeedFlags",
                value: value as u64,
            });
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn to_wire(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// How long a limit order may rest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimeInForce {
    Gtc,
    Ioc,
    Fok,
    /// Expires at the listing's session close.
    Day,
    /// Expires when the run's logical clock reaches this time.
    Gtd {
        expires_at_ns: u64,
    },
}

/// The price instruction of an order.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderType {
    Limit { price: i64 },
    Market,
}

/// A new order. `client_order_id` must be unique for the participant for
/// the whole run (across connections), as on an OUCH session: a resend with
/// a used ID commits nothing new.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NewOrder {
    pub client_order_id: u64,
    pub listing: Listing,
    pub side: Side,
    pub quantity: i64,
    pub order_type: OrderType,
    pub time_in_force: TimeInForce,
    /// Reject instead of trading on arrival.
    pub post_only: bool,
    /// Hides the participant's broker identifier on venues that publish
    /// them (ADR 0036; FIX `BuntingAnonymous` 10021).
    pub anonymous: bool,
    /// Iceberg display size; `None` displays the whole quantity.
    pub display_quantity: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ClientMessage {
    /// First message on every connection. `resume_after` is the last
    /// private `sequence` the client processed; `None` starts live.
    Hello {
        version: u16,
        resume_after: Option<u64>,
        client_name: String,
    },
    Heartbeat,
    /// Answers a server `Probe` immediately; the server times the round trip.
    ProbeReply {
        probe_id: u64,
    },
    /// Asks the server to echo `nonce` in a `Pong` at once (client-side RTT).
    Ping {
        nonce: u64,
    },
    Logout {
        reason: String,
    },
    NewOrder(NewOrder),
    CancelOrder {
        client_order_id: u64,
    },
    /// Cancels every live order of the participant and blocks new ones.
    KillSwitch {
        request_id: u64,
    },
    /// Starts a direct venue feed: a snapshot, then every later change.
    Subscribe {
        request_id: u32,
        listing: Listing,
        flags: FeedFlags,
    },
    Unsubscribe {
        request_id: u32,
    },
    /// One snapshot of the visible book; `depth` 0 is the full book.
    SnapshotRequest {
        request_id: u32,
        listing: Listing,
        depth: u16,
    },
    ListingsRequest {
        request_id: u32,
    },
    OpenOrdersRequest {
        request_id: u32,
    },
    AccountRequest {
        request_id: u32,
    },
}

impl ClientMessage {
    /// This message's type byte.
    #[must_use]
    pub const fn msg_type(&self) -> u8 {
        match self {
            Self::Hello { .. } => msg_type::HELLO,
            Self::Heartbeat => msg_type::CLIENT_HEARTBEAT,
            Self::ProbeReply { .. } => msg_type::PROBE_REPLY,
            Self::Ping { .. } => msg_type::PING,
            Self::Logout { .. } => msg_type::CLIENT_LOGOUT,
            Self::NewOrder(_) => msg_type::NEW_ORDER,
            Self::CancelOrder { .. } => msg_type::CANCEL_ORDER,
            Self::KillSwitch { .. } => msg_type::KILL_SWITCH,
            Self::Subscribe { .. } => msg_type::SUBSCRIBE,
            Self::Unsubscribe { .. } => msg_type::UNSUBSCRIBE,
            Self::SnapshotRequest { .. } => msg_type::SNAPSHOT_REQUEST,
            Self::ListingsRequest { .. } => msg_type::LISTINGS_REQUEST,
            Self::OpenOrdersRequest { .. } => msg_type::OPEN_ORDERS_REQUEST,
            Self::AccountRequest { .. } => msg_type::ACCOUNT_REQUEST,
        }
    }
}

/// The session's terms, sent once after a valid `Hello`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Welcome {
    pub version: u16,
    pub run_id: u128,
    pub participant_id: u128,
    pub role: Role,
    /// The server sends a heartbeat after this much silence and disconnects
    /// a client silent for three times as long.
    pub heartbeat_ms: u32,
    pub max_frame_bytes: u32,
    pub resume: ResumeStatus,
    /// The run's committed event sequence when the session started.
    pub run_sequence: u64,
}

/// Header of every private message: the committed event that caused it.
/// `sequence` is the resume cursor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub sequence: u64,
    pub logical_time_ns: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Level {
    pub price: i64,
    pub quantity: i64,
}

/// One public change. For `Bid` and `Ask`, `quantity` is the level's new
/// visible quantity (zero removes the level).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub price: i64,
    pub quantity: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListingInfo {
    pub listing: Listing,
    pub symbol: String,
}

/// One live order. `client_order_id` is 0 for an order not entered over BNP.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenOrder {
    pub client_order_id: u64,
    pub order_id: u128,
    pub listing: Listing,
    pub side: Side,
    pub price: i64,
    pub original_quantity: i64,
    pub remaining_quantity: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CashBalance {
    pub currency_id: u128,
    pub balance: i128,
    pub reserved: i128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Position {
    pub instrument_id: u128,
    pub position: i64,
    pub open_buy: i64,
    pub open_sell: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage {
    Welcome(Welcome),
    Heartbeat,
    /// Answer with `ProbeReply` at once: the venue measures access latency.
    Probe {
        probe_id: u64,
    },
    Pong {
        nonce: u64,
        server_time_us: u64,
    },
    /// A request refused before it reached the engine. `request_type` is
    /// the refused message's type byte; `reference` its client order ID or
    /// request ID (0 when it has none).
    Reject {
        request_type: u8,
        reference: u64,
        reason: String,
    },
    /// Every private message up to `through_sequence` has been replayed.
    ReplayComplete {
        through_sequence: u64,
    },
    Logout {
        reason: String,
    },
    OrderAccepted {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        listing: Listing,
        side: Side,
        quantity: i64,
        /// `None` for a market order.
        price: Option<i64>,
    },
    OrderRejected {
        stamp: Stamp,
        client_order_id: u64,
        reason: RejectReason,
    },
    OrderRested {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        price: i64,
        remaining: i64,
    },
    OrderReduced {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        remaining: i64,
    },
    Fill {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        listing: Listing,
        side: Side,
        price: i64,
        quantity: i64,
        /// Fee charged in minor currency units; negative is a rebate.
        fee: i128,
        liquidity: Liquidity,
    },
    /// The order is complete: nothing remains.
    OrderDone {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
    },
    OrderCanceled {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        remaining: i64,
        reason: CancelReason,
    },
    CancelRejected {
        stamp: Stamp,
        client_order_id: u64,
        reason: RejectReason,
    },
    PositionChanged {
        stamp: Stamp,
        instrument_id: u128,
        delta: i64,
    },
    BalanceChanged {
        stamp: Stamp,
        delta: i128,
    },
    KillSwitchActivated {
        stamp: Stamp,
    },
    /// A feed's starting book (or a one-shot snapshot, with `next_report` 0).
    /// Bids and asks are best first.
    MarketSnapshot {
        request_id: u32,
        listing: Listing,
        next_report: u64,
        bids: Vec<Level>,
        asks: Vec<Level>,
    },
    /// One commit's public changes to a feed's listing. Entries are
    /// numbered from `first_report`, continuing the feed without gaps.
    MarketUpdate {
        request_id: u32,
        listing: Listing,
        first_report: u64,
        entries: Vec<Entry>,
    },
    Listings {
        request_id: u32,
        listings: Vec<ListingInfo>,
    },
    OpenOrders {
        request_id: u32,
        run_sequence: u64,
        orders: Vec<OpenOrder>,
    },
    Account {
        request_id: u32,
        run_sequence: u64,
        cash: Vec<CashBalance>,
        positions: Vec<Position>,
    },
}

impl ServerMessage {
    /// This message's type byte.
    #[must_use]
    pub const fn msg_type(&self) -> u8 {
        match self {
            Self::Welcome(_) => msg_type::WELCOME,
            Self::Heartbeat => msg_type::SERVER_HEARTBEAT,
            Self::Probe { .. } => msg_type::PROBE,
            Self::Pong { .. } => msg_type::PONG,
            Self::Reject { .. } => msg_type::REJECT,
            Self::ReplayComplete { .. } => msg_type::REPLAY_COMPLETE,
            Self::Logout { .. } => msg_type::SERVER_LOGOUT,
            Self::OrderAccepted { .. } => msg_type::ORDER_ACCEPTED,
            Self::OrderRejected { .. } => msg_type::ORDER_REJECTED,
            Self::OrderRested { .. } => msg_type::ORDER_RESTED,
            Self::OrderReduced { .. } => msg_type::ORDER_REDUCED,
            Self::Fill { .. } => msg_type::FILL,
            Self::OrderDone { .. } => msg_type::ORDER_DONE,
            Self::OrderCanceled { .. } => msg_type::ORDER_CANCELED,
            Self::CancelRejected { .. } => msg_type::CANCEL_REJECTED,
            Self::PositionChanged { .. } => msg_type::POSITION_CHANGED,
            Self::BalanceChanged { .. } => msg_type::BALANCE_CHANGED,
            Self::KillSwitchActivated { .. } => msg_type::KILL_SWITCH_ACTIVATED,
            Self::MarketSnapshot { .. } => msg_type::MARKET_SNAPSHOT,
            Self::MarketUpdate { .. } => msg_type::MARKET_UPDATE,
            Self::Listings { .. } => msg_type::LISTINGS,
            Self::OpenOrders { .. } => msg_type::OPEN_ORDERS,
            Self::Account { .. } => msg_type::ACCOUNT,
        }
    }

    /// The private stream position this message carries, if it is private.
    #[must_use]
    pub const fn stamp(&self) -> Option<Stamp> {
        match self {
            Self::OrderAccepted { stamp, .. }
            | Self::OrderRejected { stamp, .. }
            | Self::OrderRested { stamp, .. }
            | Self::OrderReduced { stamp, .. }
            | Self::Fill { stamp, .. }
            | Self::OrderDone { stamp, .. }
            | Self::OrderCanceled { stamp, .. }
            | Self::CancelRejected { stamp, .. }
            | Self::PositionChanged { stamp, .. }
            | Self::BalanceChanged { stamp, .. }
            | Self::KillSwitchActivated { stamp } => Some(*stamp),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
