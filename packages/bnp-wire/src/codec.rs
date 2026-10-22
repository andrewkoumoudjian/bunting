//! Encoding and decoding of BNP frames. Field order here is the contract
//! documented in `docs/specs/bnp-v1.md`.

use crate::{
    BookOrder, CancelReason, CashBalance, ClientMessage, CurrencyInfo, DelaySource, Entry,
    EntryKind, FeedFlags, InstrumentInfo, InstrumentKind, LENGTH_BYTES, Level, Liquidity, Listing,
    ListingInfo, ListingReference, MAGIC, MAX_FRAME_BYTES, MAX_GROUP_ENTRIES, MAX_STRING_BYTES,
    NegotiationInfo, NegotiationStatus, NewOrder, NewsAudience, OpenOrder, OperatorAction,
    OrderAction, OrderEntry, OrderType, OtcDecision, Position, RejectReason, ResumeStatus, Role,
    ServerMessage, Side, Stamp, TapeEntry, TapeQuote, TenderInfo, TenderStatus, TimeInForce,
    Welcome, WireError, msg_type as t,
};

/// Kind byte of an order-by-order trade entry; order entries use their
/// [`OrderAction`] value.
const ORDER_ENTRY_TRADE: u8 = 0;

/// Appends one framed client message to `out`.
///
/// # Errors
/// Returns an error when a string or group exceeds its bound or the frame
/// exceeds [`MAX_FRAME_BYTES`]; `out` is left unchanged.
pub fn encode_client(message: &ClientMessage, out: &mut Vec<u8>) -> Result<(), WireError> {
    frame(out, message.msg_type(), |w| write_client(w, message))
}

/// Appends one framed server message to `out`.
///
/// # Errors
/// As [`encode_client`].
pub fn encode_server(message: &ServerMessage, out: &mut Vec<u8>) -> Result<(), WireError> {
    frame(out, message.msg_type(), |w| write_server(w, message))
}

fn frame(
    out: &mut Vec<u8>,
    kind: u8,
    body: impl FnOnce(&mut Writer<'_>) -> Result<(), WireError>,
) -> Result<(), WireError> {
    let start = out.len();
    out.extend_from_slice(&[0; LENGTH_BYTES]);
    out.push(kind);
    let written = body(&mut Writer(out));
    let length = out.len() - start - LENGTH_BYTES;
    let result = written.and_then(|()| {
        if length > MAX_FRAME_BYTES {
            return Err(WireError::FrameTooLarge {
                limit: MAX_FRAME_BYTES,
                actual: length,
            });
        }
        u32::try_from(length).map_err(|_| WireError::FrameTooLarge {
            limit: MAX_FRAME_BYTES,
            actual: length,
        })
    });
    match result {
        Ok(length) => {
            out[start..start + LENGTH_BYTES].copy_from_slice(&length.to_le_bytes());
            Ok(())
        }
        Err(error) => {
            out.truncate(start);
            Err(error)
        }
    }
}

/// Splits a byte stream into frames (type byte plus body), bounded by
/// `max_frame_bytes`.
#[derive(Debug)]
pub struct FrameDecoder {
    max_frame_bytes: usize,
    buffer: Vec<u8>,
}

impl FrameDecoder {
    /// `max_frame_bytes` is capped at [`MAX_FRAME_BYTES`].
    #[must_use]
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            max_frame_bytes: max_frame_bytes.clamp(1, MAX_FRAME_BYTES),
            buffer: Vec::new(),
        }
    }

    /// Adds received bytes and returns every complete frame, in order.
    ///
    /// # Errors
    /// Returns [`WireError::FrameTooLarge`] as soon as a length prefix
    /// exceeds the limit, and [`WireError::EmptyFrame`] for a zero length;
    /// the stream cannot be resynchronized after either.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, WireError> {
        self.buffer.extend_from_slice(bytes);
        let mut frames = Vec::new();
        let mut offset = 0;
        loop {
            let Some(prefix) = self.buffer.get(offset..offset + LENGTH_BYTES) else {
                break;
            };
            let mut length = [0; LENGTH_BYTES];
            length.copy_from_slice(prefix);
            let length = usize::try_from(u32::from_le_bytes(length)).unwrap_or(usize::MAX);
            if length == 0 {
                return Err(WireError::EmptyFrame);
            }
            if length > self.max_frame_bytes {
                return Err(WireError::FrameTooLarge {
                    limit: self.max_frame_bytes,
                    actual: length,
                });
            }
            let start = offset + LENGTH_BYTES;
            let Some(frame) = self.buffer.get(start..start + length) else {
                break;
            };
            frames.push(frame.to_vec());
            offset = start + length;
        }
        self.buffer.drain(..offset);
        Ok(frames)
    }

    /// Bytes received but not yet part of a complete frame.
    #[must_use]
    pub const fn pending_bytes(&self) -> usize {
        self.buffer.len()
    }
}

