//! The native BNP connection: TCP, TLS 1.3 with a client certificate,
//! `Hello`/`Welcome`, and a reader thread.

use bnp_wire::{
    ClientMessage, FrameDecoder, MAX_FRAME_BYTES, PROTOCOL_VERSION, ServerMessage, Welcome,
    decode_server, encode_client,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use rustls::{ClientConfig as TlsClientConfig, ClientConnection, RootCertStore};
use std::io::{BufReader, ErrorKind, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Server messages queued for the application before the reader stops
/// reading the socket.
const EVENT_CAPACITY: usize = 65_536;
const READ_BYTES: usize = 16_384;

#[derive(Debug)]
pub enum ClientError {
    Io(std::io::Error),
    Tls(String),
    Certificate(String),
    /// The venue refused the session; its reason.
    Refused(String),
    Protocol(String),
    /// The connection is closed; the reason, when there was one.
    Closed(Option<String>),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "network error: {error}"),
            Self::Tls(error) => write!(formatter, "TLS error: {error}"),
            Self::Certificate(error) => write!(formatter, "certificate error: {error}"),
            Self::Refused(reason) => write!(formatter, "venue refused the session: {reason}"),
            Self::Protocol(error) => write!(formatter, "protocol error: {error}"),
            Self::Closed(Some(reason)) => write!(formatter, "connection closed: {reason}"),
            Self::Closed(None) => formatter.write_str("connection closed"),
        }
    }
}

impl std::error::Error for ClientError {}

impl From<std::io::Error> for ClientError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Where and as whom to connect.
#[derive(Clone)]
pub struct ClientConfig {
    /// `host:port` of the venue's BNP listener.
    pub address: String,
    /// The name the venue's certificate is issued for.
    pub server_name: String,
    /// PEM certificates trusted to issue the venue's certificate.
    pub ca_pem: Vec<u8>,
    /// PEM certificate chain of this participant, leaf first.
    pub certificate_pem: Vec<u8>,
    /// PEM private key of the participant certificate.
    pub private_key_pem: Vec<u8>,
    /// The last private `sequence` this client processed, to resume after.
    pub resume_after: Option<u64>,
    pub client_name: String,
    /// Bound on TCP connect, the TLS handshake and `Welcome` together.
    pub connect_timeout: Duration,
}

impl ClientConfig {
    /// Reads the CA, certificate and key from PEM files.
    ///
    /// # Errors
    /// Returns an I/O error when a file cannot be read.
    pub fn from_files(
        address: impl Into<String>,
        server_name: impl Into<String>,
        ca: &Path,
        certificate: &Path,
        private_key: &Path,
    ) -> Result<Self, ClientError> {
        Ok(Self {
            address: address.into(),
            server_name: server_name.into(),
            ca_pem: std::fs::read(ca)?,
            certificate_pem: std::fs::read(certificate)?,
            private_key_pem: std::fs::read(private_key)?,
            resume_after: None,
            client_name: concat!("bunting-client/", env!("CARGO_PKG_VERSION")).to_owned(),
            connect_timeout: Duration::from_secs(10),
        })
    }

    fn tls(&self) -> Result<TlsClientConfig, ClientError> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut roots = RootCertStore::empty();
        for certificate in certificates(&self.ca_pem)? {
            roots
                .add(certificate)
                .map_err(|error| ClientError::Certificate(format!("invalid CA: {error}")))?;
        }
        if roots.is_empty() {
            return Err(ClientError::Certificate("no CA certificate".to_owned()));
        }
        let chain = certificates(&self.certificate_pem)?;
        if chain.is_empty() {
            return Err(ClientError::Certificate("no client certificate".to_owned()));
        }
        let key: PrivateKeyDer<'static> =
            rustls_pemfile::private_key(&mut BufReader::new(self.private_key_pem.as_slice()))
                .map_err(|error| ClientError::Certificate(format!("invalid key: {error}")))?
                .ok_or_else(|| ClientError::Certificate("no private key".to_owned()))?;
        TlsClientConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(|error| ClientError::Tls(error.to_string()))?
            .with_root_certificates(roots)
            .with_client_auth_cert(chain, key)
            .map_err(|error| ClientError::Certificate(error.to_string()))
    }
}

