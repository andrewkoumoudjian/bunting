//! Event-driven FIX transport ownership for the native TUI.
//!
//! The I/O task owns the TCP/TLS stream and FIX session. The UI communicates
//! through bounded channels: market-data-only snapshots may be dropped when
//! the UI is saturated, while session transitions, rejects, and execution
//! reports wait for capacity and are never dropped.

use crate::protocol::FixClient;
use simfix_wire::FixMessage;
use std::{io, time::Duration};
use tokio::{
    io::AsyncReadExt,
    sync::mpsc,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};

pub const UI_EVENT_CAPACITY: usize = 256;
pub const OUTBOUND_CAPACITY: usize = 64;
const READ_CAPACITY: usize = 16_384;
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub enum OutboundCmd {
    Send(FixMessage),
    Reconnect,
    Logout,
    ResetSession,
}

pub enum UiEvent {
    Snapshot {
        client: Box<FixClient>,
        recovery_request: bool,
        competition_request: bool,
    },
}

pub struct IoTask {
    pub outbound: mpsc::Sender<OutboundCmd>,
    pub events: mpsc::Receiver<UiEvent>,
    handle: JoinHandle<()>,
}

impl IoTask {
    pub fn spawn(client: FixClient) -> Self {
        let (outbound, commands) = mpsc::channel(OUTBOUND_CAPACITY);
        let (event_sender, events) = mpsc::channel(UI_EVENT_CAPACITY);
        let handle = tokio::spawn(run(client, commands, event_sender));
        Self {
            outbound,
            events,
            handle,
        }
    }

    pub async fn shutdown(self) {
        self.handle.abort();
        let _ = self.handle.await;
    }
}

enum Step {
    Read(io::Result<usize>),
    Command(Option<OutboundCmd>),
    Tick,
}

async fn run(
    mut client: FixClient,
    mut commands: mpsc::Receiver<OutboundCmd>,
    events: mpsc::Sender<UiEvent>,
) {
    let _ = client.reconnect().await;
    if publish(snapshot_event(&mut client), &events, true)
        .await
        .is_err()
    {
        return;
    }

    let mut buffer = Vec::with_capacity(READ_CAPACITY);
    let mut ticker = interval(SESSION_POLL_INTERVAL);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        buffer.clear();
        let before_state = client.connection_state();
        let step = if let Some(stream) = client.stream.as_mut() {
            tokio::select! {
                command = commands.recv() => Step::Command(command),
                read = stream.read_buf(&mut buffer) => Step::Read(read),
                _ = ticker.tick() => Step::Tick,
            }
        } else {
            tokio::select! {
                command = commands.recv() => Step::Command(command),
                _ = ticker.tick() => Step::Tick,
            }
        };

        let critical = match step {
            Step::Read(Ok(0)) => {
                let _ = client.mark_disconnected("peer closed the connection");
                true
            }
            Step::Read(Ok(_)) => match client.receive_bytes(&buffer).await {
                Ok(critical) => critical,
                Err(error) => {
                    let _ = client.mark_disconnected(&format!("FIX receive failed: {error}"));
                    true
                }
            },
            Step::Read(Err(error)) => {
                let _ = client.mark_disconnected(&format!("transport read failed: {error}"));
                true
            }
            Step::Command(Some(OutboundCmd::Send(message))) => {
                if let Err(error) = client.send(message).await {
                    client.status = error.to_string();
                }
                true
            }
            Step::Command(Some(OutboundCmd::Reconnect)) => {
                if let Err(error) = client.reconnect().await {
                    client.status = error.to_string();
                }
                true
            }
            Step::Command(Some(OutboundCmd::Logout)) => {
                if let Err(error) = client.logout().await {
                    client.status = error.to_string();
                }
                true
            }
            Step::Command(Some(OutboundCmd::ResetSession)) => {
                if let Err(error) = client.reset_and_reconnect().await {
                    client.status = error.to_string();
                }
                true
            }
            Step::Command(None) => break,
            Step::Tick => match client.poll_session().await {
                Ok(Some(critical)) => critical,
                Ok(None) => continue,
                Err(error) => {
                    let _ = client.mark_disconnected(&format!("FIX session poll failed: {error}"));
                    true
                }
            },
        };

        let critical = critical || before_state != client.connection_state();
        if publish(snapshot_event(&mut client), &events, critical)
            .await
            .is_err()
        {
            break;
        }
    }
}

fn snapshot_event(client: &mut FixClient) -> Result<UiEvent, ()> {
    let snapshot = client.view_clone().map_err(|_| ())?;
    let established = client.connection_state() == simfix_session::ConnectionState::Established;
    let recovery_request = established && client.take_recovery_request();
    let competition_request = established && client.take_competition_request();
    Ok(UiEvent::Snapshot {
        client: Box::new(snapshot),
        recovery_request,
        competition_request,
    })
}

async fn publish(
    event: Result<UiEvent, ()>,
    events: &mpsc::Sender<UiEvent>,
    critical: bool,
) -> Result<(), ()> {
    let event = event?;
    if critical {
        events.send(event).await.map_err(|_| ())
    } else {
        match events.try_send(event) {
            Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => Ok(()),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(()),
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::config::TerminalConfig;

    #[test]
    fn channel_bounds_match_the_ui_backpressure_contract() {
        assert_eq!(UI_EVENT_CAPACITY, 256);
        assert_eq!(OUTBOUND_CAPACITY, 64);
    }

    #[tokio::test]
    async fn saturated_queue_drops_market_snapshots_but_waits_for_critical_state() {
        let config = TerminalConfig::default();
        let profile = config.profiles.get("local").cloned().unwrap();
        let mut client =
            FixClient::new("local".to_owned(), profile, Some("test".to_owned())).unwrap();
        let (sender, mut receiver) = mpsc::channel(1);

        sender
            .try_send(snapshot_event(&mut client).unwrap())
            .unwrap();
        assert!(
            publish(snapshot_event(&mut client), &sender, false)
                .await
                .is_ok()
        );

        let critical = tokio::spawn({
            let sender = sender.clone();
            let event = snapshot_event(&mut client);
            async move { publish(event, &sender, true).await }
        });
        tokio::task::yield_now().await;
        assert!(!critical.is_finished());
        assert!(receiver.recv().await.is_some());
        assert!(critical.await.unwrap().is_ok());
        assert!(receiver.recv().await.is_some());
    }

    #[test]
    fn established_startup_requests_are_emitted_once_by_the_io_owner() {
        let config = TerminalConfig::default();
        let profile = config.profiles.get("local").cloned().unwrap();
        let mut client =
            FixClient::new("local".to_owned(), profile, Some("test".to_owned())).unwrap();
        let mut snapshot = client.session_snapshot();
        snapshot.state = simfix_session::ConnectionState::Established;
        client.restore_session_for_test(snapshot).unwrap();

        let first = snapshot_event(&mut client).unwrap();
        let second = snapshot_event(&mut client).unwrap();
        assert!(matches!(
            first,
            UiEvent::Snapshot {
                recovery_request: true,
                competition_request: true,
                ..
            }
        ));
        assert!(matches!(
            second,
            UiEvent::Snapshot {
                recovery_request: false,
                competition_request: false,
                ..
            }
        ));
    }
}