/// Decodes one frame (as returned by [`FrameDecoder::push`]) sent by a client.
///
/// # Errors
/// Returns an error for a server message type, an unknown type, a
/// malformed body or trailing bytes.
pub fn decode_client(frame: &[u8]) -> Result<ClientMessage, WireError> {
    let (&kind, body) = frame.split_first().ok_or(WireError::EmptyFrame)?;
    let mut r = Reader(body);
    let message = match kind {
        t::HELLO => {
            if r.bytes(MAGIC.len())? != MAGIC {
                return Err(WireError::BadMagic);
            }
            ClientMessage::Hello {
                version: r.u16()?,
                resume_after: r.option(Reader::u64)?,
                client_name: r.string()?,
            }
        }
        t::CLIENT_HEARTBEAT => ClientMessage::Heartbeat,
        t::PROBE_REPLY => ClientMessage::ProbeReply { probe_id: r.u64()? },
        t::PING => ClientMessage::Ping { nonce: r.u64()? },
        t::CLIENT_LOGOUT => ClientMessage::Logout {
            reason: r.string()?,
        },
        t::NEW_ORDER => ClientMessage::NewOrder(read_new_order(&mut r)?),
        t::CANCEL_ORDER => ClientMessage::CancelOrder {
            client_order_id: r.u64()?,
        },
        t::KILL_SWITCH => ClientMessage::KillSwitch {
            request_id: r.u64()?,
        },
        t::KILL_SWITCH_RESET => ClientMessage::KillSwitchReset {
            request_id: r.u64()?,
        },
        t::SUBSCRIBE => ClientMessage::Subscribe {
            request_id: r.u32()?,
            listing: r.listing()?,
            flags: FeedFlags::from_wire(r.u8()?)?,
        },
        t::UNSUBSCRIBE => ClientMessage::Unsubscribe {
            request_id: r.u32()?,
        },
        t::SNAPSHOT_REQUEST => ClientMessage::SnapshotRequest {
            request_id: r.u32()?,
            listing: r.listing()?,
            depth: r.u16()?,
        },
        t::LISTINGS_REQUEST => ClientMessage::ListingsRequest {
            request_id: r.u32()?,
        },
        t::OPEN_ORDERS_REQUEST => ClientMessage::OpenOrdersRequest {
            request_id: r.u32()?,
        },
        t::ACCOUNT_REQUEST => ClientMessage::AccountRequest {
            request_id: r.u32()?,
        },
        t::OPERATOR_COMMAND => ClientMessage::OperatorCommand {
            request_id: r.u64()?,
            action: read_operator_action(&mut r)?,
        },
        t::TENDER_DECISION => ClientMessage::TenderDecision {
            request_id: r.u64()?,
            tender_id: r.u128()?,
            accept: r.bool()?,
        },
        t::OTC_PROPOSE => ClientMessage::OtcPropose {
            request_id: r.u64()?,
            negotiation_id: r.u64()?,
            counterparty_id: r.u128()?,
            instrument_id: r.u128()?,
            side: Side::from_wire(r.u8()?)?,
            quantity: r.i64()?,
            price: r.i64()?,
            expires_at_ns: r.u64()?,
        },
        t::OTC_COUNTER => ClientMessage::OtcCounter {
            request_id: r.u64()?,
            negotiation_id: r.u128()?,
            quantity: r.i64()?,
            price: r.i64()?,
        },
        t::OTC_DECISION => ClientMessage::OtcDecision {
            request_id: r.u64()?,
            negotiation_id: r.u128()?,
            decision: OtcDecision::from_wire(r.u8()?)?,
        },
        t::DEALS_REQUEST => ClientMessage::DealsRequest {
            request_id: r.u32()?,
        },
        t::ENROLL => ClientMessage::Enroll {
            secret: r.string()?,
            csr_pem: r.string()?,
        },
        t::REFERENCE_DATA_REQUEST => ClientMessage::ReferenceDataRequest {
            request_id: r.u32()?,
            after_instrument: r.option(Reader::u128)?,
            limit: r.u16()?,
        },
        0x40..=0x7f => return Err(WireError::WrongDirection(kind)),
        other => return Err(WireError::UnknownMessageType(other)),
    };
    r.finish()?;
    Ok(message)
}

