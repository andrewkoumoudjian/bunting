//! BNP enrolment by invite (ADR 0056), end to end on the hosted practice
//! venue `bunting host-init --enrolment` provisions: an invite enrols once,
//! the issued certificate logs in as its participant without a restart,
//! refused invites and a wrong CA pin get nothing, and the enrolled roster
//! survives a restart.

mod support;

use bunting_client::bnp_wire::InviteToken;
use bunting_client::{Client, ClientError};
use bunting_server::config::ServerConfig;
use bunting_trading_state::enrolment::{enroll_with_invite, mint_invite};
use bunting_trading_state::local_venue::{VenueSpec, prepare_with};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn free_port() -> Result<u16, String> {
    TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .map(|address| address.port())
        .map_err(|error| error.to_string())
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("bunting-enrol-{name}-{}", std::process::id()))
}

/// Starts the venue in `directory` on its configured ports.
fn start(directory: &Path) -> Result<(), String> {
    let config = ServerConfig::from_file(&directory.join("server.json"))
        .map_err(|error| error.to_string())?;
    let bnp = config.bnp.as_ref().ok_or("no bnp")?.bind.clone();
    let fix = config.fix.as_ref().ok_or("no fix")?.bind.clone();
    std::thread::spawn(move || bunting_server::runtime::run(&config));
    let port = |bind: &str| {
        bind.rsplit_once(':')
            .and_then(|(_, port)| port.parse().ok())
    };
    support::wait_listening(&[port(&bnp).ok_or("bnp port")?, port(&fix).ok_or("fix port")?])
}

/// Points a copied venue at new ports and its own storage, as a restart
/// of the same venue elsewhere.
fn reconfigure(directory: &Path, bnp_port: u16) -> Result<(), String> {
    let path = directory.join("server.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    config["bnp"]["bind"] = format!("127.0.0.1:{bnp_port}").into();
    config["fix"]["bind"] = format!("127.0.0.1:{}", free_port()?).into();
    std::fs::remove_file(directory.join("origin.lock")).ok();
    std::fs::write(&path, config.to_string()).map_err(|e| e.to_string())
}

#[test]
fn an_invite_enrols_once_and_its_certificate_trades_without_a_restart() -> Result<(), String> {
    let directory = scratch("venue");
    let _ = std::fs::remove_dir_all(&directory);
    let port = free_port()?;
    let spec = VenueSpec::hosted(
        vec!["localhost".to_owned(), "127.0.0.1".to_owned()],
        "127.0.0.1".to_owned(),
        port,
    )
    .with_enrolment();
    prepare_with(&directory, &spec).map_err(|error| error.0)?;
    assert!(
        !directory.join("participant-1.key").exists(),
        "no keys handed out"
    );
    start(&directory)?;

    let invite = mint_invite(&directory, 3, Some(1), None)?;
    let identity = enroll_with_invite(&invite.to_string())?;
    assert_eq!(identity.participant_id, 3);
    // The new certificate is a registered identity at once.
    let client = Client::connect(&identity.client_config()).map_err(|e| e.to_string())?;
    assert_eq!(client.welcome().participant_id, 3);
    client.close();

    // A spent invite, a wrong secret and a participant enrolled twice get
    // the same refusal.
    for text in [
        invite.to_string(),
        InviteToken {
            secret: "0".repeat(32),
            ..invite.clone()
        }
        .to_string(),
        mint_invite(&directory, 3, None, None)?.to_string(),
    ] {
        let refused = enroll_with_invite(&text).err().ok_or("refused")?;
        assert!(refused.contains("invite refused"), "{refused}");
    }
    // A pin that is not the venue's CA fails before the secret is sent.
    let fresh = mint_invite(&directory, 4, None, None)?;
    let pinned_wrong = InviteToken {
        ca_sha256: [7; 32],
        ..fresh.clone()
    };
    let error = enroll_with_invite(&pinned_wrong.to_string())
        .err()
        .ok_or("wrong pin refused")?;
    assert!(error.contains("CA does not match"), "{error}");
    // The invite was not spent by the attempt that never reached the venue.
    assert_eq!(enroll_with_invite(&fresh.to_string())?.participant_id, 4);

    // A client without a certificate may only enrol.
    let anonymous = bunting_client::enroll(
        &mint_invite(&directory, 5, None, None)?,
        "not a certificate request",
        Duration::from_secs(5),
    );
    assert!(
        matches!(anonymous, Err(ClientError::Refused(ref reason)) if reason == "invite refused"),
        "{anonymous:?}"
    );

    // The enrolled roster outlives a restart.
    let copy = scratch("restart");
    let _ = std::fs::remove_dir_all(&copy);
    std::fs::create_dir_all(&copy).map_err(|e| e.to_string())?;
    for name in [
        "server.json",
        "scenario.json",
        "venue.pem",
        "venue.key",
        "ca.pem",
        "ca.key",
        "invites.json",
        "enrolled.json",
    ] {
        std::fs::copy(directory.join(name), copy.join(name)).map_err(|e| format!("{name}: {e}"))?;
    }
    let moved = free_port()?;
    reconfigure(&copy, moved)?;
    start(&copy)?;
    let mut config = identity.client_config();
    config.address = format!("127.0.0.1:{moved}");
    let again = Client::connect(&config).map_err(|e| e.to_string())?;
    assert_eq!(again.welcome().participant_id, 3);
    again.close();
    let _ = std::fs::remove_dir_all(&directory);
    let _ = std::fs::remove_dir_all(&copy);
    Ok(())
}
