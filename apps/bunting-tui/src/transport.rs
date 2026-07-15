//! Native TCP and TLS connection establishment.

use crate::config::TransportConfig;
use rustls::{ClientConfig, RootCertStore, pki_types::ServerName};
use std::{fs::File, io, io::BufReader, path::Path, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    net::TcpStream,
};
use tokio_rustls::TlsConnector;

pub trait FixStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T> FixStream for T where T: AsyncRead + AsyncWrite + Send + Unpin {}

pub type BoxedFixStream = Box<dyn FixStream>;

pub async fn connect(endpoint: &str, transport: &TransportConfig) -> io::Result<BoxedFixStream> {
    let tcp = TcpStream::connect(endpoint).await?;
    tcp.set_nodelay(true)?;
    match transport {
        TransportConfig::Tcp => Ok(Box::new(tcp)),
        TransportConfig::Tls {
            server_name,
            ca_file,
        } => {
            let roots = root_store(ca_file.as_deref())?;
            let config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            let name = ServerName::try_from(server_name.clone()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "invalid TLS server_name")
            })?;
            let stream = TlsConnector::from(Arc::new(config))
                .connect(name, tcp)
                .await
                .map_err(|error| io::Error::other(format!("TLS handshake failed: {error}")))?;
            Ok(Box::new(stream))
        }
    }
}

fn root_store(ca_file: Option<&Path>) -> io::Result<RootCertStore> {
    let mut roots = RootCertStore::empty();
    let native = rustls_native_certs::load_native_certs();
    for certificate in native.certs {
        roots
            .add(certificate)
            .map_err(|error| io::Error::other(format!("invalid native CA: {error}")))?;
    }
    if let Some(path) = ca_file {
        let mut reader = BufReader::new(File::open(path)?);
        for certificate in rustls_pemfile::certs(&mut reader) {
            roots
                .add(certificate.map_err(|error| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("invalid PEM CA: {error}"),
                    )
                })?)
                .map_err(|error| io::Error::other(format!("invalid configured CA: {error}")))?;
        }
    }
    if roots.is_empty() {
        return Err(io::Error::other("no TLS trust anchors are available"));
    }
    Ok(roots)
}