/// Decodes one frame sent by the server.
///
/// # Errors
/// As [`decode_client`].
#[expect(
    clippy::too_many_lines,
    reason = "one arm per message keeps the field order beside its encoder"
)]
pub fn decode_server(frame: &[u8]) -> Result<ServerMessage, WireError> {
    let (&kind, body) = frame.split_first().ok_or(WireError::EmptyFrame)?;
    let mut r = Reader(body);
    let message = match kind {
        t::WELCOME => ServerMessage::Welcome(Welcome {
            version: r.u16()?,
            run_id: r.u128()?,
            participant_id: r.u128()?,
            role: Role::from_wire(r.u8()?)?,
            heartbeat_ms: r.u32()?,
            max_frame_bytes: r.u32()?,
            resume: ResumeStatus::from_wire(r.u8()?)?,
            run_sequence: r.u64()?,
        }),
        t::SERVER_HEARTBEAT => ServerMessage::Heartbeat,
        t::PROBE => ServerMessage::Probe { probe_id: r.u64()? },
        t::PONG => ServerMessage::Pong {
            nonce: r.u64()?,
            server_time_us: r.u64()?,
        },
        t::REJECT => ServerMessage::Reject {
            request_type: r.u8()?,
            reference: r.u64()?,
            reason: r.string()?,
        },
        t::REPLAY_COMPLETE => ServerMessage::ReplayComplete {
            through_sequence: r.u64()?,
        },
        t::SERVER_LOGOUT => ServerMessage::Logout {
            reason: r.string()?,
        },
        t::OPERATOR_DONE => ServerMessage::OperatorDone {
            request_id: r.u64()?,
            run_sequence: r.u64()?,
        },
        t::DEAL_DONE => ServerMessage::DealDone {
            request_id: r.u64()?,
            run_sequence: r.u64()?,
        },
        t::TENDER => ServerMessage::Tender {
            stamp: r.stamp()?,
            tender: read_tender(&mut r)?,
        },
        t::NEGOTIATION => ServerMessage::Negotiation {
            stamp: r.stamp()?,
            negotiation: read_negotiation(&mut r)?,
        },
        t::DEALS => ServerMessage::Deals {
            request_id: r.u32()?,
            run_sequence: r.u64()?,
            tenders: r.group(read_tender)?,
            negotiations: r.group(read_negotiation)?,
        },
        t::ENROLLED => ServerMessage::Enrolled {
            participant_id: r.u128()?,
            certificate_pem: r.string()?,
            ca_pem: r.string()?,
        },
        t::REFERENCE_DATA => ServerMessage::ReferenceData {
            request_id: r.u32()?,
            run_sequence: r.u64()?,
            next_after: r.option(Reader::u128)?,
            currencies: r.group(|r| {
                Ok(CurrencyInfo {
                    currency_id: r.u128()?,
                    code: r.string()?,
                    name: r.string()?,
                    minor_decimals: r.u8()?,
                })
            })?,
            instruments: r.group(read_instrument)?,
            listings: r.group(|r| {
                Ok(ListingReference {
                    listing: r.listing()?,
                    symbol: r.string()?,
                    min_price: r.i64()?,
                    max_price: r.i64()?,
                    tick_size: r.i64()?,
                    taker_fee_per_lot: r.i128()?,
                    maker_fee_per_lot: r.i128()?,
                    broker_ids: r.bool()?,
                    primary: r.bool()?,
                    halted: r.bool()?,
                    in_window: r.bool()?,
                })
            })?,
        },
        t::RUN_STATUS => ServerMessage::RunStatus {
            stamp: r.stamp()?,
            status: r.string()?,
        },
        t::LISTING_HALT => ServerMessage::ListingHalt {
            stamp: r.stamp()?,
            instrument_id: r.u128()?,
            halted: r.bool()?,
        },
        t::NEWS => ServerMessage::News {
            stamp: r.stamp()?,
            news_id: r.u128()?,
            headline: r.string()?,
            body: r.string()?,
        },
        t::ACCESS_DELAY => ServerMessage::AccessDelay {
            source: DelaySource::from_wire(r.u8()?)?,
            min_rtt_us: r.option(Reader::u64)?,
            one_way_us: r.option(Reader::u64)?,
            probe_samples: r.u64()?,
        },
        t::ORDER_SNAPSHOT => ServerMessage::OrderSnapshot {
            request_id: r.u32()?,
            listing: r.listing()?,
            next_report: r.u64()?,
            complete: r.bool()?,
            orders: r.group(|r| {
                Ok(BookOrder {
                    side: Side::from_wire(r.u8()?)?,
                    reference: r.u64()?,
                    price: r.i64()?,
                    quantity: r.i64()?,
                    broker: r.option(Reader::u128)?,
                    queue_position: r.u32()?,
                })
            })?,
        },
        t::ORDER_UPDATE => ServerMessage::OrderUpdate {
            request_id: r.u32()?,
            listing: r.listing()?,
            first_report: r.u64()?,
            entries: r.group(|r| match r.u8()? {
                ORDER_ENTRY_TRADE => Ok(OrderEntry::Trade {
                    price: r.i64()?,
                    quantity: r.i64()?,
                    resting_reference: r.option(Reader::u64)?,
                    buyer: r.option(Reader::u128)?,
                    seller: r.option(Reader::u128)?,
                }),
                action => Ok(OrderEntry::Order {
                    action: OrderAction::from_wire(action)?,
                    side: Side::from_wire(r.u8()?)?,
                    reference: r.u64()?,
                    price: r.i64()?,
                    quantity: r.i64()?,
                    broker: r.option(Reader::u128)?,
                }),
            })?,
        },
        t::TAPE_SNAPSHOT => ServerMessage::TapeSnapshot {
            request_id: r.u32()?,
            instrument_id: r.u128()?,
            last_report: r.u64()?,
            quotes: r.group(|r| {
                Ok(TapeQuote {
                    venue_id: r.u128()?,
                    side: Side::from_wire(r.u8()?)?,
                    price: r.i64()?,
                    quantity: r.i64()?,
                })
            })?,
        },
        t::TAPE_UPDATE => ServerMessage::TapeUpdate {
            request_id: r.u32()?,
            instrument_id: r.u128()?,
            first_report: r.u64()?,
            entries: r.group(|r| {
                Ok(TapeEntry {
                    venue_id: r.u128()?,
                    kind: EntryKind::from_wire(r.u8()?)?,
                    price: r.i64()?,
                    quantity: r.i64()?,
                })
            })?,
        },
        t::ORDER_ACCEPTED => ServerMessage::OrderAccepted {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            order_id: r.u128()?,
            listing: r.listing()?,
            side: Side::from_wire(r.u8()?)?,
            quantity: r.i64()?,
            price: r.option(Reader::i64)?,
        },
        t::ORDER_REJECTED => ServerMessage::OrderRejected {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            reason: RejectReason::from_wire(r.u8()?)?,
        },
        t::ORDER_RESTED => ServerMessage::OrderRested {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            order_id: r.u128()?,
            price: r.i64()?,
            remaining: r.i64()?,
            reference: r.option(Reader::u64)?,
        },
        t::ORDER_REDUCED => ServerMessage::OrderReduced {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            order_id: r.u128()?,
            remaining: r.i64()?,
            reference: r.option(Reader::u64)?,
        },
        t::FILL => ServerMessage::Fill {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            order_id: r.u128()?,
            listing: r.listing()?,
            side: Side::from_wire(r.u8()?)?,
            price: r.i64()?,
            quantity: r.i64()?,
            fee: r.i128()?,
            liquidity: Liquidity::from_wire(r.u8()?)?,
        },
        t::ORDER_DONE => ServerMessage::OrderDone {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            order_id: r.u128()?,
        },
        t::ORDER_CANCELED => ServerMessage::OrderCanceled {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            order_id: r.u128()?,
            remaining: r.i64()?,
            reason: CancelReason::from_wire(r.u8()?)?,
        },
        t::CANCEL_REJECTED => ServerMessage::CancelRejected {
            stamp: r.stamp()?,
            client_order_id: r.u64()?,
            reason: RejectReason::from_wire(r.u8()?)?,
        },
        t::POSITION_CHANGED => ServerMessage::PositionChanged {
            stamp: r.stamp()?,
            instrument_id: r.u128()?,
            delta: r.i64()?,
        },
        t::BALANCE_CHANGED => ServerMessage::BalanceChanged {
            stamp: r.stamp()?,
            delta: r.i128()?,
        },
        t::KILL_SWITCH_ACTIVATED => ServerMessage::KillSwitchActivated { stamp: r.stamp()? },
        t::KILL_SWITCH_RESET_DONE => ServerMessage::KillSwitchReset { stamp: r.stamp()? },
        t::MARKET_SNAPSHOT => ServerMessage::MarketSnapshot {
            request_id: r.u32()?,
            listing: r.listing()?,
            next_report: r.u64()?,
            bids: r.group(Reader::level)?,
            asks: r.group(Reader::level)?,
        },
        t::MARKET_UPDATE => ServerMessage::MarketUpdate {
            request_id: r.u32()?,
            listing: r.listing()?,
            first_report: r.u64()?,
            entries: r.group(|r| {
                Ok(Entry {
                    kind: EntryKind::from_wire(r.u8()?)?,
                    price: r.i64()?,
                    quantity: r.i64()?,
                })
            })?,
        },
        t::LISTINGS => ServerMessage::Listings {
            request_id: r.u32()?,
            listings: r.group(|r| {
                Ok(ListingInfo {
                    listing: r.listing()?,
                    symbol: r.string()?,
                })
            })?,
        },
        t::OPEN_ORDERS => ServerMessage::OpenOrders {
            request_id: r.u32()?,
            run_sequence: r.u64()?,
            orders: r.group(|r| {
                Ok(OpenOrder {
                    client_order_id: r.u64()?,
                    order_id: r.u128()?,
                    listing: r.listing()?,
                    side: Side::from_wire(r.u8()?)?,
                    price: r.i64()?,
                    original_quantity: r.i64()?,
                    remaining_quantity: r.i64()?,
                })
            })?,
        },
        t::ACCOUNT => ServerMessage::Account {
            request_id: r.u32()?,
            run_sequence: r.u64()?,
            cash: r.group(|r| {
                Ok(CashBalance {
                    currency_id: r.u128()?,
                    balance: r.i128()?,
                    reserved: r.i128()?,
                })
            })?,
            positions: r.group(|r| {
                Ok(Position {
                    instrument_id: r.u128()?,
                    position: r.i64()?,
                    open_buy: r.i64()?,
                    open_sell: r.i64()?,
                })
            })?,
        },
        0x01..=0x3f => return Err(WireError::WrongDirection(kind)),
        other => return Err(WireError::UnknownMessageType(other)),
    };
    r.finish()?;
    Ok(message)
}

