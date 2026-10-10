//! ADR 0034 end to end: distance does not decide, the client's own speed
//! does. A real venue, three real FIX connections, and one participant
//! routed through a proxy that adds 30 ms each way.

mod support;

use bunting_admission_sequencer::AdmissionMode;
use bunting_api_contract::ActorRole;
use bunting_server::config::{
    AdmissionConfig, RosterEntry, RttSources, ScenarioConfig, ServerConfig,
};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use support::{Client, TIMEOUT, order};

const D_US: u64 = 80_000;
const FAR_ONE_WAY: Duration = Duration::from_millis(30);
/// How much earlier the far participant sends, in true time.
const HEAD_START: Duration = Duration::from_millis(5);

fn start_server(mode: AdmissionMode) -> Result<u16, String> {
    let port = TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map_err(|error| error.to_string())?
        .port();
    let scenario = include_str!("../config/scenario.json");
    let path = std::env::temp_dir().join(format!("bunting-fair-{port}.json"));
    std::fs::write(&path, scenario).map_err(|error| error.to_string())?;
    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.scenario = Some(ScenarioConfig {
        path: path.display().to_string(),
        run_id: 1,
        iteration_id: 1,
    });
    let fix = config.fix.as_mut().ok_or("local profile has FIX")?;
    fix.bind = format!("127.0.0.1:{port}");
    fix.roster.push(RosterEntry {
        target_comp_id: "MAKER".to_owned(),
        username: "maker".to_owned(),
        password: "bunting-maker-dev".to_owned(),
        role: ActorRole::Participant,
        participant_id: 10,
    });
    fix.max_connections = 3;
    fix.admission = AdmissionConfig::equalized(D_US);
    fix.admission.policy.mode = mode;
    // The test proxy terminates TCP, exactly like a TLS terminator: the
    // kernel would see only the loopback hop, so probes must measure.
    fix.admission.rtt_sources = RttSources::ProbeOnly;
    fix.admission.probe_interval_ms = 50;
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let deadline = Instant::now() + TIMEOUT;
    while TcpStream::connect(("127.0.0.1", port)).is_err() {
        if Instant::now() > deadline {
            return Err("server did not start".to_owned());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Ok(port)
}

/// A TCP proxy adding `one_way` in each direction, preserving byte order.
fn delaying_proxy(upstream: u16, one_way: Duration) -> Result<u16, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    std::thread::spawn(move || -> std::io::Result<()> {
        let (client, _) = listener.accept()?;
        let server = TcpStream::connect(("127.0.0.1", upstream))?;
        for socket in [&client, &server] {
            socket.set_nodelay(true)?;
        }
        let pipes = [(client.try_clone()?, server.try_clone()?), (server, client)];
        for (mut from, mut to) in pipes {
            let (sender, receiver) = mpsc::channel::<(Instant, Vec<u8>)>();
            std::thread::spawn(move || {
                let mut buffer = [0_u8; 16_384];
                while let Ok(count) = from.read(&mut buffer) {
                    if count == 0
                        || sender
                            .send((Instant::now(), buffer[..count].to_vec()))
                            .is_err()
                    {
                        break;
                    }
                }
            });
            std::thread::spawn(move || {
                for (arrived, bytes) in receiver {
                    let due = arrived + one_way;
                    let now = Instant::now();
                    if due > now {
                        std::thread::sleep(due - now);
                    }
                    if to.write_all(&bytes).is_err() {
                        break;
                    }
                }
            });
        }
        Ok(())
    });
    Ok(port)
}

/// Keeps both clients reading continuously, as a real client does, so
/// probes are answered promptly, for `duration`.
fn pump_both(near: &mut Client, far: &mut Client, duration: Duration) -> Result<(), String> {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        let step = Instant::now() + Duration::from_micros(500);
        near.pump_until(|_| Instant::now() >= step)?;
        let step = Instant::now() + Duration::from_micros(500);
        far.pump_until(|_| Instant::now() >= step)?;
    }
    Ok(())
}

fn has_any_report(client: &Client, id: &str) -> bool {
    !client.reports_for(id).is_empty()
}

/// Returns `(near filled, far filled)` for one race over the same last lot;
/// `far_first` says which participant sends `HEAD_START` earlier.
fn race(mode: AdmissionMode, far_first: bool) -> Result<(bool, bool), String> {
    let port = start_server(mode)?;
    let mut maker = Client::logon(port, "MAKER", "maker", "bunting-maker-dev")?;
    maker.send(order("901", "sell", 1, Some(100)))?;
    maker.wait_report("901", "0")?;

    let mut near = Client::logon(port, "HUMAN", "participant", "bunting-local-dev")?;
    let proxy = delaying_proxy(port, FAR_ONE_WAY)?;
    let mut far = Client::logon(proxy, "TEAM2", "team2", "bunting-team2-dev")?;
    for client in [&near, &far] {
        client
            .stream
            .set_nodelay(true)
            .and_then(|()| {
                client
                    .stream
                    .set_read_timeout(Some(Duration::from_micros(500)))
            })
            .map_err(|error| error.to_string())?;
    }
    // Let the logon burst and periodic probes measure both paths.
    pump_both(&mut near, &mut far, Duration::from_millis(600))?;

    // Both send within `HEAD_START` of each other in true time.
    if far_first {
        far.send(order("301", "buy", 1, Some(100)))?;
        std::thread::sleep(HEAD_START);
        near.send(order("201", "buy", 1, Some(100)))?;
    } else {
        near.send(order("201", "buy", 1, Some(100)))?;
        std::thread::sleep(HEAD_START);
        far.send(order("301", "buy", 1, Some(100)))?;
    }

    // Collect every report: both orders are acknowledged within D plus the
    // outbound hold; then settle long enough for any fill to arrive too.
    let deadline = Instant::now() + TIMEOUT;
    while !(has_any_report(&near, "201") && has_any_report(&far, "301")) {
        if Instant::now() > deadline {
            return Err("orders were not acknowledged".to_owned());
        }
        pump_both(&mut near, &mut far, Duration::from_millis(5))?;
    }
    pump_both(&mut near, &mut far, Duration::from_millis(3 * D_US / 1_000))?;
    Ok((near.has_report("201", "F"), far.has_report("301", "F")))
}

#[test]
fn equalized_admission_lets_the_earlier_far_client_win() -> Result<(), String> {
    let (near_filled, far_filled) = race(AdmissionMode::Equalized, true)?;
    assert!(
        far_filled && !near_filled,
        "equalized: the far client sent 5 ms earlier and must win (near {near_filled}, far {far_filled})"
    );
    Ok(())
}

#[test]
fn physical_admission_lets_proximity_win() -> Result<(), String> {
    let (near_filled, far_filled) = race(AdmissionMode::Physical, true)?;
    assert!(
        near_filled && !far_filled,
        "physical: the near client's order arrives 25 ms sooner and must win (near {near_filled}, far {far_filled})"
    );
    Ok(())
}

#[test]
fn equalized_admission_does_not_favor_distance_either() -> Result<(), String> {
    let (near_filled, far_filled) = race(AdmissionMode::Equalized, false)?;
    assert!(
        near_filled && !far_filled,
        "equalized: the near client sent 5 ms earlier and must win (near {near_filled}, far {far_filled})"
    );
    Ok(())
}
