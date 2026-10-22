//! Enrolment by invite (ADR 0056): a session with no client certificate
//! that sends one `Enroll` and receives the certificate the venue issued.
//! The venue's chain is trusted only if it ends in the CA certificate the
//! invite pins, so the secret never reaches a venue the invite does not
//! name.

use crate::connection::ClientError;
use bnp_wire::{
    ClientMessage, FrameDecoder, InviteToken, MAX_FRAME_BYTES, ServerMessage, decode_server,
    encode_client,
};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    ClientConfig as TlsClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore,
};
use std::io::{ErrorKind, Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs as _};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What the venue issued for an `Enroll`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Enrolment {
    pub participant_id: u128,
    pub certificate_pem: String,
    pub ca_pem: String,
}

/// Trusts a server chain only through the pinned CA certificate.
#[derive(Debug)]
struct PinnedCa {
    pin: [u8; 32],
    provider: Arc<CryptoProvider>,
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes).into()
}

impl ServerCertVerifier for PinnedCa {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ca = intermediates
            .iter()
            .find(|certificate| sha256(certificate.as_ref()) == self.pin)
            .ok_or_else(|| {
                rustls::Error::General("the venue's CA does not match the invite".to_owned())
            })?;
        let mut roots = RootCertStore::empty();
        roots.add(ca.clone().into_owned())?;
        WebPkiServerVerifier::builder_with_provider(Arc::new(roots), self.provider.clone())
            .build()
            .map_err(|error| rustls::Error::General(error.to_string()))?
            .verify_server_cert(end_entity, &[], server_name, ocsp_response, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The host of `host:port` (brackets removed from an IPv6 literal).
fn host_of(address: &str) -> &str {
    let host = address.rsplit_once(':').map_or(address, |(host, _)| host);
    host.trim_start_matches('[').trim_end_matches(']')
}

/// Enrols with `invite`, sending `csr_pem` (a PEM certificate signing
/// request for a key that stays with the caller).
///
/// # Errors
/// [`ClientError::Refused`] with the venue's reason when it refuses the
/// invite; TLS errors when the venue's CA does not match the pin (before
/// the secret is sent); network and protocol errors otherwise.
pub fn enroll(
    invite: &InviteToken,
    csr_pem: &str,
    timeout: Duration,
) -> Result<Enrolment, ClientError> {
    let deadline = Instant::now() + timeout;
    let address = invite
        .address
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| ClientError::Protocol(format!("cannot resolve {}", invite.address)))?;
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(timeout))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let tls = TlsClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| ClientError::Tls(error.to_string()))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedCa {
            pin: invite.ca_sha256,
            provider,
        }))
        .with_no_client_auth();
    let name = ServerName::try_from(host_of(&invite.address).to_owned())
        .map_err(|error| ClientError::Tls(format!("invalid server name: {error}")))?;
    let mut connection = ClientConnection::new(Arc::new(tls), name)
        .map_err(|error| ClientError::Tls(error.to_string()))?;
    while connection.is_handshaking() {
        connection
            .complete_io(&mut stream)
            .map_err(|error| ClientError::Tls(error.to_string()))?;
    }
    let mut frame = Vec::new();
    encode_client(
        &ClientMessage::Enroll {
            secret: invite.secret.clone(),
            csr_pem: csr_pem.to_owned(),
        },
        &mut frame,
    )
    .map_err(|error| ClientError::Protocol(error.to_string()))?;
    connection.writer().write_all(&frame)?;
    while connection.wants_write() {
        connection
            .write_tls(&mut stream)
            .map_err(|error| ClientError::Tls(error.to_string()))?;
    }
    let mut decoder = FrameDecoder::new(MAX_FRAME_BYTES);
    let mut enrolled = None;
    let mut buffer = [0_u8; 16_384];
    loop {
        if Instant::now() > deadline {
            return Err(ClientError::Protocol("enrolment timed out".to_owned()));
        }
        let count = match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        let mut bytes = &buffer[..count];
        while !bytes.is_empty() {
            connection
                .read_tls(&mut bytes)
                .map_err(|error| ClientError::Tls(error.to_string()))?;
            connection
                .process_new_packets()
                .map_err(|error| ClientError::Tls(error.to_string()))?;
        }
        let mut plain = Vec::new();
        match connection.reader().read_to_end(&mut plain) {
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            Err(error) => return Err(error.into()),
        }
        let frames = decoder
            .push(&plain)
            .map_err(|error| ClientError::Protocol(error.to_string()))?;
        for frame in frames {
            match decode_server(&frame).map_err(|error| ClientError::Protocol(error.to_string()))? {
                ServerMessage::Enrolled {
                    participant_id,
                    certificate_pem,
                    ca_pem,
                } => {
                    enrolled = Some(Enrolment {
                        participant_id,
                        certificate_pem,
                        ca_pem,
                    });
                }
                ServerMessage::Logout { reason } => {
                    return enrolled.ok_or(ClientError::Refused(reason));
                }
                other => {
                    return Err(ClientError::Protocol(format!(
                        "unexpected message type {:#04x} while enrolling",
                        other.msg_type()
                    )));
                }
            }
        }
    }
    enrolled.ok_or(ClientError::Closed(None))
}

/// Whether `ca_pem`'s first certificate is the one `invite` pins.
#[must_use]
pub fn ca_matches(invite: &InviteToken, ca_pem: &str) -> bool {
    rustls_pemfile::certs(&mut ca_pem.as_bytes())
        .next()
        .and_then(Result::ok)
        .is_some_and(|der| sha256(der.as_ref()) == invite.ca_sha256)
}

#[cfg(test)]
mod tests {
    use super::host_of;

    #[test]
    fn the_server_name_is_the_invite_host() {
        assert_eq!(host_of("100.113.213.48:7400"), "100.113.213.48");
        assert_eq!(host_of("venue.example:7400"), "venue.example");
        assert_eq!(host_of("[fd7a::1]:7400"), "fd7a::1");
    }
}