const AUDIENCE_PUBLIC: u8 = 0;
const AUDIENCE_PARTICIPANT: u8 = 1;
const AUDIENCE_TEAM: u8 = 2;
const AUDIENCE_ROLE: u8 = 3;

fn read_operator_action(r: &mut Reader<'_>) -> Result<OperatorAction, WireError> {
    Ok(match r.u8()? {
        1 => OperatorAction::Start,
        2 => OperatorAction::Pause,
        3 => OperatorAction::Resume,
        4 => OperatorAction::Advance { steps: r.u32()? },
        5 => OperatorAction::Terminate {
            reason: r.string()?,
        },
        6 => OperatorAction::Halt {
            instrument_id: r.u128()?,
            halted: r.bool()?,
            reason: r.string()?,
        },
        7 => OperatorAction::MassCancel {
            participant_id: r.option(Reader::u128)?,
            instrument_id: r.option(Reader::u128)?,
        },
        8 => OperatorAction::PublishNews {
            news_id: r.u128()?,
            audience: match r.u8()? {
                AUDIENCE_PUBLIC => NewsAudience::Public,
                AUDIENCE_PARTICIPANT => NewsAudience::Participant(r.u128()?),
                AUDIENCE_TEAM => NewsAudience::Team(r.u128()?),
                AUDIENCE_ROLE => NewsAudience::Role(r.string()?),
                other => {
                    return Err(WireError::InvalidValue {
                        field: "NewsAudience",
                        value: u64::from(other),
                    });
                }
            },
            headline: r.string()?,
            body: r.string()?,
        },
        9 => OperatorAction::Score,
        10 => OperatorAction::Fine {
            participant_id: r.u128()?,
            currency_id: r.u128()?,
            amount: r.i128()?,
            reason: r.string()?,
        },
        11 => OperatorAction::OpenTender {
            tender_id: r.u128()?,
            participant_id: r.u128()?,
            instrument_id: r.u128()?,
            side: Side::from_wire(r.u8()?)?,
            quantity: r.i64()?,
            price: r.i64()?,
            expires_at_ns: r.u64()?,
        },
        other => {
            return Err(WireError::InvalidValue {
                field: "OperatorAction",
                value: u64::from(other),
            });
        }
    })
}

fn write_operator_action(w: &mut Writer<'_>, action: &OperatorAction) -> Result<(), WireError> {
    w.u8(action.code());
    match action {
        OperatorAction::Start
        | OperatorAction::Pause
        | OperatorAction::Resume
        | OperatorAction::Score => {}
        OperatorAction::Advance { steps } => w.u32(*steps),
        OperatorAction::Terminate { reason } => w.string(reason)?,
        OperatorAction::Halt {
            instrument_id,
            halted,
            reason,
        } => {
            w.u128(*instrument_id);
            w.bool(*halted);
            w.string(reason)?;
        }
        OperatorAction::MassCancel {
            participant_id,
            instrument_id,
        } => {
            w.option(*participant_id, Writer::u128);
            w.option(*instrument_id, Writer::u128);
        }
        OperatorAction::PublishNews {
            news_id,
            audience,
            headline,
            body,
        } => {
            w.u128(*news_id);
            match audience {
                NewsAudience::Public => w.u8(AUDIENCE_PUBLIC),
                NewsAudience::Participant(id) => {
                    w.u8(AUDIENCE_PARTICIPANT);
                    w.u128(*id);
                }
                NewsAudience::Team(id) => {
                    w.u8(AUDIENCE_TEAM);
                    w.u128(*id);
                }
                NewsAudience::Role(role) => {
                    w.u8(AUDIENCE_ROLE);
                    w.string(role)?;
                }
            }
            w.string(headline)?;
            w.string(body)?;
        }
        OperatorAction::Fine {
            participant_id,
            currency_id,
            amount,
            reason,
        } => {
            w.u128(*participant_id);
            w.u128(*currency_id);
            w.i128(*amount);
            w.string(reason)?;
        }
        OperatorAction::OpenTender {
            tender_id,
            participant_id,
            instrument_id,
            side,
            quantity,
            price,
            expires_at_ns,
        } => {
            w.u128(*tender_id);
            w.u128(*participant_id);
            w.u128(*instrument_id);
            w.u8(side.to_wire());
            w.i64(*quantity);
            w.i64(*price);
            w.u64(*expires_at_ns);
        }
    }
    Ok(())
}

fn read_tender(r: &mut Reader<'_>) -> Result<TenderInfo, WireError> {
    Ok(TenderInfo {
        tender_id: r.u128()?,
        participant_id: r.u128()?,
        instrument_id: r.u128()?,
        side: Side::from_wire(r.u8()?)?,
        quantity: r.i64()?,
        price: r.i64()?,
        expires_at_ns: r.u64()?,
        status: TenderStatus::from_wire(r.u8()?)?,
    })
}

fn write_tender(w: &mut Writer<'_>, tender: &TenderInfo) {
    w.u128(tender.tender_id);
    w.u128(tender.participant_id);
    w.u128(tender.instrument_id);
    w.u8(tender.side.to_wire());
    w.i64(tender.quantity);
    w.i64(tender.price);
    w.u64(tender.expires_at_ns);
    w.u8(tender.status.to_wire());
}