fn certificates(pem: &[u8]) -> Result<Vec<CertificateDer<'static>>, ClientError> {
    rustls_pemfile::certs(&mut BufReader::new(pem))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ClientError::Certificate(format!("invalid PEM: {error}")))
}

/// Lowercase hex SHA-256 of the first certificate in `pem`: the value an
/// operator registers in the venue's BNP roster.
///
/// # Errors
/// Returns an error when `pem` holds no certificate.
pub fn certificate_fingerprint(pem: &[u8]) -> Result<String, ClientError> {
    let leaf = certificates(pem)?
        .into_iter()
        .next()
        .ok_or_else(|| ClientError::Certificate("no certificate".to_owned()))?;
    let digest = sha256(leaf.as_ref());
    Ok(digest.iter().fold(String::new(), |mut text, byte| {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
        text
    }))
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).to_vec()
}

/// TLS state and the socket's write half, shared by the reader thread
/// (probe replies, heartbeats) and the application (requests).
struct Writer {
    connection: ClientConnection,
    stream: TcpStream,
    last_sent: Instant,
}

impl Writer {
    fn send(&mut self, message: &ClientMessage) -> Result<(), ClientError> {
        let mut bytes = Vec::new();
        encode_client(message, &mut bytes)
            .map_err(|error| ClientError::Protocol(error.to_string()))?;
        self.connection.writer().write_all(&bytes)?;
        self.flush()?;
        self.last_sent = Instant::now();
        Ok(())
    }

    fn flush(&mut self) -> Result<(), ClientError> {
        while self.connection.wants_write() {
            self.connection.write_tls(&mut self.stream)?;
        }
        Ok(())
    }

    /// Decrypts received bytes into frames; `None` once the venue closed TLS.
    fn receive(
        &mut self,
        mut bytes: &[u8],
        decoder: &mut FrameDecoder,
    ) -> Result<Option<Vec<Vec<u8>>>, ClientError> {
        while !bytes.is_empty() {
            self.connection.read_tls(&mut bytes)?;
            if let Err(error) = self.connection.process_new_packets() {
                let _ = self.flush();
                return Err(ClientError::Tls(error.to_string()));
            }
        }
        let mut frames = Vec::new();
        let mut buffer = [0_u8; READ_BYTES];
        let closed = loop {
            match self.connection.reader().read(&mut buffer) {
                Ok(0) => break true,
                Ok(count) => frames.extend(
                    decoder
                        .push(&buffer[..count])
                        .map_err(|error| ClientError::Protocol(error.to_string()))?,
                ),
                Err(error) if error.kind() == ErrorKind::WouldBlock => break false,
                Err(error) => return Err(error.into()),
            }
        };
        self.flush()?;
        Ok((!closed || !frames.is_empty()).then_some(frames))
    }
}

/// A live BNP session.
pub struct Client {
    writer: Arc<Mutex<Writer>>,
    events: Receiver<Result<ServerMessage, ClientError>>,
    welcome: Welcome,
    cursor: Arc<AtomicU64>,
    shutdown: TcpStream,
}

