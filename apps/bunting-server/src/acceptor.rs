use crate::config::{FixConfig, StorageKind, TlsConfig};
use crate::session_host::handle_fix_connection;
use crate::storage::NativeOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_command_transaction::InMemorySnapshotCache;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) fn run(
    config: &FixConfig,
    storage_kind: StorageKind,
    storage_path: Option<&str>,
    origin: &Arc<NativeOrigin>,
    cache: &Arc<InMemorySnapshotCache>,
    writer: &Arc<AuthoritativeWriter>,
) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .map_err(|error| format!("cannot bind FIX listener {}: {error}", config.bind))?;
    let active_connections = Arc::new(AtomicUsize::new(0));
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
        if active_connections.fetch_add(1, Ordering::AcqRel) >= config.max_connections {
            active_connections.fetch_sub(1, Ordering::AcqRel);
            let rejection = format!(
                "FIX connection rejected: max_connections limit {}\n",
                config.max_connections
            );
            let _ = stream.write_all(rejection.as_bytes());
            continue;
        }
        let config = (*config).clone();
        let origin = origin.clone();
        let cache = cache.clone();
        let writer = writer.clone();
        let session_path = session_path.clone();
        let active_connections = active_connections.clone();
        std::thread::Builder::new()
            .name("bunting-fix-session".to_owned())
            .spawn(move || {
                let _connection = ConnectionGuard(active_connections);
                if let Err(error) = handle_fix_connection(
                    stream,
                    &config,
                    &origin,
                    &cache,
                    &writer,
                    session_path.as_deref(),
                ) {
                    eprintln!("bunting-server: FIX connection closed: {error}");
                }
            })
            .map_err(|error| format!("cannot spawn FIX session: {error}"))?;
    }
}

struct ConnectionGuard(Arc<AtomicUsize>);

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
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
