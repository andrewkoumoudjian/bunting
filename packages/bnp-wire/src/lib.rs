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

/// BNP 1.0, written `1` in `Hello` and `Welcome` (the legacy value).
pub const VERSION_1_0: u16 = 1;
/// BNP 1.1 (ADR 0054 §1): `major × 256 + minor`. It adds message types
/// only; every 1.0 body is unchanged.
pub const VERSION_1_1: u16 = 257;
/// The highest version this crate speaks: a client offers it in `Hello`,
/// and a server answers with [`negotiate`]'s choice in `Welcome`.
pub const PROTOCOL_VERSION: u16 = VERSION_1_1;

/// The version a server speaks to a client that offered `offered`: the
/// lower of the two 1.x minors, or `None` for another major.
#[must_use]
pub const fn negotiate(offered: u16) -> Option<u16> {
    if offered == VERSION_1_0 {
        return Some(VERSION_1_0);
    }
    if offered >> 8 != 1 {
        return None;
    }
    let [minor, _] = offered.to_le_bytes();
    if minor == 0 {
        Some(VERSION_1_0)
    } else {
        Some(VERSION_1_1)
    }
}

/// Whether a session negotiated at `version` carries 1.1 messages.
#[must_use]
pub const fn speaks_1_1(version: u16) -> bool {
    version >= VERSION_1_1
}
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
    /// BNP 1.1 (ADR 0056): the only message of a session without a
    /// client certificate.
    pub const ENROLL: u8 = 0x06;
    pub const NEW_ORDER: u8 = 0x10;
    pub const CANCEL_ORDER: u8 = 0x11;
    pub const KILL_SWITCH: u8 = 0x12;
    pub const KILL_SWITCH_RESET: u8 = 0x13;
    pub const TENDER_DECISION: u8 = 0x18;
    pub const OTC_PROPOSE: u8 = 0x19;
    pub const OTC_COUNTER: u8 = 0x1a;
    pub const OTC_DECISION: u8 = 0x1b;
    pub const SUBSCRIBE: u8 = 0x20;
    pub const UNSUBSCRIBE: u8 = 0x21;
    pub const SNAPSHOT_REQUEST: u8 = 0x22;
    pub const LISTINGS_REQUEST: u8 = 0x30;
    pub const OPEN_ORDERS_REQUEST: u8 = 0x31;
    pub const ACCOUNT_REQUEST: u8 = 0x32;
    pub const DEALS_REQUEST: u8 = 0x33;
    /// BNP 1.1.
    pub const REFERENCE_DATA_REQUEST: u8 = 0x34;
    pub const OPERATOR_COMMAND: u8 = 0x38;

    pub const WELCOME: u8 = 0x41;
    /// BNP 1.1 (ADR 0056).
    pub const ENROLLED: u8 = 0x4e;
    pub const SERVER_HEARTBEAT: u8 = 0x42;
    pub const PROBE: u8 = 0x43;
    pub const PONG: u8 = 0x44;
    pub const REJECT: u8 = 0x45;
    pub const REPLAY_COMPLETE: u8 = 0x46;
    pub const SERVER_LOGOUT: u8 = 0x47;
    pub const ACCESS_DELAY: u8 = 0x48;
    pub const OPERATOR_DONE: u8 = 0x49;
    pub const DEAL_DONE: u8 = 0x4a;
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
    pub const KILL_SWITCH_RESET_DONE: u8 = 0x5b;
    pub const RUN_STATUS: u8 = 0x5c;
    pub const LISTING_HALT: u8 = 0x5d;
    pub const NEWS: u8 = 0x5e;
    pub const MARKET_SNAPSHOT: u8 = 0x60;
    pub const MARKET_UPDATE: u8 = 0x61;
    pub const ORDER_SNAPSHOT: u8 = 0x62;
    pub const ORDER_UPDATE: u8 = 0x63;
    pub const TAPE_SNAPSHOT: u8 = 0x64;
    pub const TAPE_UPDATE: u8 = 0x65;
    pub const LISTINGS: u8 = 0x70;
    pub const OPEN_ORDERS: u8 = 0x71;
    pub const ACCOUNT: u8 = 0x72;
    pub const DEALS: u8 = 0x73;
    pub const TENDER: u8 = 0x74;
    pub const NEGOTIATION: u8 = 0x75;
    /// BNP 1.1.
    pub const REFERENCE_DATA: u8 = 0x76;
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
    /// What happened to one displayed order on an order-by-order feed.
    OrderAction {
        /// A new displayed order, behind every order already at its price.
        Add = 1,
        /// Its displayed quantity changed; it keeps its reference and place.
        Change = 2,
        /// It left the book (executed, canceled, or lost priority).
        Delete = 3,
    }
);

