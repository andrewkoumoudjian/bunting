use super::*;

fn listing() -> Listing {
    Listing {
        venue_id: 2,
        instrument_id: u128::MAX - 7,
    }
}

fn stamp() -> Stamp {
    Stamp {
        sequence: 41,
        logical_time_ns: 1_700_000_000_123_456_789,
    }
}

fn clients() -> Vec<ClientMessage> {
    vec![
        ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            resume_after: Some(17),
            client_name: "bunting-trader/0.1 ü".to_owned(),
        },
        ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            resume_after: None,
            client_name: String::new(),
        },
        ClientMessage::Heartbeat,
        ClientMessage::ProbeReply { probe_id: 9 },
        ClientMessage::Ping { nonce: u64::MAX },
        ClientMessage::Logout {
            reason: "done".to_owned(),
        },
        ClientMessage::NewOrder(NewOrder {
            client_order_id: 1,
            listing: listing(),
            side: Side::Buy,
            quantity: 10,
            order_type: OrderType::Limit { price: -5 },
            time_in_force: TimeInForce::Gtd { expires_at_ns: 123 },
            post_only: true,
            anonymous: false,
            display_quantity: Some(2),
        }),
        ClientMessage::NewOrder(NewOrder {
            client_order_id: u64::MAX,
            listing: listing(),
            side: Side::Sell,
            quantity: 1,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            anonymous: false,
            display_quantity: None,
        }),
        ClientMessage::CancelOrder { client_order_id: 3 },
        ClientMessage::KillSwitch { request_id: 4 },
        ClientMessage::Subscribe {
            request_id: 5,
            listing: listing(),
            flags: FeedFlags::BIDS.union(FeedFlags::TRADES),
        },
        ClientMessage::Unsubscribe { request_id: 5 },
        ClientMessage::SnapshotRequest {
            request_id: 6,
            listing: listing(),
            depth: 0,
        },
        ClientMessage::ListingsRequest { request_id: 7 },
        ClientMessage::OpenOrdersRequest { request_id: 8 },
        ClientMessage::AccountRequest { request_id: 9 },
    ]
}

#[expect(clippy::too_many_lines, reason = "one value of every server message")]
fn servers() -> Vec<ServerMessage> {
    vec![
        ServerMessage::Welcome(Welcome {
            version: PROTOCOL_VERSION,
            run_id: 1,
            participant_id: 2,
            role: Role::Participant,
            heartbeat_ms: 1_000,
            max_frame_bytes: 65_536,
            resume: ResumeStatus::Replaying,
            run_sequence: 99,
        }),
        ServerMessage::Heartbeat,
        ServerMessage::Probe { probe_id: 1 },
        ServerMessage::Pong {
            nonce: 2,
            server_time_us: 3,
        },
        ServerMessage::Reject {
            request_type: msg_type::NEW_ORDER,
            reference: 4,
            reason: "rate limit".to_owned(),
        },
        ServerMessage::ReplayComplete {
            through_sequence: 5,
        },
        ServerMessage::Logout {
            reason: "bye".to_owned(),
        },
        ServerMessage::OrderAccepted {
            stamp: stamp(),
            client_order_id: 1,
            order_id: u128::MAX,
            listing: listing(),
            side: Side::Sell,
            quantity: 3,
            price: None,
        },
        ServerMessage::OrderRejected {
            stamp: stamp(),
            client_order_id: 1,
            reason: RejectReason::UnknownListing,
        },
        ServerMessage::OrderRested {
            stamp: stamp(),
            client_order_id: 1,
            order_id: 7,
            price: 101,
            remaining: 2,
        },
        ServerMessage::OrderReduced {
            stamp: stamp(),
            client_order_id: 1,
            order_id: 7,
            remaining: 1,
        },
        ServerMessage::Fill {
            stamp: stamp(),
            client_order_id: 1,
            order_id: 7,
            listing: listing(),
            side: Side::Buy,
            price: 100,
            quantity: 1,
            fee: -12,
            liquidity: Liquidity::Maker,
        },
        ServerMessage::OrderDone {
            stamp: stamp(),
            client_order_id: 1,
            order_id: 7,
        },
        ServerMessage::OrderCanceled {
            stamp: stamp(),
            client_order_id: 1,
            order_id: 7,
            remaining: 0,
            reason: CancelReason::Expired,
        },
        ServerMessage::CancelRejected {
            stamp: stamp(),
            client_order_id: 1,
            reason: RejectReason::UnknownOrder,
        },
        ServerMessage::PositionChanged {
            stamp: stamp(),
            instrument_id: 1,
            delta: -4,
        },
        ServerMessage::BalanceChanged {
            stamp: stamp(),
            delta: i128::MIN,
        },
        ServerMessage::KillSwitchActivated { stamp: stamp() },
        ServerMessage::MarketSnapshot {
            request_id: 1,
            listing: listing(),
            next_report: 1,
            bids: vec![
                Level {
                    price: 99,
                    quantity: 5,
                },
                Level {
                    price: 98,
                    quantity: 1,
                },
            ],
            asks: Vec::new(),
        },
        ServerMessage::MarketUpdate {
            request_id: 1,
            listing: listing(),
            first_report: 3,
            entries: vec![
                Entry {
                    kind: EntryKind::Trade,
                    price: 100,
                    quantity: 2,
                },
                Entry {
                    kind: EntryKind::Ask,
                    price: 100,
                    quantity: 0,
                },
            ],
        },
        ServerMessage::Listings {
            request_id: 2,
            listings: vec![ListingInfo {
                listing: listing(),
                symbol: "BNT.B".to_owned(),
            }],
        },
        ServerMessage::OpenOrders {
            request_id: 3,
            run_sequence: 4,
            orders: vec![OpenOrder {
                client_order_id: 0,
                order_id: 5,
                listing: listing(),
                side: Side::Buy,
                price: 6,
                original_quantity: 7,
                remaining_quantity: 8,
            }],
        },
        ServerMessage::Account {
            request_id: 4,
            run_sequence: 5,
            cash: vec![CashBalance {
                currency_id: 1,
                balance: 1_000,
                reserved: 10,
            }],
            positions: vec![Position {
                instrument_id: 1,
                position: -3,
                open_buy: 0,
                open_sell: 4,
            }],
        },
    ]
}