impl Client {
    /// Connects, authenticates with the configured certificate and waits
    /// for `Welcome`.
    ///
    /// # Errors
    /// Returns [`ClientError::Refused`] when the venue answers `Hello` with
    /// `Logout`, and other errors for network, TLS or protocol failures.
    pub fn connect(config: &ClientConfig) -> Result<Self, ClientError> {
        let deadline = Instant::now() + config.connect_timeout;
        let address =
            config.address.to_socket_addrs()?.next().ok_or_else(|| {
                ClientError::Protocol(format!("cannot resolve {}", config.address))
            })?;
        let mut stream = TcpStream::connect_timeout(&address, config.connect_timeout)?;
        // A good client: never hold small requests for a delayed ACK.
        stream.set_nodelay(true)?;
        stream.set_read_timeout(Some(config.connect_timeout))?;
        let name = ServerName::try_from(config.server_name.clone())
            .map_err(|error| ClientError::Tls(format!("invalid server name: {error}")))?;
        let mut connection = ClientConnection::new(Arc::new(config.tls()?), name)
            .map_err(|error| ClientError::Tls(error.to_string()))?;
        while connection.is_handshaking() {
            connection
                .complete_io(&mut stream)
                .map_err(|error| ClientError::Tls(error.to_string()))?;
        }
        let mut writer = Writer {
            connection,
            stream: stream.try_clone()?,
            last_sent: Instant::now(),
        };
        writer.send(&ClientMessage::Hello {
            version: PROTOCOL_VERSION,
            resume_after: config.resume_after,
            client_name: config.client_name.clone(),
        })?;
        let mut decoder = FrameDecoder::new(MAX_FRAME_BYTES);
        let mut buffer = [0_u8; READ_BYTES];
        let mut frames = std::collections::VecDeque::<Vec<u8>>::new();
        let welcome = loop {
            if let Some(frame) = frames.pop_front() {
                match decode_server(&frame)
                    .map_err(|error| ClientError::Protocol(error.to_string()))?
                {
                    ServerMessage::Welcome(welcome) => break welcome,
                    ServerMessage::Logout { reason } => return Err(ClientError::Refused(reason)),
                    other => {
                        return Err(ClientError::Protocol(format!(
                            "expected Welcome, received message type {:#04x}",
                            other.msg_type()
                        )));
                    }
                }
            }
            if Instant::now() > deadline {
                return Err(ClientError::Protocol("Welcome timed out".to_owned()));
            }
            let count = match stream.read(&mut buffer) {
                Ok(0) => return Err(ClientError::Closed(None)),
                Ok(count) => count,
                Err(error) => {
                    return Err(match error.kind() {
                        // A refused certificate surfaces as a TLS alert or
                        // a reset right after the handshake.
                        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted => {
                            ClientError::Refused(error.to_string())
                        }
                        _ => error.into(),
                    });
                }
            };
            match writer.receive(&buffer[..count], &mut decoder) {
                Ok(Some(received)) => frames.extend(received),
                Ok(None) => return Err(ClientError::Closed(None)),
                Err(ClientError::Tls(alert)) => return Err(ClientError::Refused(alert)),
                Err(error) => return Err(error),
            }
        };
        let heartbeat = Duration::from_millis(u64::from(welcome.heartbeat_ms.max(1)));
        // Wake at least twice per heartbeat interval to send our own.
        stream.set_read_timeout(Some(heartbeat / 2))?;
        let shutdown = stream.try_clone()?;
        let writer = Arc::new(Mutex::new(writer));
        let cursor = Arc::new(AtomicU64::new(config.resume_after.unwrap_or(0)));
        let (sender, events) = sync_channel(EVENT_CAPACITY);
        let reader = Reader {
            stream,
            writer: writer.clone(),
            decoder,
            sender,
            heartbeat,
        };
        // Frames that arrived with the Welcome (probes, replay) go first.
        let pending: Vec<Vec<u8>> = frames.into_iter().collect();
        std::thread::Builder::new()
            .name("bunting-client-reader".to_owned())
            .spawn(move || reader.run(pending))?;
        Ok(Self {
            writer,
            events,
            welcome,
            cursor,
            shutdown,
        })
    }

    /// The venue's terms for this session.
    #[must_use]
    pub const fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// Sends one request, written to the socket before this returns.
    ///
    /// # Errors
    /// Returns an error when the connection is closed or the message
    /// cannot be encoded.
    pub fn send(&self, message: &ClientMessage) -> Result<(), ClientError> {
        self.writer
            .lock()
            .map_err(|_| ClientError::Closed(Some("writer poisoned".to_owned())))?
            .send(message)
    }

