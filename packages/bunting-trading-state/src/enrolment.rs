//! Joining a hosted venue by invite (ADR 0056), for the desk, the terminal
//! and `bunting-trader`: create a key here, send only its certificate
//! request with the invite, and keep the certificate the venue issues. The
//! private key never leaves this machine. Also the operator's side of an
//! invite: minting one for a venue directory written by `bunting host-init
//! --enrolment`.

use bunting_client::bnp_wire::InviteToken;
use bunting_client::{ClientConfig, ca_matches, certificate_fingerprint, enroll};
use rcgen::{CertificateParams, DnType, KeyPair};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How long one enrolment exchange may take.
const ENROL_TIMEOUT: Duration = Duration::from_secs(15);

/// A participant identity issued by a venue: where it trades and the
/// certificate and key that prove who it is.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub address: String,
    pub server_name: String,
    pub participant_id: u128,
    pub ca_pem: String,
    pub certificate_pem: String,
    pub private_key_pem: String,
}

/// 128 random bits as 32 lowercase hex digits.
///
/// # Errors
/// Returns why the system random source failed.
pub fn new_secret() -> Result<String, String> {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0_u8; 16];
    ring::rand::SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "the system random source failed".to_owned())?;
    Ok(hex(&bytes))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// Lowercase hex SHA-256 of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref())
}

/// Enrols with a pasted invite: generates a key, sends its certificate
/// request, and returns the identity the venue issued.
///
/// # Errors
/// Returns the venue's refusal or what failed, as a sentence for the
/// trader.
pub fn enroll_with_invite(invite: &str) -> Result<Identity, String> {
    let invite = InviteToken::parse(invite)?;
    let key = KeyPair::generate().map_err(|error| format!("cannot create a key: {error}"))?;
    let mut params = CertificateParams::new(Vec::<String>::new())
        .map_err(|error| format!("cannot build a certificate request: {error}"))?;
    params
        .distinguished_name
        .push(DnType::CommonName, "Bunting participant");
    let csr = params
        .serialize_request(&key)
        .and_then(|request| request.pem())
        .map_err(|error| format!("cannot build a certificate request: {error}"))?;
    let enrolment = enroll(&invite, &csr, ENROL_TIMEOUT).map_err(|error| match error {
        bunting_client::ClientError::Refused(reason) => format!("the venue refused: {reason}"),
        other => format!("cannot enrol at {}: {other}", invite.address),
    })?;
    if !ca_matches(&invite, &enrolment.ca_pem) {
        return Err("the venue's CA does not match the invite".to_owned());
    }
    let host = invite
        .address
        .rsplit_once(':')
        .map_or(invite.address.as_str(), |(host, _)| host)
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    Ok(Identity {
        address: invite.address,
        server_name: host,
        participant_id: enrolment.participant_id,
        ca_pem: enrolment.ca_pem,
        certificate_pem: enrolment.certificate_pem,
        private_key_pem: key.serialize_pem(),
    })
}

impl Identity {
    /// How this identity connects.
    #[must_use]
    pub fn client_config(&self) -> ClientConfig {
        ClientConfig {
            address: self.address.clone(),
            server_name: self.server_name.clone(),
            ca_pem: self.ca_pem.as_bytes().to_vec(),
            certificate_pem: self.certificate_pem.as_bytes().to_vec(),
            private_key_pem: self.private_key_pem.as_bytes().to_vec(),
            resume_after: None,
            client_name: concat!("bunting-trading-state/", env!("CARGO_PKG_VERSION")).to_owned(),
            connect_timeout: Duration::from_secs(10),
        }
    }

    /// Writes `ca.pem`, `participant.pem`, `participant.key` and
    /// `identity.json` into `directory`, owner-only.
    ///
    /// # Errors
    /// Returns the I/O error.
    pub fn save(&self, directory: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(directory)?;
        let write = |name: &str, contents: &str| {
            crate::local_venue::write_private(&directory.join(name), contents)
        };
        write("ca.pem", &self.ca_pem)?;
        write("participant.pem", &self.certificate_pem)?;
        write("participant.key", &self.private_key_pem)?;
        write(
            "identity.json",
            &json!({
                "address": self.address,
                "server_name": self.server_name,
                "participant_id": self.participant_id.to_string(),
            })
            .to_string(),
        )
    }