wire_enum!(
    /// Where a measured access delay came from.
    DelaySource {
        None = 0,
        /// The operating system's TCP round-trip minimum.
        Kernel = 1,
        /// Application probes.
        Probe = 2,
        /// Both; the smaller is used.
        Both = 3,
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
        MarketClosed = 25,
        CallPhaseOrderType = 26,
        /// BNP 1.1: a limit price off the listing's tick grid. A 1.0
        /// session receives `PriceOutOfBounds` instead.
        PriceNotOnTick = 27,
    }
);

wire_enum!(
    /// An instrument's kind in `ReferenceData` (BNP 1.1).
    InstrumentKind {
        Equity = 1,
        Currency = 2,
        Bond = 3,
        Option = 4,
        Future = 5,
        Commodity = 6,
        Synthetic = 7,
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

/// Who a news item is for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NewsAudience {
    Public,
    Participant(u128),
    Team(u128),
    /// Every actor with this role name (`participant`, `instructor`, …).
    Role(String),
}

/// An instructor or administrator action (ADR 0042).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperatorAction {
    Start,
    Pause,
    Resume,
    /// Advances the run clock by whole steps.
    Advance {
        steps: u32,
    },
    Terminate {
        reason: String,
    },
    /// Halts (or resumes) every listing of an instrument.
    Halt {
        instrument_id: u128,
        halted: bool,
        reason: String,
    },
    /// Cancels live orders, optionally only one participant's or one
    /// instrument's.
    MassCancel {
        participant_id: Option<u128>,
        instrument_id: Option<u128>,
    },
    PublishNews {
        news_id: u128,
        audience: NewsAudience,
        headline: String,
        body: String,
    },
    /// Freezes the iteration's scores.
    Score,
    Fine {
        participant_id: u128,
        currency_id: u128,
        /// Minor currency units.
        amount: i128,
        reason: String,
    },
    /// Offers one participant a tender: it may take `quantity` on `side` at
    /// `price` until `expires_at_ns` of run time (ADR 0043).
    OpenTender {
        tender_id: u128,
        participant_id: u128,
        instrument_id: u128,
        side: Side,
        quantity: i64,
        price: i64,
        expires_at_ns: u64,
    },
}

wire_enum!(
    /// A tender's state (ADR 0043).
    TenderStatus {
        Open = 1,
        Accepted = 2,
        Declined = 3,
        Expired = 4,
    }
);

wire_enum!(
    /// An OTC negotiation's state (ADR 0043).
    NegotiationStatus {
        Proposed = 1,
        Countered = 2,
        Accepted = 3,
        Rejected = 4,
        Broken = 5,
        Expired = 6,
    }
);

wire_enum!(
    /// An answer to an OTC negotiation. Only the party it awaits accepts
    /// or rejects; either party may break an open negotiation.
    OtcDecision {
        Accept = 1,
        Reject = 2,
        Break = 3,
    }
);

/// A tender offered to one participant: it buys or sells `quantity` at
/// `price` on `side` if it accepts before `expires_at_ns`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TenderInfo {
    pub tender_id: u128,
    pub participant_id: u128,
    pub instrument_id: u128,
    pub side: Side,
    pub quantity: i64,
    pub price: i64,
    pub expires_at_ns: u64,
    pub status: TenderStatus,
}

/// A bilateral OTC negotiation between two participants. `side` is the
/// proposer's; `awaiting` is the party whose answer it waits for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NegotiationInfo {
    pub negotiation_id: u128,
    pub proposer_id: u128,
    pub counterparty_id: u128,
    pub instrument_id: u128,
    pub side: Side,
    pub quantity: i64,
    pub price: i64,
    pub expires_at_ns: u64,
    pub status: NegotiationStatus,
    pub awaiting: u128,
}

