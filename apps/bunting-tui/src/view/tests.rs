//! Headless rendering: the model drawn on a test buffer.

use super::*;
use crate::ui::{Effect, Ui};
use bunting_trading_state::bunting_client::bnp_wire::{
    BookOrder, ClientMessage, DelaySource, Entry, EntryKind, ListingInfo, ResumeStatus, Role,
    ServerMessage, Stamp, TapeEntry, TapeQuote, Welcome,
};
use bunting_trading_state::{Model, SessionEvent};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const NEAR: Listing = Listing {
    venue_id: 1,
    instrument_id: 1,
};
const FAR: Listing = Listing {
    venue_id: 2,
    instrument_id: 1,
};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn book_order(side: Side, reference: u64, price: i64, quantity: i64, broker: u128) -> BookOrder {
    BookOrder {
        side,
        reference,
        price,
        quantity,
        broker: Some(broker),
        queue_position: 0,
    }
}

fn tape_quote(venue_id: u128, side: Side, price: i64, quantity: i64) -> TapeQuote {
    TapeQuote {
        venue_id,
        side,
        price,
        quantity,
    }
}

/// A connected model with two venues' books (venue 1 order by order), one
/// trade on venue 2's direct feed and the same trade on the tape.
fn market() -> Model {
    let mut model = Model::new(500);
    let mut requests = model.apply(
        SessionEvent::Connected(Welcome {
            version: 1,
            run_id: 1,
            participant_id: 7,
            role: Role::Participant,
            heartbeat_ms: 1_000,
            max_frame_bytes: 65_536,
            resume: ResumeStatus::Live,
            run_sequence: 0,
        }),
        0,
    );
    requests.extend(model.apply(
        SessionEvent::Message(ServerMessage::Listings {
            request_id: 1,
            listings: vec![
                ListingInfo {
                    listing: NEAR,
                    symbol: "BNT".to_owned(),
                },
                ListingInfo {
                    listing: FAR,
                    symbol: "BNT".to_owned(),
                },
            ],
        }),
        10,
    ));
    for request in requests {
        if let ClientMessage::Subscribe {
            request_id,
            listing,
            ..
        } = request
        {
            let message = if listing.venue_id == 0 {
                ServerMessage::TapeSnapshot {
                    request_id,
                    instrument_id: 1,
                    last_report: 0,
                    quotes: vec![
                        tape_quote(1, Side::Buy, 99, 5),
                        tape_quote(1, Side::Sell, 101, 8),
                        tape_quote(2, Side::Buy, 98, 5),
                        tape_quote(2, Side::Sell, 103, 8),
                    ],
                }
            } else if listing == NEAR {
                ServerMessage::OrderSnapshot {
                    request_id,
                    listing,
                    next_report: 1,
                    complete: true,
                    orders: vec![
                        book_order(Side::Buy, 1, 99, 3, 4),
                        book_order(Side::Buy, 2, 99, 2, 5),
                        book_order(Side::Sell, 3, 101, 8, 4),
                    ],
                }
            } else {
                ServerMessage::MarketSnapshot {
                    request_id,
                    listing,
                    next_report: 1,
                    bids: vec![Level {
                        price: 98,
                        quantity: 5,
                    }],
                    asks: vec![Level {
                        price: 103,
                        quantity: 8,
                    }],
                }
            };
            model.apply(SessionEvent::Message(message), 20);
            if listing == FAR {
                model.apply(
                    SessionEvent::Message(ServerMessage::MarketUpdate {
                        request_id,
                        listing,
                        first_report: 1,
                        entries: vec![Entry {
                            kind: EntryKind::Trade,
                            price: 103,
                            quantity: 2,
                        }],
                    }),
                    1_234_567,
                );
            }
            if listing.venue_id == 0 {
                model.apply(
                    SessionEvent::Message(ServerMessage::TapeUpdate {
                        request_id,
                        instrument_id: 1,
                        first_report: 1,
                        entries: vec![TapeEntry {
                            venue_id: 2,
                            kind: EntryKind::Trade,
                            price: 103,
                            quantity: 2,
                        }],
                    }),
                    1_456_789,
                );
            }
        }
    }
    model.apply(SessionEvent::RoundTrip { micros: 420 }, 30);
    model
}