fn read_negotiation(r: &mut Reader<'_>) -> Result<NegotiationInfo, WireError> {
    Ok(NegotiationInfo {
        negotiation_id: r.u128()?,
        proposer_id: r.u128()?,
        counterparty_id: r.u128()?,
        instrument_id: r.u128()?,
        side: Side::from_wire(r.u8()?)?,
        quantity: r.i64()?,
        price: r.i64()?,
        expires_at_ns: r.u64()?,
        status: NegotiationStatus::from_wire(r.u8()?)?,
        awaiting: r.u128()?,
    })
}

fn write_negotiation(w: &mut Writer<'_>, negotiation: &NegotiationInfo) {
    w.u128(negotiation.negotiation_id);
    w.u128(negotiation.proposer_id);
    w.u128(negotiation.counterparty_id);
    w.u128(negotiation.instrument_id);
    w.u8(negotiation.side.to_wire());
    w.i64(negotiation.quantity);
    w.i64(negotiation.price);
    w.u64(negotiation.expires_at_ns);
    w.u8(negotiation.status.to_wire());
    w.u128(negotiation.awaiting);
}

fn read_new_order(r: &mut Reader<'_>) -> Result<NewOrder, WireError> {
    let client_order_id = r.u64()?;
    let listing = r.listing()?;
    let side = Side::from_wire(r.u8()?)?;
    let quantity = r.i64()?;
    let order_type = match r.u8()? {
        1 => OrderType::Limit { price: r.i64()? },
        2 => {
            r.i64()?;
            OrderType::Market
        }
        other => {
            return Err(WireError::InvalidValue {
                field: "OrderType",
                value: u64::from(other),
            });
        }
    };
    let tif = r.u8()?;
    let expires_at_ns = r.u64()?;
    let time_in_force = match tif {
        0 => TimeInForce::Gtc,
        1 => TimeInForce::Ioc,
        2 => TimeInForce::Fok,
        3 => TimeInForce::Day,
        4 => TimeInForce::Gtd { expires_at_ns },
        other => {
            return Err(WireError::InvalidValue {
                field: "TimeInForce",
                value: u64::from(other),
            });
        }
    };
    let post_only = r.bool()?;
    let anonymous = r.bool()?;
    let display_quantity = r.option(Reader::i64)?;
    Ok(NewOrder {
        client_order_id,
        listing,
        side,
        quantity,
        order_type,
        time_in_force,
        post_only,
        anonymous,
        display_quantity,
    })
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per message keeps each field order beside its decoder"
)]
fn write_client(w: &mut Writer<'_>, message: &ClientMessage) -> Result<(), WireError> {
    match message {
        ClientMessage::Hello {
            version,
            resume_after,
            client_name,
        } => {
            w.bytes(&MAGIC);
            w.u16(*version);
            w.option(*resume_after, Writer::u64);
            w.string(client_name)?;
        }
        ClientMessage::Heartbeat => {}
        ClientMessage::ProbeReply { probe_id } => w.u64(*probe_id),
        ClientMessage::Ping { nonce } => w.u64(*nonce),
        ClientMessage::Logout { reason } => w.string(reason)?,
        ClientMessage::NewOrder(order) => {
            w.u64(order.client_order_id);
            w.listing(order.listing);
            w.u8(order.side.to_wire());
            w.i64(order.quantity);
            match order.order_type {
                OrderType::Limit { price } => {
                    w.u8(1);
                    w.i64(price);
                }
                OrderType::Market => {
                    w.u8(2);
                    w.i64(0);
                }
            }
            let (tif, expires) = match order.time_in_force {
                TimeInForce::Gtc => (0, 0),
                TimeInForce::Ioc => (1, 0),
                TimeInForce::Fok => (2, 0),
                TimeInForce::Day => (3, 0),
                TimeInForce::Gtd { expires_at_ns } => (4, expires_at_ns),
            };
            w.u8(tif);
            w.u64(expires);
            w.bool(order.post_only);
            w.bool(order.anonymous);
            w.option(order.display_quantity, Writer::i64);
        }
        ClientMessage::CancelOrder { client_order_id } => w.u64(*client_order_id),
        ClientMessage::KillSwitch { request_id }
        | ClientMessage::KillSwitchReset { request_id } => w.u64(*request_id),
        ClientMessage::Subscribe {
            request_id,
            listing,
            flags,
        } => {
            w.u32(*request_id);
            w.listing(*listing);
            w.u8(flags.to_wire());
        }
        ClientMessage::Unsubscribe { request_id }
        | ClientMessage::ListingsRequest { request_id }
        | ClientMessage::OpenOrdersRequest { request_id }
        | ClientMessage::AccountRequest { request_id }
        | ClientMessage::DealsRequest { request_id } => w.u32(*request_id),
        ClientMessage::Enroll { secret, csr_pem } => {
            w.string(secret)?;
            w.string(csr_pem)?;
        }
        ClientMessage::ReferenceDataRequest {
            request_id,
            after_instrument,
            limit,
        } => {
            w.u32(*request_id);
            w.option(*after_instrument, Writer::u128);
            w.u16(*limit);
        }
        ClientMessage::OperatorCommand { request_id, action } => {
            w.u64(*request_id);
            write_operator_action(w, action)?;
        }
        ClientMessage::TenderDecision {
            request_id,
            tender_id,
            accept,
        } => {
            w.u64(*request_id);
            w.u128(*tender_id);
            w.bool(*accept);
        }
        ClientMessage::OtcPropose {
            request_id,
            negotiation_id,
            counterparty_id,
            instrument_id,
            side,
            quantity,
            price,
            expires_at_ns,
        } => {
            w.u64(*request_id);
            w.u64(*negotiation_id);
            w.u128(*counterparty_id);
            w.u128(*instrument_id);
            w.u8(side.to_wire());
            w.i64(*quantity);
            w.i64(*price);
            w.u64(*expires_at_ns);
        }
        ClientMessage::OtcCounter {
            request_id,
            negotiation_id,
            quantity,
            price,
        } => {
            w.u64(*request_id);
            w.u128(*negotiation_id);
            w.i64(*quantity);
            w.i64(*price);
        }
        ClientMessage::OtcDecision {
            request_id,
            negotiation_id,
            decision,
        } => {
            w.u64(*request_id);
            w.u128(*negotiation_id);
            w.u8(decision.to_wire());
        }
        ClientMessage::SnapshotRequest {
            request_id,
            listing,
            depth,
        } => {
            w.u32(*request_id);
            w.listing(*listing);
            w.u16(*depth);
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_lines,
    reason = "one arm per message keeps the field order beside its decoder"
)]
fn write_server(w: &mut Writer<'_>, message: &ServerMessage) -> Result<(), WireError> {
    match message {
        ServerMessage::Welcome(welcome) => {
            w.u16(welcome.version);
            w.u128(welcome.run_id);
            w.u128(welcome.participant_id);
            w.u8(welcome.role.to_wire());
            w.u32(welcome.heartbeat_ms);
            w.u32(welcome.max_frame_bytes);
            w.u8(welcome.resume.to_wire());
            w.u64(welcome.run_sequence);
        }
        ServerMessage::Heartbeat => {}
        ServerMessage::Probe { probe_id } => w.u64(*probe_id),
        ServerMessage::Pong {
            nonce,
            server_time_us,
        } => {
            w.u64(*nonce);
            w.u64(*server_time_us);
        }
        ServerMessage::Reject {
            request_type,
            reference,
            reason,
        } => {
            w.u8(*request_type);
            w.u64(*reference);
            w.string(reason)?;
        }
        ServerMessage::ReplayComplete { through_sequence } => w.u64(*through_sequence),
        ServerMessage::Logout { reason } => w.string(reason)?,
        ServerMessage::OperatorDone {
            request_id,
            run_sequence,
        }
        | ServerMessage::DealDone {
            request_id,
            run_sequence,
        } => {
            w.u64(*request_id);
            w.u64(*run_sequence);
        }
        ServerMessage::Tender { stamp, tender } => {
            w.stamp(*stamp);
            write_tender(w, tender);
        }
        ServerMessage::Negotiation { stamp, negotiation } => {
            w.stamp(*stamp);
            write_negotiation(w, negotiation);
        }
        ServerMessage::Deals {
            request_id,
            run_sequence,
            tenders,
            negotiations,
        } => {
            w.u32(*request_id);
            w.u64(*run_sequence);
            w.group(tenders, |w, tender| {
                write_tender(w, tender);
                Ok(())
            })?;
            w.group(negotiations, |w, negotiation| {
                write_negotiation(w, negotiation);
                Ok(())
            })?;
        }
        ServerMessage::Enrolled {
            participant_id,
            certificate_pem,
            ca_pem,
        } => {
            w.u128(*participant_id);
            w.string(certificate_pem)?;
            w.string(ca_pem)?;
        }
        ServerMessage::ReferenceData {
            request_id,
            run_sequence,
            next_after,
            currencies,
            instruments,
            listings,
        } => {
            w.u32(*request_id);
            w.u64(*run_sequence);
            w.option(*next_after, Writer::u128);
            w.group(currencies, |w, currency| {
                w.u128(currency.currency_id);
                w.string(&currency.code)?;
                w.string(&currency.name)?;
                w.u8(currency.minor_decimals);
                Ok(())
            })?;
            w.group(instruments, write_instrument)?;
            w.group(listings, |w, listing| {
                w.listing(listing.listing);
                w.string(&listing.symbol)?;
                w.i64(listing.min_price);
                w.i64(listing.max_price);
                w.i64(listing.tick_size);
                w.i128(listing.taker_fee_per_lot);
                w.i128(listing.maker_fee_per_lot);
                w.bool(listing.broker_ids);
                w.bool(listing.primary);
                w.bool(listing.halted);
                w.bool(listing.in_window);
                Ok(())
            })?;
        }
        ServerMessage::RunStatus { stamp, status } => {
            w.stamp(*stamp);
            w.string(status)?;
        }
        ServerMessage::ListingHalt {
            stamp,
            instrument_id,
            halted,
        } => {
            w.stamp(*stamp);
            w.u128(*instrument_id);
            w.bool(*halted);
        }
        ServerMessage::News {
            stamp,
            news_id,
            headline,
            body,
        } => {
            w.stamp(*stamp);
            w.u128(*news_id);
            w.string(headline)?;
            w.string(body)?;
        }
        ServerMessage::AccessDelay {
            source,
            min_rtt_us,
            one_way_us,
            probe_samples,
        } => {
            w.u8(source.to_wire());
            w.option(*min_rtt_us, Writer::u64);
            w.option(*one_way_us, Writer::u64);
            w.u64(*probe_samples);
        }
        ServerMessage::OrderSnapshot {
            request_id,
            listing,
            next_report,
            complete,
            orders,
        } => {
            w.u32(*request_id);
            w.listing(*listing);
            w.u64(*next_report);
            w.bool(*complete);
            w.group(orders, |w, order| {
                w.u8(order.side.to_wire());
                w.u64(order.reference);
                w.i64(order.price);
                w.i64(order.quantity);
                w.option(order.broker, Writer::u128);
                w.u32(order.queue_position);
                Ok(())
            })?;
        }
        ServerMessage::OrderUpdate {
            request_id,
            listing,
            first_report,
            entries,
        } => {
            w.u32(*request_id);
            w.listing(*listing);
            w.u64(*first_report);
            w.group(entries, |w, entry| {
                match *entry {
                    OrderEntry::Trade {
                        price,
                        quantity,
                        resting_reference,
                        buyer,
                        seller,
                    } => {
                        w.u8(ORDER_ENTRY_TRADE);
                        w.i64(price);
                        w.i64(quantity);
                        w.option(resting_reference, Writer::u64);
                        w.option(buyer, Writer::u128);
                        w.option(seller, Writer::u128);
                    }
                    OrderEntry::Order {
                        action,
                        side,
                        reference,
                        price,
                        quantity,
                        broker,
                    } => {
                        w.u8(action.to_wire());
                        w.u8(side.to_wire());
                        w.u64(reference);
                        w.i64(price);
                        w.i64(quantity);
                        w.option(broker, Writer::u128);
                    }
                }
                Ok(())
            })?;
        }
        ServerMessage::TapeSnapshot {
            request_id,
            instrument_id,
            last_report,
            quotes,
        } => {
            w.u32(*request_id);
            w.u128(*instrument_id);
            w.u64(*last_report);
            w.group(quotes, |w, quote| {
                w.u128(quote.venue_id);
                w.u8(quote.side.to_wire());
                w.i64(quote.price);
                w.i64(quote.quantity);
                Ok(())
            })?;
        }
        ServerMessage::TapeUpdate {
            request_id,
            instrument_id,
            first_report,
            entries,
        } => {
            w.u32(*request_id);
            w.u128(*instrument_id);
            w.u64(*first_report);
            w.group(entries, |w, entry| {
                w.u128(entry.venue_id);
                w.u8(entry.kind.to_wire());
                w.i64(entry.price);
                w.i64(entry.quantity);
                Ok(())
            })?;
        }
        ServerMessage::OrderAccepted {
            stamp,
            client_order_id,
            order_id,
            listing,
            side,
            quantity,
            price,
        } => {
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u128(*order_id);
            w.listing(*listing);
            w.u8(side.to_wire());
            w.i64(*quantity);
            w.option(*price, Writer::i64);
        }
        ServerMessage::OrderRejected {
            stamp,
            client_order_id,
            reason,
        }
        | ServerMessage::CancelRejected {
            stamp,
            client_order_id,
            reason,
        } => {
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u8(reason.to_wire());
        }
        ServerMessage::OrderRested {
            stamp,
            client_order_id,
            order_id,
            price,
            remaining,
            reference,
        } => {
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u128(*order_id);
            w.i64(*price);
            w.i64(*remaining);
            w.option(*reference, Writer::u64);
        }
        ServerMessage::OrderReduced {
            stamp,
            client_order_id,
            order_id,
            remaining,
            reference,
        } => {
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u128(*order_id);
            w.i64(*remaining);
            w.option(*reference, Writer::u64);
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
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u128(*order_id);
            w.listing(*listing);
            w.u8(side.to_wire());
            w.i64(*price);
            w.i64(*quantity);
            w.i128(*fee);
            w.u8(liquidity.to_wire());
        }
        ServerMessage::OrderDone {
            stamp,
            client_order_id,
            order_id,
        } => {
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u128(*order_id);
        }
        ServerMessage::OrderCanceled {
            stamp,
            client_order_id,
            order_id,
            remaining,
            reason,
        } => {
            w.stamp(*stamp);
            w.u64(*client_order_id);
            w.u128(*order_id);
            w.i64(*remaining);
            w.u8(reason.to_wire());
        }
        ServerMessage::PositionChanged {
            stamp,
            instrument_id,
            delta,
        } => {
            w.stamp(*stamp);
            w.u128(*instrument_id);
            w.i64(*delta);
        }
        ServerMessage::BalanceChanged { stamp, delta } => {
            w.stamp(*stamp);
            w.i128(*delta);
        }
        ServerMessage::KillSwitchActivated { stamp } | ServerMessage::KillSwitchReset { stamp } => {
            w.stamp(*stamp);
        }
        ServerMessage::MarketSnapshot {
            request_id,
            listing,
            next_report,
            bids,
            asks,
        } => {
            w.u32(*request_id);
            w.listing(*listing);
            w.u64(*next_report);
            w.group(bids, |w, level| {
                w.level(*level);
                Ok(())
            })?;
            w.group(asks, |w, level| {
                w.level(*level);
                Ok(())
            })?;
        }
        ServerMessage::MarketUpdate {
            request_id,
            listing,
            first_report,
            entries,
        } => {
            w.u32(*request_id);
            w.listing(*listing);
            w.u64(*first_report);
            w.group(entries, |w, entry| {
                w.u8(entry.kind.to_wire());
                w.i64(entry.price);
                w.i64(entry.quantity);
                Ok(())
            })?;
        }
        ServerMessage::Listings {
            request_id,
            listings,
        } => {
            w.u32(*request_id);
            w.group(listings, |w, info| {
                w.listing(info.listing);
                w.string(&info.symbol)
            })?;
        }
        ServerMessage::OpenOrders {
            request_id,
            run_sequence,
            orders,
        } => {
            w.u32(*request_id);
            w.u64(*run_sequence);
            w.group(orders, |w, order| {
                w.u64(order.client_order_id);
                w.u128(order.order_id);
                w.listing(order.listing);
                w.u8(order.side.to_wire());
                w.i64(order.price);
                w.i64(order.original_quantity);
                w.i64(order.remaining_quantity);
                Ok(())
            })?;
        }
        ServerMessage::Account {
            request_id,
            run_sequence,
            cash,
            positions,
        } => {
            w.u32(*request_id);
            w.u64(*run_sequence);
            w.group(cash, |w, balance| {
                w.u128(balance.currency_id);
                w.i128(balance.balance);
                w.i128(balance.reserved);
                Ok(())
            })?;
            w.group(positions, |w, position| {
                w.u128(position.instrument_id);
                w.i64(position.position);
                w.i64(position.open_buy);
                w.i64(position.open_sell);
                Ok(())
            })?;
        }
    }
    Ok(())
}

struct Writer<'a>(&'a mut Vec<u8>);

