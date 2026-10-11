//! The FIX competition profile over real TCP: a participant logs on to the
//! native venue and receives a book snapshot and the competition
//! projections (discovery, account, risk). This replaces the black-box test
//! that drove the old FIX terminal (the terminal now speaks BNP, ADR 0047).

mod support;

use bunting_server::config::ServerConfig;
use simfix_wire::FixMessage;
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT};

#[test]
fn a_fix_participant_receives_the_book_and_competition_projections() -> Result<(), String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.fix.as_mut().ok_or("local profile has FIX")?.bind = format!("127.0.0.1:{port}");
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut client = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let mut book = FixMessage::new("V");
    book.push(262, "book-1");
    book.push(263, "0");
    book.push(264, "20");
    book.push(267, "2");
    book.push(269, "0");
    book.push(269, "1");
    book.push(48, "1");
    book.push(207, "1");
    client.send(book)?;
    let mut discovery = FixMessage::new("x");
    discovery.push(320, "discovery-1");
    client.send(discovery)?;
    let mut account = FixMessage::new("AN");
    account.push(710, "account-1");
    client.send(account)?;
    let mut risk = FixMessage::new("UB");
    risk.push(10018, "query");
    client.send(risk)?;
    client.pump_until(|client| {
        ["W", "y", "AP", "UB"].iter().all(|kind| {
            client
                .received
                .iter()
                .any(|message| message.msg_type == *kind)
        })
    })?;
    assert!(
        client
            .received
            .iter()
            .any(|message| message.value(10010).is_some()),
        "projections carry the committed sequence"
    );
    Ok(())
}
