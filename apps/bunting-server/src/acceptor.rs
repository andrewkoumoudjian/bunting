use crate::config::{FixConfig, StorageKind, TlsConfig};
use crate::session_host::handle_fix_connection;
use crate::storage::NativeOrigin;
use crate::writer::AuthoritativeWriter;
use bunting_command_transaction::InMemorySnapshotCache;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

pub(crate) async fn run(
    config: FixConfig,
    storage_kind: StorageKind,
    storage_path: Option<String>,
    origin: Arc<NativeOrigin>,
    cache: Arc<InMemorySnapshotCache>,
    writer: Arc<AuthoritativeWriter>,
) -> Result<(), String> {
    let listener = TcpListener::bind(&config.bind)
        .await
        .map_err(|error| format!("cannot bind FIX listener {}: {error}", config.bind))?;
    let capacity = Arc::new(Semaphore::new(config.max_connections));
    let session_path = match storage_kind {
        StorageKind::File => storage_path
            .as_deref()
            .map(|path| PathBuf::from(path).with_extension("fix-session.json")),
        StorageKind::Memory => None,
    };
    loop {
        let (mut stream, _) = listener
            .accept()
            .await
            .map_err(|error| format!("FIX accept failed: {error}"))?;
        verify_terminated_peer(&stream, &config.tls, "FIX")?;
        let Ok(permit) = capacity.clone().try_acquire_owned() else {
            let rejection = format!(
                "FIX connection rejected: max_connections limit {}\n",
                config.max_connections
            );
            let _ = stream.write_all(rejection.as_bytes()).await;
            continue;
        };
        let standard = stream
            .into_std()
            .map_err(|error| format!("cannot adopt FIX socket: {error}"))?;
        standard
            .set_nonblocking(false)
            .map_err(|error| format!("cannot configure FIX socket: {error}"))?;
        let config = config.clone();
        let origin = origin.clone();
        let cache = cache.clone();
        let writer = writer.clone();
        let session_path = session_path.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            if let Err(error) = handle_fix_connection(
                standard,
                &config,
                &origin,
                &cache,
                &writer,
                session_path.as_deref(),
            ) {
                eprintln!("bunting-server: FIX connection closed: {error}");
            }
        });
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