impl OperatorAction {
    /// This action's wire byte.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Start => 1,
            Self::Pause => 2,
            Self::Resume => 3,
            Self::Advance { .. } => 4,
            Self::Terminate { .. } => 5,
            Self::Halt { .. } => 6,
            Self::MassCancel { .. } => 7,
            Self::PublishNews { .. } => 8,
            Self::Score => 9,
            Self::Fine { .. } => 10,
            Self::OpenTender { .. } => 11,
        }
    }
}

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
    /// Bids, offers and trades.
    pub const ALL: Self = Self(7);
    /// Order by order (L3): every displayed order by its public reference,
    /// instead of price levels. Venue feeds only.
    pub const ORDERS: Self = Self(8);
    const KNOWN: u8 = 15;

    /// # Errors
    /// Returns [`WireError::InvalidValue`] for unknown bits.
    pub const fn from_wire(value: u8) -> Result<Self, WireError> {
        if value & !Self::KNOWN != 0 {
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
    /// Activates the participant's own kill switch: cancels its live
    /// orders and refuses its new ones until `KillSwitchReset`. Other
    /// participants are unaffected.
    KillSwitch {
        request_id: u64,
    },
    /// Lifts the participant's kill switch.
    KillSwitchReset {
        request_id: u64,
    },
    /// Starts a direct venue feed: a snapshot, then every later change.
    /// With [`FeedFlags::ORDERS`] it is order by order. A listing with
    /// `venue_id` 0 names the instrument's consolidated tape instead, which
    /// always carries quotes and trades.
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
    /// An instructor's or administrator's command; refused for other
    /// certificates. `request_id` is unique for the sender for the run, as
    /// a client order ID: a reused one is a duplicate and commits nothing.
    OperatorCommand {
        request_id: u64,
        action: OperatorAction,
    },
    /// Accepts or declines a tender offered to this participant (ADR 0043).
    TenderDecision {
        request_id: u64,
        tender_id: u128,
        accept: bool,
    },
    /// Proposes an OTC trade to another participant: this participant
    /// takes `side`. `negotiation_id` is the sender's own, unique for the
    /// run; the venue names the negotiation in its own namespace.
    OtcPropose {
        request_id: u64,
        negotiation_id: u64,
        counterparty_id: u128,
        instrument_id: u128,
        side: Side,
        quantity: i64,
        price: i64,
        expires_at_ns: u64,
    },
    /// Counters the negotiation awaiting this participant.
    OtcCounter {
        request_id: u64,
        negotiation_id: u128,
        quantity: i64,
        price: i64,
    },
    OtcDecision {
        request_id: u64,
        negotiation_id: u128,
        decision: OtcDecision,
    },
    /// This participant's tenders and negotiations (every one, for an
    /// operator).
    DealsRequest {
        request_id: u32,
    },
    /// BNP 1.1: one page of reference data, instruments after
    /// `after_instrument` in ID order, at most `limit` (1..=64; 0 is 64).
    ReferenceDataRequest {
        request_id: u32,
        after_instrument: Option<u128>,
        limit: u16,
    },
    /// BNP 1.1 (ADR 0056): an invite's secret and a PEM certificate
    /// signing request, sent on a session without a client certificate.
    Enroll {
        secret: String,
        csr_pem: String,
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
            Self::KillSwitchReset { .. } => msg_type::KILL_SWITCH_RESET,
            Self::Subscribe { .. } => msg_type::SUBSCRIBE,
            Self::Unsubscribe { .. } => msg_type::UNSUBSCRIBE,
            Self::SnapshotRequest { .. } => msg_type::SNAPSHOT_REQUEST,
            Self::ListingsRequest { .. } => msg_type::LISTINGS_REQUEST,
            Self::OpenOrdersRequest { .. } => msg_type::OPEN_ORDERS_REQUEST,
            Self::AccountRequest { .. } => msg_type::ACCOUNT_REQUEST,
            Self::OperatorCommand { .. } => msg_type::OPERATOR_COMMAND,
            Self::TenderDecision { .. } => msg_type::TENDER_DECISION,
            Self::OtcPropose { .. } => msg_type::OTC_PROPOSE,
            Self::OtcCounter { .. } => msg_type::OTC_COUNTER,
            Self::OtcDecision { .. } => msg_type::OTC_DECISION,
            Self::DealsRequest { .. } => msg_type::DEALS_REQUEST,
            Self::ReferenceDataRequest { .. } => msg_type::REFERENCE_DATA_REQUEST,
            Self::Enroll { .. } => msg_type::ENROLL,
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

/// One displayed order in an order-by-order snapshot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BookOrder {
    pub side: Side,
    /// Anonymous public reference: the order's time priority in its book,
    /// unrelated to any order ID, and new whenever it loses priority.
    pub reference: u64,
    pub price: i64,
    /// Displayed quantity.
    pub quantity: i64,
    /// The owner's broker identifier on venues that publish them.
    pub broker: Option<u128>,
    /// 1-based place among the side's displayed orders in matching priority.
    pub queue_position: u32,
}

/// One order-by-order change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrderEntry {
    /// A trade against the resting order `resting_reference`; `buyer` and
    /// `seller` are broker identifiers on venues that publish them.
    Trade {
        price: i64,
        quantity: i64,
        resting_reference: Option<u64>,
        buyer: Option<u128>,
        seller: Option<u128>,
    },
    /// A displayed order added, changed or deleted; `quantity` is the
    /// displayed quantity after the change (0 when deleted).
    Order {
        action: OrderAction,
        side: Side,
        reference: u64,
        price: i64,
        quantity: i64,
        broker: Option<u128>,
    },
}

/// One venue's best bid or offer on the consolidated tape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TapeQuote {
    pub venue_id: u128,
    pub side: Side,
    pub price: i64,
    pub quantity: i64,
}

