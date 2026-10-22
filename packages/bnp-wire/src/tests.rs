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
        ClientMessage::KillSwitchReset { request_id: 5 },
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
        ClientMessage::TenderDecision {
            request_id: 10,
            tender_id: 3,
            accept: true,
        },
        ClientMessage::OtcPropose {
            request_id: 11,
            negotiation_id: 4,
            counterparty_id: 2,
            instrument_id: 1,
            side: Side::Sell,
            quantity: 50,
            price: 101,
            expires_at_ns: 9_000,
        },
        ClientMessage::OtcCounter {
            request_id: 12,
            negotiation_id: u128::MAX,
            quantity: 40,
            price: 102,
        },
        ClientMessage::OtcDecision {
            request_id: 13,
            negotiation_id: 5,
            decision: OtcDecision::Break,
        },
        ClientMessage::DealsRequest { request_id: 14 },
    ]
    .into_iter()
    .chain(reference_requests())
    .chain(
        operator_actions()
            .into_iter()
            .zip(100..)
            .map(|(action, request_id)| ClientMessage::OperatorCommand { request_id, action }),
    )
    .collect()
}

/// BNP 1.1 requests.
fn reference_requests() -> Vec<ClientMessage> {
    vec![
        ClientMessage::ReferenceDataRequest {
            request_id: 15,
            after_instrument: Some(u128::MAX - 1),
            limit: 64,
        },
        ClientMessage::ReferenceDataRequest {
            request_id: 16,
            after_instrument: None,
            limit: 0,
        },
        ClientMessage::Enroll {
            secret: "00112233445566778899aabbccddeeff".to_owned(),
            csr_pem: "-----BEGIN CERTIFICATE REQUEST-----".to_owned(),
        },
    ]
}

fn tender() -> TenderInfo {
    TenderInfo {
        tender_id: 3,
        participant_id: 2,
        instrument_id: 1,
        side: Side::Buy,
        quantity: 500,
        price: 99,
        expires_at_ns: 7_000,
        status: TenderStatus::Open,
    }
}

fn negotiation() -> NegotiationInfo {
    NegotiationInfo {
        negotiation_id: u128::MAX - 1,
        proposer_id: 1,
        counterparty_id: 2,
        instrument_id: 1,
        side: Side::Sell,
        quantity: 40,
        price: 102,
        expires_at_ns: 9_000,
        status: NegotiationStatus::Countered,
        awaiting: 1,
    }
}

