use crate::admission::AdmissionService;
use crate::config::{FixConfig, StorageKind, TlsConfig};
use crate::distributor::PublishingOrigin;
use crate::session_host::handle_fix_connection;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub(crate) fn run(
    config: &FixConfig,
    storage_kind: StorageKind,
    storage_path: Option<&str>,
    origin: &Arc<PublishingOrigin>,
    admission: &Arc<AdmissionService>,
) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind FIX listener {}: {error}", config.bind))?;
    let slots = Arc::new(ConnectionSlots::new(config.max_connections));
    let session_path = match storage_kind {
        StorageKind::File => {
            storage_path.map(|path| PathBuf::from(path).with_extension("fix-session.json"))
        }
        StorageKind::Memory => None,
    };
    loop {
        let (mut stream, _) = listener
            .accept()
            .map_err(|error| format!("FIX accept failed: {error}"))?;
        verify_terminated_peer(&stream, &config.tls, "FIX")?;
        // Never let the venue's own Nagle buffering delay a participant's
        // reports (ADR 0034 §4).
        stream
            .set_nodelay(true)
            .map_err(|error| format!("cannot disable Nagle on FIX socket: {error}"))?;
        if !slots.try_queue() {
            reject(&mut stream, config.max_connections);
            continue;
        }
        let config = (*config).clone();
        let origin = origin.clone();
        let admission = admission.clone();
        let session_path = session_path.clone();
        let slots = slots.clone();
        std::thread::Builder::new()
            .name("bunting-fix-session".to_owned())
            .spawn(move || {
                // A reconnecting client may arrive while its previous
                // connection is still closing; wait briefly for that slot.
                let Some(_slot) = slots.acquire(SLOT_WAIT) else {
                    reject(&mut stream, config.max_connections);
                    return;
                };
                if let Err(error) = handle_fix_connection(
                    stream,
                    &config,
                    &origin,
                    &admission,
                    session_path.as_deref(),
                ) {
                    eprintln!("bunting-server: FIX connection closed: {error}");
                }
            })
            .map_err(|error| format!("cannot spawn FIX session: {error}"))?;
    }
}

/// How long an accepted connection waits for a free session slot.
const SLOT_WAIT: Duration = Duration::from_secs(5);

fn reject(stream: &mut TcpStream, limit: usize) {
    let rejection = format!("FIX connection rejected: max_connections limit {limit}\n");
    let _ = stream.write_all(rejection.as_bytes());
}

/// At most `max` live sessions, plus at most `max` accepted connections
/// waiting for one of them to close.
struct ConnectionSlots {
    max: usize,
    /// `(active, waiting)`.
    state: Mutex<(usize, usize)>,
    freed: Condvar,
}

impl ConnectionSlots {
    const fn new(max: usize) -> Self {
        Self {
            max,
            state: Mutex::new((0, 0)),
            freed: Condvar::new(),
        }
    }

    /// Admits a connection to the waiting area, or refuses at the hard cap.
    fn try_queue(&self) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if state.0 + state.1 >= self.max.saturating_mul(2) {
            return false;
        }
        state.1 += 1;
        true
    }

    /// Moves a queued connection into a session slot within `wait`.
    fn acquire(&self, wait: Duration) -> Option<SlotGuard<'_>> {
        let deadline = Instant::now() + wait;
        let mut state = self.state.lock().ok()?;
        while state.0 >= self.max {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.1 -= 1;
                return None;
            }
            state = self.freed.wait_timeout(state, remaining).ok()?.0;
        }
        state.1 -= 1;
        state.0 += 1;
        Some(SlotGuard(self))
    }
}

struct SlotGuard<'a>(&'a ConnectionSlots);

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.state.lock() {
            state.0 -= 1;
        }
        self.0.freed.notify_all();
    }
}

fn verify_terminated_peer(
    stream: &TcpStream,
    tls: &TlsConfig,
    listener: &str,
) -> Result<(), String> {
    let TlsConfig::Terminated { trusted_proxy, .. } = tls else {
        return Ok(());
    };
    let expected = trusted_proxy
        .parse::<std::net::IpAddr>()
        .map_err(|_| format!("invalid trusted proxy for {listener}"))?;
    let actual = stream
        .peer_addr()
        .map_err(|error| format!("cannot inspect {listener} peer: {error}"))?
        .ip();
    if actual != expected {
        return Err(format!(
            "{listener} peer {actual} is not configured TLS terminator {expected}"
        ));
    }
    Ok(())
}
