#![allow(clippy::expect_used, clippy::panic)]

use bunting_server::config::ServerConfig;
use std::net::TcpListener;
use std::thread;
use std::time::Duration;

#[tokio::test]
async fn tui_and_native_server_complete_the_competition_profile_over_real_tcp() {
    let probe = TcpListener::bind("127.0.0.1:0").expect("loopback port should be available");
    let endpoint = probe.local_addr().expect("probe should have an address");
    drop(probe);

    let mut config = ServerConfig::local_default();
    config.admin = None;
    config.runtime = None;
    config.fix.as_mut().expect("local FIX config").bind = endpoint.to_string();
    thread::spawn(move || {
        bunting_server::runtime::run(&config).expect("native server should remain available");
    });

    let mut validation = None;
    for _ in 0..40 {
        match bunting_tui::validate_server(&endpoint.to_string(), "bunting-local-dev").await {
            Ok(value) => {
                validation = Some(value);
                break;
            }
            Err(error) if error.contains("Connection refused") => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(error) => panic!("TUI validation failed: {error}"),
        }
    }
    let validation = validation.expect("native server did not bind within the retry bound");
    assert_eq!(validation.verified_role, "participant");
    assert_eq!(
        validation.observed_projections,
        ["market_snapshot", "discovery", "account", "risk"]
    );
    assert_ne!(validation.committed_sequence, "-");
}