fn draw(model: &Model, ui: &Ui) -> Result<String, String> {
    let mut terminal =
        Terminal::new(TestBackend::new(160, 40)).map_err(|error| error.to_string())?;
    let screen = Screen {
        model,
        ui,
        label: "venue.test:9443",
        now_us: 2_000_000,
    };
    terminal
        .draw(|frame| render(frame, &screen))
        .map_err(|error| error.to_string())?;
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            text.push_str(buffer[(x, y)].symbol());
        }
        text.push('\n');
    }
    Ok(text)
}

#[test]
fn every_venue_book_the_trades_and_the_connection_are_drawn() -> Result<(), String> {
    let screen = draw(&market(), &Ui::default())?;
    assert!(
        screen.contains("● connected · participant 7 · run 1"),
        "{screen}"
    );
    assert!(screen.contains("ping 420 µs"), "{screen}");
    assert!(screen.contains("BNT · Venue 1 · live"), "{screen}");
    assert!(screen.contains("BNT · Venue 2 · live"), "{screen}");
    assert!(screen.contains("spread 2"), "{screen}");
    assert!(screen.contains("spread 5"), "{screen}");
    assert!(
        screen.contains("Direct feeds, as they reached you"),
        "{screen}"
    );
    assert!(
        screen.contains("1.234"),
        "the trade's arrival time: {screen}"
    );
    Ok(())
}

#[test]
fn the_tape_shows_every_venues_quote_and_its_own_arrival_times() -> Result<(), String> {
    let screen = draw(&market(), &Ui::default())?;
    assert!(screen.contains("Tape from the hub · live"), "{screen}");
    assert!(screen.contains("5 × 99"), "venue 1's bid: {screen}");
    assert!(screen.contains("8 × 103"), "venue 2's offer: {screen}");
    assert!(
        screen.contains("1.456"),
        "the tape trade arrives on its own path: {screen}"
    );
    Ok(())
}

#[test]
fn o_shows_every_order_with_its_queue_place_and_broker() -> Result<(), String> {
    let mut model = market();
    let mut ui = Ui::default();
    let levels = draw(&model, &ui)?;
    assert!(levels.contains("Venue 1 · live · levels"), "{levels}");
    ui.handle(key(KeyCode::Char('o')), &mut model, 1);
    let orders = draw(&model, &ui)?;
    assert!(orders.contains("Venue 1 · live · orders"), "{orders}");
    assert!(orders.contains("1 b4"), "{orders}");
    assert!(orders.contains("2 b5"), "{orders}");
    // Venue 2 sent price levels only.
    assert!(orders.contains("Venue 2 · live · levels only"), "{orders}");
    Ok(())
}

#[test]
fn the_venue_measured_access_delay_is_shown() -> Result<(), String> {
    let mut model = market();
    model.apply(
        SessionEvent::Message(ServerMessage::AccessDelay {
            source: DelaySource::Kernel,
            min_rtt_us: Some(180),
            one_way_us: Some(90),
            probe_samples: 0,
        }),
        40,
    );
    let screen = draw(&model, &Ui::default())?;
    assert!(screen.contains("venue measures 90 µs one way"), "{screen}");
    Ok(())
}