/// One of every operator action, including every news audience.
fn operator_actions() -> Vec<OperatorAction> {
    let news = |audience| OperatorAction::PublishNews {
        news_id: 12,
        audience,
        headline: "Rates cut".to_owned(),
        body: "By 25 bp — ü".to_owned(),
    };
    vec![
        OperatorAction::Start,
        OperatorAction::Pause,
        OperatorAction::Resume,
        OperatorAction::Advance { steps: 3 },
        OperatorAction::Terminate {
            reason: "round over".to_owned(),
        },
        OperatorAction::Halt {
            instrument_id: 1,
            halted: true,
            reason: "news pending".to_owned(),
        },
        OperatorAction::MassCancel {
            participant_id: Some(2),
            instrument_id: None,
        },
        news(NewsAudience::Public),
        news(NewsAudience::Participant(2)),
        news(NewsAudience::Team(7)),
        news(NewsAudience::Role("instructor".to_owned())),
        OperatorAction::Score,
        OperatorAction::Fine {
            participant_id: 2,
            currency_id: 1,
            amount: -1_000,
            reason: "late".to_owned(),
        },
        OperatorAction::OpenTender {
            tender_id: 3,
            participant_id: 2,
            instrument_id: 1,
            side: Side::Buy,
            quantity: 500,
            price: 99,
            expires_at_ns: 7_000,
        },
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
            reference: Some(41),
        },
        ServerMessage::OrderReduced {
            stamp: stamp(),
            client_order_id: 1,
            order_id: 7,
            remaining: 1,
            reference: None,
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
        ServerMessage::KillSwitchReset { stamp: stamp() },
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
        ServerMessage::OperatorDone {
            request_id: 3,
            run_sequence: 99,
        },
        ServerMessage::DealDone {
            request_id: u64::MAX,
            run_sequence: 100,
        },
        ServerMessage::Tender {
            stamp: stamp(),
            tender: tender(),
        },
        ServerMessage::Negotiation {
            stamp: stamp(),
            negotiation: negotiation(),
        },
        ServerMessage::Deals {
            request_id: 6,
            run_sequence: 101,
            tenders: vec![tender()],
            negotiations: vec![negotiation(), negotiation()],
        },
        ServerMessage::RunStatus {
            stamp: stamp(),
            status: "paused".to_owned(),
        },
        ServerMessage::ListingHalt {
            stamp: stamp(),
            instrument_id: 1,
            halted: true,
        },
        ServerMessage::News {
            stamp: stamp(),
            news_id: 12,
            headline: "Rates cut".to_owned(),
            body: String::new(),
        },
        ServerMessage::AccessDelay {
            source: DelaySource::Both,
            min_rtt_us: Some(812),
            one_way_us: Some(406),
            probe_samples: 9,
        },
        ServerMessage::AccessDelay {
            source: DelaySource::None,
            min_rtt_us: None,
            one_way_us: None,
            probe_samples: 0,
        },
        ServerMessage::OrderSnapshot {
            request_id: 4,
            listing: listing(),
            next_report: 1,
            complete: true,
            orders: vec![
                BookOrder {
                    side: Side::Buy,
                    reference: 0,
                    price: 99,
                    quantity: 5,
                    broker: Some(10),
                    queue_position: 1,
                },
                BookOrder {
                    side: Side::Sell,
                    reference: u64::MAX,
                    price: 101,
                    quantity: 2,
                    broker: None,
                    queue_position: 1,
                },
            ],
        },
        ServerMessage::OrderUpdate {
            request_id: 4,
            listing: listing(),
            first_report: 3,
            entries: vec![
                OrderEntry::Trade {
                    price: 101,
                    quantity: 1,
                    resting_reference: Some(7),
                    buyer: Some(2),
                    seller: None,
                },
                OrderEntry::Trade {
                    price: 100,
                    quantity: 1,
                    resting_reference: None,
                    buyer: None,
                    seller: Some(u128::MAX),
                },
                OrderEntry::Order {
                    action: OrderAction::Change,
                    side: Side::Sell,
                    reference: 7,
                    price: 101,
                    quantity: 1,
                    broker: Some(10),
                },
                OrderEntry::Order {
                    action: OrderAction::Delete,
                    side: Side::Buy,
                    reference: 3,
                    price: 99,
                    quantity: 0,
                    broker: None,
                },
            ],
        },
        ServerMessage::TapeSnapshot {
            request_id: 5,
            instrument_id: u128::MAX - 7,
            last_report: 12,
            quotes: vec![TapeQuote {
                venue_id: 2,
                side: Side::Sell,
                price: 101,
                quantity: 4,
            }],
        },
        ServerMessage::TapeUpdate {
            request_id: 5,
            instrument_id: u128::MAX - 7,
            first_report: 13,
            entries: vec![
                TapeEntry {
                    venue_id: 1,
                    kind: EntryKind::Trade,
                    price: 100,
                    quantity: 3,
                },
                TapeEntry {
                    venue_id: 2,
                    kind: EntryKind::Bid,
                    price: 0,
                    quantity: 0,
                },
            ],
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
        ServerMessage::ReferenceData {
            request_id: 6,
            run_sequence: 7,
            next_after: Some(3),
            currencies: vec![CurrencyInfo {
                currency_id: 1,
                code: "USD".to_owned(),
                name: "US dollar".to_owned(),
                minor_decimals: 2,
            }],
            instruments: vec![InstrumentInfo {
                instrument_id: 3,
                symbol: "SPY-C".to_owned(),
                kind: InstrumentKind::Option,
                underlying: Some(2),
                strike: Some(45_000),
                expiry_ns: Some(u64::MAX),
                is_call: true,
                physical_delivery: false,
                coupon_bps: 0,
                components: vec![(2, -1), (u128::MAX, i64::MAX)],
                multiplier: 100,
                currency_id: 1,
                quoted_decimals: 2,
                shortable: true,
                tradeable: true,
                window_start_ns: Some(0),
                window_stop_ns: None,
                display_unit: "contracts".to_owned(),
                description: "a call ü".to_owned(),
                opening_mark: Some(-1),
                max_order_quantity: None,
            }],
            listings: vec![ListingReference {
                listing: listing(),
                symbol: "SPY-C".to_owned(),
                min_price: 1,
                max_price: i64::MAX,
                tick_size: 5,
                taker_fee_per_lot: -2,
                maker_fee_per_lot: i128::MIN,
                broker_ids: true,
                primary: false,
                halted: true,
                in_window: false,
            }],
        },
        ServerMessage::Enrolled {
            participant_id: u128::MAX,
            certificate_pem: "-----BEGIN CERTIFICATE-----".to_owned(),
            ca_pem: String::new(),
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
    assert_eq!(client_types.len(), 23);
    assert_eq!(server_types.len(), 39);
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
                .chain([16])
                .collect::<Vec<_>>()
        ),
        Err(WireError::InvalidValue {
            field: "FeedFlags",
            value: 16
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
    assert_eq!(RejectReason::ALL.len(), 27);
    assert_eq!(InstrumentKind::ALL.len(), 7);
    assert_eq!(CancelReason::ALL.len(), 6);
    assert!(FeedFlags::ALL.contains(FeedFlags::OFFERS));
    assert!(!FeedFlags::BIDS.contains(FeedFlags::OFFERS));
    assert!(!FeedFlags::ALL.contains(FeedFlags::ORDERS));
    assert_eq!(
        FeedFlags::from_wire(FeedFlags::ALL.union(FeedFlags::ORDERS).to_wire()),
        Ok(FeedFlags::ALL.union(FeedFlags::ORDERS))
    );
    assert_eq!(OrderAction::ALL.len(), 3);
    assert_eq!(DelaySource::ALL.len(), 4);
    assert_eq!(TenderStatus::ALL.len(), 4);
    assert_eq!(NegotiationStatus::ALL.len(), 6);
    assert_eq!(OtcDecision::ALL.len(), 3);
}

#[test]
fn an_order_by_order_entry_has_the_documented_layout() -> Result<(), WireError> {
    let mut frame = Vec::new();
    encode_server(
        &ServerMessage::OrderUpdate {
            request_id: 1,
            listing: Listing {
                venue_id: 1,
                instrument_id: 2,
            },
            first_report: 3,
            entries: vec![OrderEntry::Order {
                action: OrderAction::Add,
                side: Side::Sell,
                reference: 4,
                price: 5,
                quantity: 6,
                broker: Some(7),
            }],
        },
        &mut frame,
    )?;
    let mut expected = vec![msg_type::ORDER_UPDATE];
    expected.extend(1_u32.to_le_bytes());
    expected.extend(1_u128.to_le_bytes());
    expected.extend(2_u128.to_le_bytes());
    expected.extend(3_u64.to_le_bytes());
    expected.extend(1_u16.to_le_bytes());
    expected.extend([1, 2]);
    expected.extend(4_u64.to_le_bytes());
    expected.extend(5_i64.to_le_bytes());
    expected.extend(6_i64.to_le_bytes());
    expected.push(1);
    expected.extend(7_u128.to_le_bytes());
    assert_eq!(&frame[LENGTH_BYTES..], expected.as_slice());
    Ok(())
}

#[test]
fn versions_negotiate_to_the_lower_1x_minor() {
    assert_eq!(negotiate(VERSION_1_0), Some(VERSION_1_0));
    assert_eq!(negotiate(256), Some(VERSION_1_0), "1.0 in the 1.1 encoding");
    assert_eq!(negotiate(VERSION_1_1), Some(VERSION_1_1));
    assert_eq!(
        negotiate(258),
        Some(VERSION_1_1),
        "a later 1.x speaks 1.1 here"
    );
    assert_eq!(negotiate(2), None);
    assert_eq!(negotiate(513), None, "2.1 is another major");
    assert!(speaks_1_1(PROTOCOL_VERSION));
    assert!(!speaks_1_1(VERSION_1_0));
}

#[test]
fn an_invite_token_round_trips_and_rejects_malformed_text() -> Result<(), String> {
    let token = InviteToken {
        address: "100.113.213.48:7400".to_owned(),
        ca_sha256: [0xab; 32],
        secret: "0123456789abcdef0123456789abcdef".to_owned(),
    };
    let text = token.to_string();
    assert!(text.starts_with("bnpi1:100.113.213.48:7400:abab"));
    assert_eq!(InviteToken::parse(&format!("  {text}\n"))?, token);
    for bad in [
        "bnpi2:h:1:00",
        "bnpi1:host:abab:0123456789abcdef0123456789abcdef",
        "bnpi1:nohostport:abababababababababababababababababababababababababababababababab:0123456789abcdef0123456789abcdef",
        "bnpi1:h:1:zz:0123456789abcdef0123456789abcdef",
        "bnpi1:h:1:abababababababababababababababababababababababababababababababab:short",
    ] {
        assert!(InviteToken::parse(bad).is_err(), "{bad}");
    }
    Ok(())
}