    /// The next server message, waiting at most `timeout`. Probes and
    /// heartbeats are handled by the reader and never returned.
    ///
    /// # Errors
    /// Returns [`ClientError::Closed`] (with the venue's `Logout` reason
    /// when it sent one) once the session has ended.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<ServerMessage>, ClientError> {
        match self.events.recv_timeout(timeout) {
            Ok(Ok(message)) => {
                if let Some(stamp) = message.stamp() {
                    self.cursor.fetch_max(stamp.sequence, Ordering::AcqRel);
                }
                Ok(Some(message))
            }
            Ok(Err(error)) => Err(error),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(ClientError::Closed(None)),
        }
    }

    /// The highest private `sequence` returned by [`Self::recv_timeout`]:
    /// pass it as `resume_after` when reconnecting.
    #[must_use]
    pub fn cursor(&self) -> u64 {
        self.cursor.load(Ordering::Acquire)
    }

    /// Sends `Logout` and closes the connection.
    pub fn close(self) {
        let _ = self.send(&ClientMessage::Logout {
            reason: "client closed".to_owned(),
        });
        let _ = self.shutdown.shutdown(std::net::Shutdown::Both);
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.shutdown.shutdown(std::net::Shutdown::Both);
    }
}

/// The socket's read half, drained continuously on its own thread.
struct Reader {
    stream: TcpStream,
    writer: Arc<Mutex<Writer>>,
    decoder: FrameDecoder,
    sender: SyncSender<Result<ServerMessage, ClientError>>,
    heartbeat: Duration,
}

impl Reader {
    fn run(mut self, pending: Vec<Vec<u8>>) {
        let result = self.serve(pending);
        let _ = self.sender.send(Err(match result {
            Ok(()) => ClientError::Closed(None),
            Err(error) => error,
        }));
    }

    fn serve(&mut self, pending: Vec<Vec<u8>>) -> Result<(), ClientError> {
        if !self.dispatch(pending)? {
            return Ok(());
        }
        let mut buffer = [0_u8; READ_BYTES];
        loop {
            let count = match self.stream.read(&mut buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => Some(count),
                Err(error)
                    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                {
                    None
                }
                Err(error) => return Err(error.into()),
            };
            let frames = {
                let mut writer = self
                    .writer
                    .lock()
                    .map_err(|_| ClientError::Closed(Some("writer poisoned".to_owned())))?;
                let frames = match count {
                    Some(count) => match writer.receive(&buffer[..count], &mut self.decoder)? {
                        Some(frames) => frames,
                        None => return Ok(()),
                    },
                    None => Vec::new(),
                };
                if writer.last_sent.elapsed() >= self.heartbeat {
                    writer.send(&ClientMessage::Heartbeat)?;
                }
                frames
            };
            if !self.dispatch(frames)? {
                return Ok(());
            }
        }
    }

    /// Answers probes at once and queues everything else; `false` after a
    /// `Logout`.
    fn dispatch(&mut self, frames: Vec<Vec<u8>>) -> Result<bool, ClientError> {
        for frame in frames {
            let message =
                decode_server(&frame).map_err(|error| ClientError::Protocol(error.to_string()))?;
            match message {
                ServerMessage::Probe { probe_id } => self
                    .writer
                    .lock()
                    .map_err(|_| ClientError::Closed(Some("writer poisoned".to_owned())))?
                    .send(&ClientMessage::ProbeReply { probe_id })?,
                ServerMessage::Heartbeat => {}
                ServerMessage::Logout { reason } => {
                    let _ = self.sender.send(Err(ClientError::Closed(Some(reason))));
                    return Ok(false);
                }
                other => {
                    if self.sender.send(Ok(other)).is_err() {
                        // The application dropped the client.
                        return Ok(false);
                    }
                }
            }
        }
        Ok(true)
    }
}