#[test]
fn every_message_round_trips_through_frames() -> Result<(), WireError> {
    let mut stream = Vec::new();
    for message in clients() {
        encode_client(&message, &mut stream)?;
    }
    let mut decoder = FrameDecoder::new(MAX_FRAME_BYTES);
    // One byte at a time: frames complete only when their last byte arrives.
    let mut frames = Vec::new();
    for byte in &stream {
        frames.extend(decoder.push(std::slice::from_ref(byte))?);
    }
    assert_eq!(decoder.pending_bytes(), 0);
    let clients_decoded = frames
        .iter()
        .map(|frame| decode_client(frame))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(clients_decoded, clients());

    let mut stream = Vec::new();
    for message in servers() {
        encode_server(&message, &mut stream)?;
    }
    let frames = FrameDecoder::new(MAX_FRAME_BYTES).push(&stream)?;
    let servers_decoded = frames
        .iter()
        .map(|frame| decode_server(frame))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(servers_decoded, servers());
    Ok(())
}

#[test]
fn every_message_type_is_covered_and_directional() -> Result<(), WireError> {
    let mut client_types: Vec<u8> = clients().iter().map(ClientMessage::msg_type).collect();
    let mut server_types: Vec<u8> = servers().iter().map(ServerMessage::msg_type).collect();
    client_types.dedup();
    server_types.dedup();
    assert_eq!(client_types.len(), 14);
    assert_eq!(server_types.len(), 23);
    for message in servers() {
        let mut frame = Vec::new();
        encode_server(&message, &mut frame)?;
        assert!(matches!(
            decode_client(&frame[LENGTH_BYTES..]),
            Err(WireError::WrongDirection(_))
        ));
    }
    for message in clients() {
        let mut frame = Vec::new();
        encode_client(&message, &mut frame)?;
        assert!(matches!(
            decode_server(&frame[LENGTH_BYTES..]),
            Err(WireError::WrongDirection(_))
        ));
    }
    Ok(())
}

#[test]
fn the_layout_is_fixed_little_endian() -> Result<(), WireError> {
    let mut frame = Vec::new();
    encode_client(
        &ClientMessage::CancelOrder {
            client_order_id: 0x0102,
        },
        &mut frame,
    )?;
    assert_eq!(
        frame,
        [9, 0, 0, 0, msg_type::CANCEL_ORDER, 2, 1, 0, 0, 0, 0, 0, 0]
    );
    frame.clear();
    encode_client(
        &ClientMessage::Hello {
            version: 1,
            resume_after: None,
            client_name: "a".to_owned(),
        },
        &mut frame,
    )?;
    assert_eq!(
        frame,
        [
            11,
            0,
            0,
            0,
            msg_type::HELLO,
            b'B',
            b'N',
            b'P',
            b'1',
            1,
            0,
            0,
            1,
            0,
            b'a'
        ]
    );
    Ok(())
}

