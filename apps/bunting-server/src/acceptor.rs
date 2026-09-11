use crate::config::{FixConfig, StorageKind, TlsConfig};
use crate::session_host::handle_fix_connection;
use crate::storage::NativeOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_command_transaction::InMemorySnapshotCache;
use std::io::Write;
use std::net::{IpAddr, TcpListener, TcpStream};
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
        StorageKind::Memory | StorageKind::Turso => None,
    };
    loop {
        let (mut stream, _) = match listener.accept() {
            Ok(accepted) => accepted,
            Err(error) => {
                eprintln!("bunting-server: FIX accept failed: {error}");
                continue;
            }
        };
        if let Err(error) = verify_terminated_peer(&stream, &config.tls, "FIX") {
            let _ = stream.write_all(format!("FIX connection rejected: {error}\n").as_bytes());
            eprintln!("bunting-server: FIX connection rejected: {error}");
            continue;
        }
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

fn terminated_peer_allowed(expected: IpAddr, actual: IpAddr) -> bool {
    expected == actual
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
        .parse::<IpAddr>()
        .map_err(|_| format!("invalid trusted proxy for {listener}"))?;
    let actual = stream
        .peer_addr()
        .map_err(|error| format!("cannot inspect {listener} peer: {error}"))?
        .ip();
    if !terminated_peer_allowed(expected, actual) {
        return Err(format!(
            "{listener} peer {actual} is not configured TLS terminator {expected}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ServerConfig;
    use bunting_origin_store::InMemoryOrigin;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn untrusted_terminated_peer_is_rejected_without_listener_failure() -> Result<(), String> {
        let expected = "127.0.0.2"
            .parse::<IpAddr>()
            .map_err(|error| error.to_string())?;
        let actual = "127.0.0.1"
            .parse::<IpAddr>()
            .map_err(|error| error.to_string())?;
        assert!(!terminated_peer_allowed(expected, actual));
        Ok(())
    }

    #[test]
    fn mismatched_terminated_peer_is_connection_scoped() -> Result<(), String> {
        let probe = TcpListener::bind("127.0.0.1:0").map_err(|error| error.to_string())?;
        let address = probe.local_addr().map_err(|error| error.to_string())?;
        drop(probe);

        let mut config = ServerConfig::local_default()
            .fix
            .ok_or_else(|| "local FIX configuration missing".to_owned())?;
        config.bind = address.to_string();
        config.tls = TlsConfig::Terminated {
            trusted_proxy: "127.0.0.2".to_owned(),
            require_mutual_tls: true,
        };
        let origin = Arc::new(NativeOrigin::Memory(InMemoryOrigin::new()));
        let cache = Arc::new(InMemorySnapshotCache::new());
        let writer = Arc::new(AuthoritativeWriter::new(Duration::from_millis(1), 8));
        let _server = thread::spawn(move || {
            let _ = run(&config, StorageKind::Memory, None, &origin, &cache, &writer);
        });

        let connect = || -> Result<TcpStream, String> {
            for _ in 0..50 {
                match TcpStream::connect_timeout(&address, Duration::from_millis(20)) {
                    Ok(stream) => return Ok(stream),
                    Err(_) => thread::sleep(Duration::from_millis(2)),
                }
            }
            Err("FIX acceptor did not remain available".to_owned())
        };

        drop(connect()?);
        thread::sleep(Duration::from_millis(20));
        drop(connect()?);
        Ok(())
    }
}