#[test]
fn a_ticket_sends_to_the_chosen_venue_and_the_order_is_listed() -> Result<(), String> {
    let mut model = market();
    let mut ui = Ui::default();
    // Choose venue 2, open a buy ticket: it joins venue 2's best bid.
    ui.handle(key(KeyCode::Right), &mut model, 100);
    ui.handle(key(KeyCode::Char('b')), &mut model, 100);
    let form = ui.ticket.as_ref().ok_or("ticket open")?;
    assert_eq!(form.ticket.listing, Some(FAR));
    assert_eq!(form.price, "98");
    let ticket = draw(&model, &ui)?;
    assert!(ticket.contains("New BUY order"), "{ticket}");
    assert!(ticket.contains("Venue 2"), "{ticket}");
    // Quantity 12: clear the default 1, type 12, send.
    ui.handle(key(KeyCode::Backspace), &mut model, 100);
    ui.handle(key(KeyCode::Char('1')), &mut model, 100);
    ui.handle(key(KeyCode::Char('2')), &mut model, 100);
    let effects = ui.handle(key(KeyCode::Enter), &mut model, 150);
    let [Effect::Send(ClientMessage::NewOrder(order))] = effects.as_slice() else {
        return Err(format!("expected one order, got {effects:?}"));
    };
    assert_eq!((order.listing, order.quantity), (FAR, 12));
    assert!(ui.ticket.is_none());
    model.apply(
        SessionEvent::Message(ServerMessage::OrderAccepted {
            stamp: Stamp {
                sequence: 3,
                logical_time_ns: 3,
            },
            client_order_id: order.client_order_id,
            order_id: 9,
            listing: FAR,
            side: Side::Buy,
            quantity: 12,
            price: Some(98),
        }),
        4_350,
    );
    let screen = draw(&model, &ui)?;
    assert!(screen.contains("1 Orders (1 live)"), "{screen}");
    assert!(screen.contains("accepted"), "{screen}");
    assert!(
        screen.contains("4.20 ms"),
        "the venue's answer time: {screen}"
    );
    assert!(screen.contains("order rt 4.20 ms"), "{screen}");
    // Cancel it from the orders list.
    let effects = ui.handle(key(KeyCode::Char('c')), &mut model, 200);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Send(ClientMessage::CancelOrder { .. })]
    ));
    Ok(())
}

#[test]
fn the_kill_switch_asks_first() -> Result<(), String> {
    let mut model = market();
    let mut ui = Ui::default();
    assert!(ui.handle(key(KeyCode::Char('K')), &mut model, 1).is_empty());
    assert!(draw(&model, &ui)?.contains("y to activate the kill switch"));
    assert!(ui.handle(key(KeyCode::Char('n')), &mut model, 1).is_empty());
    ui.handle(key(KeyCode::Char('K')), &mut model, 1);
    assert!(matches!(
        ui.handle(key(KeyCode::Char('y')), &mut model, 1).as_slice(),
        [Effect::Send(ClientMessage::KillSwitch { .. })]
    ));
    model.apply(
        SessionEvent::Message(ServerMessage::KillSwitchActivated {
            stamp: Stamp {
                sequence: 5,
                logical_time_ns: 5,
            },
        }),
        2,
    );
    // Once the venue reports it, K offers to reset it.
    ui.handle(key(KeyCode::Char('K')), &mut model, 3);
    assert!(draw(&model, &ui)?.contains("y to reset it"));
    assert!(matches!(
        ui.handle(key(KeyCode::Char('y')), &mut model, 3).as_slice(),
        [Effect::Send(ClientMessage::KillSwitchReset { .. })]
    ));
    Ok(())
}

#[test]
fn a_bad_ticket_stays_open_with_the_reason() -> Result<(), String> {
    let mut model = Model::new(1);
    let mut ui = Ui::default();
    ui.handle(key(KeyCode::Char('s')), &mut model, 1);
    assert!(ui.handle(key(KeyCode::Enter), &mut model, 1).is_empty());
    let form = ui.ticket.as_ref().ok_or("ticket stays open")?;
    assert_eq!(form.error.as_deref(), Some("choose a venue listing"));
    assert!(draw(&model, &ui)?.contains("choose a venue listing"));
    Ok(())
}