impl Writer<'_> {
    fn bytes(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }
    fn u8(&mut self, value: u8) {
        self.0.push(value);
    }
    fn bool(&mut self, value: bool) {
        self.0.push(u8::from(value));
    }
    fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }
    fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }
    fn u64(&mut self, value: u64) {
        self.bytes(&value.to_le_bytes());
    }
    fn i64(&mut self, value: i64) {
        self.bytes(&value.to_le_bytes());
    }
    fn u128(&mut self, value: u128) {
        self.bytes(&value.to_le_bytes());
    }
    fn i128(&mut self, value: i128) {
        self.bytes(&value.to_le_bytes());
    }
    fn option<T>(&mut self, value: Option<T>, write: impl FnOnce(&mut Self, T)) {
        match value {
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
            None => self.u8(0),
        }
    }
    fn string(&mut self, value: &str) -> Result<(), WireError> {
        let length = u16::try_from(value.len())
            .ok()
            .filter(|length| usize::from(*length) <= MAX_STRING_BYTES)
            .ok_or(WireError::StringTooLong(value.len()))?;
        self.u16(length);
        self.bytes(value.as_bytes());
        Ok(())
    }
    fn listing(&mut self, listing: Listing) {
        self.u128(listing.venue_id);
        self.u128(listing.instrument_id);
    }
    fn stamp(&mut self, stamp: Stamp) {
        self.u64(stamp.sequence);
        self.u64(stamp.logical_time_ns);
    }
    fn level(&mut self, level: Level) {
        self.i64(level.price);
        self.i64(level.quantity);
    }
    fn group<T>(
        &mut self,
        items: &[T],
        mut write: impl FnMut(&mut Self, &T) -> Result<(), WireError>,
    ) -> Result<(), WireError> {
        let count = u16::try_from(items.len())
            .ok()
            .filter(|count| usize::from(*count) <= MAX_GROUP_ENTRIES)
            .ok_or(WireError::GroupTooLarge(items.len()))?;
        self.u16(count);
        for item in items {
            write(self, item)?;
        }
        Ok(())
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, count: usize) -> Result<&'a [u8], WireError> {
        if self.0.len() < count {
            return Err(WireError::Truncated);
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut array = [0; N];
        array.copy_from_slice(self.bytes(N)?);
        Ok(array)
    }
    fn u8(&mut self) -> Result<u8, WireError> {
        Ok(self.array::<1>()?[0])
    }
    fn bool(&mut self) -> Result<bool, WireError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            other => Err(WireError::InvalidValue {
                field: "bool",
                value: u64::from(other),
            }),
        }
    }
    fn u16(&mut self) -> Result<u16, WireError> {
        self.array().map(u16::from_le_bytes)
    }
    fn u32(&mut self) -> Result<u32, WireError> {
        self.array().map(u32::from_le_bytes)
    }
    fn u64(&mut self) -> Result<u64, WireError> {
        self.array().map(u64::from_le_bytes)
    }
    fn i64(&mut self) -> Result<i64, WireError> {
        self.array().map(i64::from_le_bytes)
    }
    fn u128(&mut self) -> Result<u128, WireError> {
        self.array().map(u128::from_le_bytes)
    }
    fn i128(&mut self) -> Result<i128, WireError> {
        self.array().map(i128::from_le_bytes)
    }
    fn option<T>(
        &mut self,
        read: impl FnOnce(&mut Self) -> Result<T, WireError>,
    ) -> Result<Option<T>, WireError> {
        if self.bool()? {
            read(self).map(Some)
        } else {
            Ok(None)
        }
    }
    fn string(&mut self) -> Result<String, WireError> {
        let length = usize::from(self.u16()?);
        if length > MAX_STRING_BYTES {
            return Err(WireError::StringTooLong(length));
        }
        String::from_utf8(self.bytes(length)?.to_vec()).map_err(|_| WireError::InvalidUtf8)
    }
    fn listing(&mut self) -> Result<Listing, WireError> {
        Ok(Listing {
            venue_id: self.u128()?,
            instrument_id: self.u128()?,
        })
    }
    fn stamp(&mut self) -> Result<Stamp, WireError> {
        Ok(Stamp {
            sequence: self.u64()?,
            logical_time_ns: self.u64()?,
        })
    }
    fn level(&mut self) -> Result<Level, WireError> {
        Ok(Level {
            price: self.i64()?,
            quantity: self.i64()?,
        })
    }
    fn group<T>(
        &mut self,
        mut read: impl FnMut(&mut Self) -> Result<T, WireError>,
    ) -> Result<Vec<T>, WireError> {
        let count = usize::from(self.u16()?);
        if count > MAX_GROUP_ENTRIES {
            return Err(WireError::GroupTooLarge(count));
        }
        // Each entry is at least one byte, so the remaining body bounds
        // the allocation before any entry is read.
        if count > self.0.len() {
            return Err(WireError::Truncated);
        }
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(read(self)?);
        }
        Ok(items)
    }
    const fn finish(&self) -> Result<(), WireError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(WireError::TrailingBytes)
        }
    }
}