/// One consolidated change, naming its venue. For `Bid` and `Ask` it is the
/// venue's new best (quantity 0: the venue has none on that side).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TapeEntry {
    pub venue_id: u128,
    pub kind: EntryKind,
    pub price: i64,
    pub quantity: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListingInfo {
    pub listing: Listing,
    pub symbol: String,
}

/// Instruments per `ReferenceData` page at most.
pub const MAX_REFERENCE_INSTRUMENTS: u16 = 64;

/// A currency in `ReferenceData` (BNP 1.1). Code and name are empty when
/// the scenario declares none.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurrencyInfo {
    pub currency_id: u128,
    pub code: String,
    pub name: String,
    pub minor_decimals: u8,
}

/// An instrument's reference data (BNP 1.1, ADR 0054 §2). Kind
/// parameters are present only for their kind.
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one field per wire flag, in ADR 0054 order"
)]
pub struct InstrumentInfo {
    pub instrument_id: u128,
    pub symbol: String,
    pub kind: InstrumentKind,
    pub underlying: Option<u128>,
    pub strike: Option<i64>,
    pub expiry_ns: Option<u64>,
    pub is_call: bool,
    pub physical_delivery: bool,
    pub coupon_bps: u32,
    /// `(instrument_id, weight)` of a synthetic.
    pub components: Vec<(u128, i64)>,
    pub multiplier: i64,
    pub currency_id: u128,
    /// Decimal places for showing a price; display only.
    pub quoted_decimals: u8,
    pub shortable: bool,
    pub tradeable: bool,
    pub window_start_ns: Option<u64>,
    pub window_stop_ns: Option<u64>,
    pub display_unit: String,
    pub description: String,
    pub opening_mark: Option<i64>,
    pub max_order_quantity: Option<i64>,
}