    /// Reads what [`Self::save`] wrote, if it is all there.
    #[must_use]
    pub fn load(directory: &Path) -> Option<Self> {
        let read = |name: &str| std::fs::read_to_string(directory.join(name)).ok();
        let info: Value = serde_json::from_str(&read("identity.json")?).ok()?;
        Some(Self {
            address: info["address"].as_str()?.to_owned(),
            server_name: info["server_name"].as_str()?.to_owned(),
            participant_id: info["participant_id"].as_str()?.parse().ok()?,
            ca_pem: read("ca.pem")?,
            certificate_pem: read("participant.pem")?,
            private_key_pem: read("participant.key")?,
        })
    }

    /// The certificate's fingerprint, which names this identity.
    ///
    /// # Errors
    /// Returns an error for an unreadable certificate.
    pub fn fingerprint(&self) -> Result<String, String> {
        certificate_fingerprint(self.certificate_pem.as_bytes()).map_err(|error| error.to_string())
    }
}

/// Mints an invite for `participant_id` on the enrolment venue in
/// `venue_dir` (written by `bunting host-init --enrolment`), valid for
/// `hours` (none: until used). `address` overrides the BNP address in the
/// invite, for a venue bound to every interface.
///
/// # Errors
/// Returns what is missing or cannot be written.
pub fn mint_invite(
    venue_dir: &Path,
    participant_id: u128,
    hours: Option<u64>,
    address: Option<String>,
) -> Result<InviteToken, String> {
    let read = |name: &str| {
        std::fs::read_to_string(venue_dir.join(name))
            .map_err(|error| format!("cannot read {name}: {error}"))
    };
    let config: Value =
        serde_json::from_str(&read("server.json")?).map_err(|error| error.to_string())?;
    let bnp = &config["bnp"];
    let invites_path = bnp["enrolment"]["invites"]
        .as_str()
        .map(|path| resolve(venue_dir, path))
        .ok_or("this venue has no bnp.enrolment; provision it with host-init --enrolment")?;
    let address = match address {
        Some(address) => address,
        None => bnp["bind"]
            .as_str()
            .ok_or("bnp.bind is missing")?
            .to_owned(),
    };
    if address.starts_with("0.0.0.0:") || address.starts_with("[::]:") {
        return Err("the venue listens on every interface; give --address host:port".to_owned());
    }
    let ca_pem = read(bnp["client_ca"].as_str().unwrap_or("ca.pem"))?;
    let pin = certificate_fingerprint(ca_pem.as_bytes()).map_err(|error| error.to_string())?;
    let mut ca_sha256 = [0_u8; 32];
    for (index, byte) in ca_sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&pin[index * 2..index * 2 + 2], 16)
            .map_err(|_| "the CA fingerprint is not hex".to_owned())?;
    }
    let secret = new_secret()?;
    let mut invites: Value = std::fs::read_to_string(&invites_path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({"version": 1, "invites": []}));
    let expires_unix = hours.map(|hours| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |now| now.as_secs())
            .saturating_add(hours.saturating_mul(3_600))
    });
    invites["invites"]
        .as_array_mut()
        .ok_or("the invites file is malformed")?
        .push(json!({
            "secret_sha256": sha256_hex(secret.as_bytes()),
            "participant_id": participant_id,
            "expires_unix": expires_unix,
            "used": false,
        }));
    let partial = invites_path.with_extension("json.partial");
    crate::local_venue::write_private(&partial, &invites.to_string())
        .and_then(|()| std::fs::rename(&partial, &invites_path))
        .map_err(|error| format!("cannot write {}: {error}", invites_path.display()))?;
    Ok(InviteToken {
        address,
        ca_sha256,
        secret,
    })
}

fn resolve(base: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_relative() {
        base.join(path)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_fresh_32_hex_digits() -> Result<(), String> {
        let (one, two) = (new_secret()?, new_secret()?);
        assert_eq!(one.len(), 32);
        assert!(one.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(one, two);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        Ok(())
    }

    #[test]
    fn an_identity_survives_save_and_load() -> std::io::Result<()> {
        let directory =
            std::env::temp_dir().join(format!("bunting-identity-{}", std::process::id()));
        let identity = Identity {
            address: "100.64.0.9:7400".to_owned(),
            server_name: "100.64.0.9".to_owned(),
            participant_id: 3,
            ca_pem: "ca".to_owned(),
            certificate_pem: "cert".to_owned(),
            private_key_pem: "key".to_owned(),
        };
        identity.save(&directory)?;
        assert_eq!(Identity::load(&directory), Some(identity));
        std::fs::remove_dir_all(&directory)
    }
}