#[test]
fn malformed_frames_are_errors_never_skipped() -> Result<(), WireError> {
    let mut frame = Vec::new();
    encode_client(&ClientMessage::Ping { nonce: 1 }, &mut frame)?;
    let body = &frame[LENGTH_BYTES..];
    assert_eq!(
        decode_client(&body[..body.len() - 1]),
        Err(WireError::Truncated)
    );
    let mut longer = body.to_vec();
    longer.push(0);
    assert_eq!(decode_client(&longer), Err(WireError::TrailingBytes));
    assert_eq!(
        decode_client(&[0x3f]),
        Err(WireError::UnknownMessageType(0x3f))
    );
    assert_eq!(decode_client(&[]), Err(WireError::EmptyFrame));
    assert_eq!(
        decode_client(&[msg_type::HELLO, b'F', b'I', b'X', b'T', 1, 0, 0, 0, 0]),
        Err(WireError::BadMagic)
    );
    assert!(matches!(
        decode_client(
            &[msg_type::SUBSCRIBE, 0, 0, 0, 0]
                .iter()
                .copied()
                .chain([0; 32])
                .chain([8])
                .collect::<Vec<_>>()
        ),
        Err(WireError::InvalidValue {
            field: "FeedFlags",
            value: 8
        })
    ));
    // A side of 3 is not a side.
    let mut order = Vec::new();
    encode_client(
        &ClientMessage::NewOrder(NewOrder {
            client_order_id: 1,
            listing: listing(),
            side: Side::Buy,
            quantity: 1,
            order_type: OrderType::Market,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            anonymous: false,
            display_quantity: None,
        }),
        &mut order,
    )?;
    order[LENGTH_BYTES + 1 + 8 + 32] = 3;
    assert!(matches!(
        decode_client(&order[LENGTH_BYTES..]),
        Err(WireError::InvalidValue {
            field: "Side",
            value: 3
        })
    ));
    Ok(())
}

#[test]
fn frame_and_field_bounds_are_enforced() {
    let mut decoder = FrameDecoder::new(16);
    assert_eq!(
        decoder.push(&17_u32.to_le_bytes()),
        Err(WireError::FrameTooLarge {
            limit: 16,
            actual: 17
        })
    );
    assert_eq!(
        FrameDecoder::new(16).push(&0_u32.to_le_bytes()),
        Err(WireError::EmptyFrame)
    );
    let mut out = vec![0xaa];
    let long = "x".repeat(MAX_STRING_BYTES + 1);
    assert_eq!(
        encode_client(&ClientMessage::Logout { reason: long }, &mut out),
        Err(WireError::StringTooLong(MAX_STRING_BYTES + 1))
    );
    // A failed encode leaves earlier output untouched.
    assert_eq!(out, vec![0xaa]);
    let levels = vec![
        Level {
            price: 1,
            quantity: 1
        };
        MAX_GROUP_ENTRIES + 1
    ];
    assert_eq!(
        encode_server(
            &ServerMessage::MarketSnapshot {
                request_id: 1,
                listing: listing(),
                next_report: 0,
                bids: levels,
                asks: Vec::new(),
            },
            &mut out
        ),
        Err(WireError::GroupTooLarge(MAX_GROUP_ENTRIES + 1))
    );
    // A claimed group larger than the remaining body is refused before
    // allocating for it.
    let mut snapshot = vec![msg_type::MARKET_SNAPSHOT];
    snapshot.extend_from_slice(&[0; 4 + 32 + 8]);
    snapshot.extend_from_slice(&4_000_u16.to_le_bytes());
    assert_eq!(decode_server(&snapshot), Err(WireError::Truncated));
}

#[test]
fn wire_enums_cover_their_values_exactly() {
    for &reason in RejectReason::ALL {
        assert_eq!(RejectReason::from_wire(reason.to_wire()), Ok(reason));
    }
    assert!(RejectReason::from_wire(0).is_err());
    assert_eq!(RejectReason::ALL.len(), 24);
    assert_eq!(CancelReason::ALL.len(), 6);
    assert!(FeedFlags::ALL.contains(FeedFlags::OFFERS));
    assert!(!FeedFlags::BIDS.contains(FeedFlags::OFFERS));
}