/// A listing's reference data (BNP 1.1).
#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "one field per wire flag, in ADR 0054 order"
)]
pub struct ListingReference {
    pub listing: Listing,
    pub symbol: String,
    pub min_price: i64,
    pub max_price: i64,
    /// Smallest price step, in ticks.
    pub tick_size: i64,
    pub taker_fee_per_lot: i128,
    pub maker_fee_per_lot: i128,
    pub broker_ids: bool,
    pub primary: bool,
    pub halted: bool,
    pub in_window: bool,
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
    /// The venue's current measurement of this connection's access delay.
    /// Published only; it never orders commands (ADR 0035). A session
    /// message, never delayed.
    AccessDelay {
        source: DelaySource,
        /// Smallest round trip seen, if any.
        min_rtt_us: Option<u64>,
        /// Half of `min_rtt_us`.
        one_way_us: Option<u64>,
        probe_samples: u64,
    },
    /// An operator command committed at `run_sequence`.
    OperatorDone {
        request_id: u64,
        run_sequence: u64,
    },
    /// A tender decision or OTC request committed (ADR 0043).
    DealDone {
        request_id: u64,
        run_sequence: u64,
    },
    /// A tender offered to this participant changed (every tender, for an
    /// operator).
    Tender {
        stamp: Stamp,
        tender: TenderInfo,
    },
    /// A negotiation this participant is party to changed (every one, for
    /// an operator).
    Negotiation {
        stamp: Stamp,
        negotiation: NegotiationInfo,
    },
    /// The run's lifecycle changed (`active`, `paused`, `terminated`, …).
    RunStatus {
        stamp: Stamp,
        status: String,
    },
    /// Every listing of an instrument was halted or resumed.
    ListingHalt {
        stamp: Stamp,
        instrument_id: u128,
        halted: bool,
    },
    /// A news item whose audience includes this session's actor.
    News {
        stamp: Stamp,
        news_id: u128,
        headline: String,
        body: String,
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
    /// `reference` is the order's public reference on the venue's
    /// order-by-order feed (`BookOrder::reference`), absent when it
    /// displays nothing.
    OrderRested {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        price: i64,
        remaining: i64,
        reference: Option<u64>,
    },
    /// After a fill, `reference` is the order's public reference if it is
    /// still resting; a different value than before means it lost time
    /// priority (an iceberg slice refreshed at the back of its level).
    OrderReduced {
        stamp: Stamp,
        client_order_id: u64,
        order_id: u128,
        remaining: i64,
        reference: Option<u64>,
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
    /// This participant's kill switch is active.
    KillSwitchActivated {
        stamp: Stamp,
    },
    /// This participant's kill switch was reset.
    KillSwitchReset {
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
    /// An order-by-order feed's starting book: every displayed order, bids
    /// then asks, each side in matching priority. A book too large for one
    /// frame arrives in consecutive parts; the last has `complete` set, and
    /// updates follow only after it.
    OrderSnapshot {
        request_id: u32,
        listing: Listing,
        next_report: u64,
        complete: bool,
        orders: Vec<BookOrder>,
    },
    /// One commit's order-by-order changes: trades first, then orders.
    /// Entries are numbered from `first_report`.
    OrderUpdate {
        request_id: u32,
        listing: Listing,
        first_report: u64,
        entries: Vec<OrderEntry>,
    },
    /// The consolidated tape's view of an instrument: each venue's best bid
    /// and offer as the processor at the hub sees them. `last_report` is the
    /// last tape report included; updates continue with the next.
    TapeSnapshot {
        request_id: u32,
        instrument_id: u128,
        last_report: u64,
        quotes: Vec<TapeQuote>,
    },
    /// Consolidated changes in the tape's per-instrument report sequence,
    /// the same for every subscriber, numbered from `first_report`.
    TapeUpdate {
        request_id: u32,
        instrument_id: u128,
        first_report: u64,
        entries: Vec<TapeEntry>,
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
    Deals {
        request_id: u32,
        run_sequence: u64,
        tenders: Vec<TenderInfo>,
        negotiations: Vec<NegotiationInfo>,
    },
    /// BNP 1.1: one page of reference data. `currencies` only on the first
    /// page; `listings` are the listings of this page's instruments;
    /// `next_after` is present iff more instruments remain.
    ReferenceData {
        request_id: u32,
        run_sequence: u64,
        next_after: Option<u128>,
        currencies: Vec<CurrencyInfo>,
        instruments: Vec<InstrumentInfo>,
        listings: Vec<ListingReference>,
    },
    /// BNP 1.1 (ADR 0056): the certificate issued for an `Enroll`, and
    /// the client CA that issued it.
    Enrolled {
        participant_id: u128,
        certificate_pem: String,
        ca_pem: String,
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
            Self::AccessDelay { .. } => msg_type::ACCESS_DELAY,
            Self::OperatorDone { .. } => msg_type::OPERATOR_DONE,
            Self::DealDone { .. } => msg_type::DEAL_DONE,
            Self::Tender { .. } => msg_type::TENDER,
            Self::Negotiation { .. } => msg_type::NEGOTIATION,
            Self::RunStatus { .. } => msg_type::RUN_STATUS,
            Self::ListingHalt { .. } => msg_type::LISTING_HALT,
            Self::News { .. } => msg_type::NEWS,
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
            Self::KillSwitchReset { .. } => msg_type::KILL_SWITCH_RESET_DONE,
            Self::MarketSnapshot { .. } => msg_type::MARKET_SNAPSHOT,
            Self::MarketUpdate { .. } => msg_type::MARKET_UPDATE,
            Self::OrderSnapshot { .. } => msg_type::ORDER_SNAPSHOT,
            Self::OrderUpdate { .. } => msg_type::ORDER_UPDATE,
            Self::TapeSnapshot { .. } => msg_type::TAPE_SNAPSHOT,
            Self::TapeUpdate { .. } => msg_type::TAPE_UPDATE,
            Self::Listings { .. } => msg_type::LISTINGS,
            Self::OpenOrders { .. } => msg_type::OPEN_ORDERS,
            Self::Account { .. } => msg_type::ACCOUNT,
            Self::Deals { .. } => msg_type::DEALS,
            Self::ReferenceData { .. } => msg_type::REFERENCE_DATA,
            Self::Enrolled { .. } => msg_type::ENROLLED,
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
            | Self::KillSwitchActivated { stamp }
            | Self::KillSwitchReset { stamp }
            | Self::RunStatus { stamp, .. }
            | Self::ListingHalt { stamp, .. }
            | Self::News { stamp, .. }
            | Self::Tender { stamp, .. }
            | Self::Negotiation { stamp, .. } => Some(*stamp),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;

/// An enrolment invite (ADR 0056): where the venue listens, the SHA-256 of
/// its client CA certificate (DER) that the client pins, and the one-time
/// secret. Written `bnpi1:<host:port>:<ca-sha256-hex>:<secret-hex>`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InviteToken {
    pub address: String,
    pub ca_sha256: [u8; 32],
    pub secret: String,
}

impl InviteToken {
    const PREFIX: &'static str = "bnpi1:";

    /// Parses a pasted token (surrounding whitespace ignored).
    ///
    /// # Errors
    /// Returns what is malformed.
    pub fn parse(text: &str) -> Result<Self, String> {
        let body = text
            .trim()
            .strip_prefix(Self::PREFIX)
            .ok_or("an invite starts with bnpi1:")?;
        // The address may hold colons (host:port); the pin and the
        // secret are the last two fields.
        let (rest, secret) = body.rsplit_once(':').ok_or("the invite has no secret")?;
        let (address, pin) = rest.rsplit_once(':').ok_or("the invite has no CA pin")?;
        if address.is_empty() || !address.contains(':') {
            return Err("the invite's address must be host:port".to_owned());
        }
        if secret.len() != 32 || !secret.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("the invite's secret must be 32 hex digits".to_owned());
        }
        let mut ca_sha256 = [0_u8; 32];
        if pin.len() != 64 {
            return Err("the invite's CA pin must be 64 hex digits".to_owned());
        }
        for (index, byte) in ca_sha256.iter_mut().enumerate() {
            *byte = u8::from_str_radix(pin.get(index * 2..index * 2 + 2).unwrap_or(""), 16)
                .map_err(|_| "the invite's CA pin must be hex".to_owned())?;
        }
        Ok(Self {
            address: address.to_owned(),
            ca_sha256,
            secret: secret.to_ascii_lowercase(),
        })
    }
}

impl std::fmt::Display for InviteToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}:", Self::PREFIX, self.address)?;
        for byte in self.ca_sha256 {
            write!(f, "{byte:02x}")?;
        }
        write!(f, ":{}", self.secret)
    }
}