fn write_instrument(w: &mut Writer<'_>, instrument: &InstrumentInfo) -> Result<(), WireError> {
    w.u128(instrument.instrument_id);
    w.string(&instrument.symbol)?;
    w.u8(instrument.kind.to_wire());
    w.option(instrument.underlying, Writer::u128);
    w.option(instrument.strike, Writer::i64);
    w.option(instrument.expiry_ns, Writer::u64);
    w.bool(instrument.is_call);
    w.bool(instrument.physical_delivery);
    w.u32(instrument.coupon_bps);
    w.group(&instrument.components, |w, (component, weight)| {
        w.u128(*component);
        w.i64(*weight);
        Ok(())
    })?;
    w.i64(instrument.multiplier);
    w.u128(instrument.currency_id);
    w.u8(instrument.quoted_decimals);
    w.bool(instrument.shortable);
    w.bool(instrument.tradeable);
    w.option(instrument.window_start_ns, Writer::u64);
    w.option(instrument.window_stop_ns, Writer::u64);
    w.string(&instrument.display_unit)?;
    w.string(&instrument.description)?;
    w.option(instrument.opening_mark, Writer::i64);
    w.option(instrument.max_order_quantity, Writer::i64);
    Ok(())
}

fn read_instrument(r: &mut Reader<'_>) -> Result<InstrumentInfo, WireError> {
    Ok(InstrumentInfo {
        instrument_id: r.u128()?,
        symbol: r.string()?,
        kind: InstrumentKind::from_wire(r.u8()?)?,
        underlying: r.option(Reader::u128)?,
        strike: r.option(Reader::i64)?,
        expiry_ns: r.option(Reader::u64)?,
        is_call: r.bool()?,
        physical_delivery: r.bool()?,
        coupon_bps: r.u32()?,
        components: r.group(|r| Ok((r.u128()?, r.i64()?)))?,
        multiplier: r.i64()?,
        currency_id: r.u128()?,
        quoted_decimals: r.u8()?,
        shortable: r.bool()?,
        tradeable: r.bool()?,
        window_start_ns: r.option(Reader::u64)?,
        window_stop_ns: r.option(Reader::u64)?,
        display_unit: r.string()?,
        description: r.string()?,
        opening_mark: r.option(Reader::i64)?,
        max_order_quantity: r.option(Reader::i64)?,
    })
}
