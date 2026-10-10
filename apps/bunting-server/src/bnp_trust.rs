//! The BNP listener's certificate trust (ADR 0040, ADR 0041): the TLS
//! configuration built from the operator CA and its revocation lists, kept
//! current while the venue runs.
//!
//! A watcher re-reads `bnp.client_ca` and every `bnp.revocation_lists` file
//! once per [`TRUST_POLL`]. When their bytes change it builds a new verifier
//! and TLS configuration and bumps the generation; new handshakes use it at
//! once, and every live session re-verifies its own certificate chain on the
//! next turn of its loop, so a revoked team is logged out without a venue
//! restart. A file that fails to parse leaves the previous trust in force
//! and is reported; trust never silently widens.

use crate::config::BnpConfig;
use rustls::pki_types::{CertificateDer, CertificateRevocationListDer, PrivateKeyDer, UnixTime};
use rustls::server::WebPkiClientVerifier;
use rustls::server::danger::ClientCertVerifier;
use rustls::{RootCertStore, ServerConfig as TlsServerConfig};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// How often the CA and revocation files are checked for changes.
pub(crate) const TRUST_POLL: Duration = Duration::from_secs(1);

/// One built trust: the TLS configuration for new handshakes and the
/// verifier live sessions re-check their chains against.
pub(crate) struct TrustState {
    pub(crate) tls: Arc<TlsServerConfig>,
    verifier: Arc<dyn ClientCertVerifier>,
    /// SHA-256 over the CA and revocation files this state was built from.
    sources: [u8; 32],
}

impl TrustState {
    /// Re-verifies a session's certificate chain (leaf first) against this
    /// trust: the CA and the current revocation lists.
    pub(crate) fn verify(&self, chain: &[CertificateDer<'static>]) -> Result<(), String> {
        let (leaf, intermediates) = chain
            .split_first()
            .ok_or("BNP session has no client certificate")?;
        self.verifier
            .verify_client_cert(leaf, intermediates, UnixTime::now())
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// The current trust and its generation, shared by the listener and every
/// session.
pub(crate) struct Trust {
    current: RwLock<Arc<TrustState>>,
    generation: AtomicU64,
}

impl Trust {
    pub(crate) fn load(config: &BnpConfig) -> Result<Self, String> {
        Ok(Self {
            current: RwLock::new(Arc::new(build(config)?)),
            generation: AtomicU64::new(0),
        })
    }

    pub(crate) fn current(&self) -> Result<Arc<TrustState>, String> {
        self.current
            .read()
            .map(|state| state.clone())
            .map_err(|_| "BNP trust is unavailable".to_owned())
    }

    /// Bumped on every reload; a session re-verifies when it changes.
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Rebuilds the trust when the CA or revocation files changed; returns
    /// whether it did.
    pub(crate) fn reload_if_changed(&self, config: &BnpConfig) -> Result<bool, String> {
        let sources = sources(config)?;
        if self.current()?.sources == sources {
            return Ok(false);
        }
        let state = Arc::new(build(config)?);
        *self
            .current
            .write()
            .map_err(|_| "BNP trust is unavailable".to_owned())? = state;
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }

    /// Starts the watcher thread.
    pub(crate) fn watch(self: &Arc<Self>, config: BnpConfig) -> Result<(), String> {
        let trust = self.clone();
        std::thread::Builder::new()
            .name("bunting-bnp-trust".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(TRUST_POLL);
                    match trust.reload_if_changed(&config) {
                        Ok(true) => eprintln!(
                            "bunting-server: BNP trust reloaded (generation {}); live sessions re-verify",
                            trust.generation()
                        ),
                        Ok(false) => {}
                        Err(error) => eprintln!(
                            "bunting-server: BNP trust not reloaded, previous trust stays in force: {error}"
                        ),
                    }
                }
            })
            .map(|_| ())
            .map_err(|error| format!("cannot spawn BNP trust watcher: {error}"))
    }
}

/// SHA-256 over the client CA and every revocation list file, in order.
fn sources(config: &BnpConfig) -> Result<[u8; 32], String> {
    let mut digest = Sha256::new();
    for path in std::iter::once(&config.client_ca).chain(&config.revocation_lists) {
        let bytes = std::fs::read(path).map_err(|error| format!("cannot read {path}: {error}"))?;
        digest.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(digest.finalize().into())
}

/// TLS 1.3 only, client certificates required and verified against the
/// operator CA and its revocation lists.
fn build(config: &BnpConfig) -> Result<TrustState, String> {
    let sources = sources(config)?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let chain = read_pem(&config.certificate_chain, |reader| {
        rustls_pemfile::certs(reader).collect::<Result<Vec<CertificateDer<'static>>, _>>()
    })?;
    if chain.is_empty() {
        return Err(format!(
            "bnp.certificate_chain {} holds no certificate",
            config.certificate_chain
        ));
    }
    let key: PrivateKeyDer<'static> = read_pem(&config.private_key, |reader| {
        rustls_pemfile::private_key(reader)
    })?
    .ok_or_else(|| {
        format!(
            "bnp.private_key {} holds no private key",
            config.private_key
        )
    })?;
    let mut roots = RootCertStore::empty();
    for certificate in read_pem(&config.client_ca, |reader| {
        rustls_pemfile::certs(reader).collect::<Result<Vec<_>, _>>()
    })? {
        roots
            .add(certificate)
            .map_err(|error| format!("invalid bnp.client_ca certificate: {error}"))?;
    }
    if roots.is_empty() {
        return Err(format!(
            "bnp.client_ca {} holds no certificate",
            config.client_ca
        ));
    }
    let mut revocations: Vec<CertificateRevocationListDer<'static>> = Vec::new();
    for path in &config.revocation_lists {
        let lists = read_pem(path, |reader| {
            rustls_pemfile::crls(reader).collect::<Result<Vec<_>, _>>()
        })?;
        // A listed file with no CRL would drop its revocations: refuse it.
        if lists.is_empty() {
            return Err(format!("bnp.revocation_lists {path} holds no CRL"));
        }
        revocations.extend(lists);
    }
    let verifier = WebPkiClientVerifier::builder_with_provider(Arc::new(roots), provider.clone())
        .with_crls(revocations)
        .build()
        .map_err(|error| format!("invalid BNP client verifier: {error}"))?;
    let tls = TlsServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|error| format!("BNP TLS versions: {error}"))?
        .with_client_cert_verifier(verifier.clone())
        .with_single_cert(chain, key)
        .map_err(|error| format!("invalid BNP server certificate or key: {error}"))?;
    Ok(TrustState {
        tls: Arc::new(tls),
        verifier,
        sources,
    })
}

fn read_pem<T>(
    path: &str,
    read: impl FnOnce(&mut BufReader<File>) -> Result<T, std::io::Error>,
) -> Result<T, String> {
    let file = File::open(path).map_err(|error| format!("cannot open {path}: {error}"))?;
    read(&mut BufReader::new(file)).map_err(|error| format!("invalid PEM in {path}: {error}"))
}
