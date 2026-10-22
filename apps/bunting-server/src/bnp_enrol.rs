//! BNP enrolment by invite (ADR 0056): a session without a client
//! certificate sends one `Enroll`; an unused invite and a certificate
//! signing request whose signature verifies earn a client certificate from
//! the venue's client CA. The new fingerprint joins the roster at once and
//! is kept in the enrolled file, so it outlives a restart.

use crate::config::{BnpRosterEntry, EnrolmentConfig};
use bunting_api_contract::ActorRole;
use rcgen::{
    CertificateParams, CertificateSigningRequestParams, DnType, ExtendedKeyUsagePurpose, IsCa,
    Issuer, KeyPair, KeyUsagePurpose,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Fingerprint (SHA-256 of the certificate DER) to roster entry.
pub(crate) type Roster = BTreeMap<[u8; 32], BnpRosterEntry>;

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct InvitesFile {
    version: u32,
    invites: Vec<Invite>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    secret_sha256: String,
    participant_id: u128,
    #[serde(default)]
    expires_unix: Option<u64>,
    used: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EnrolledFile {
    version: u32,
    roster: Vec<BnpRosterEntry>,
}

/// The venue's enrolment authority.
pub(crate) struct Enrolment {
    config: EnrolmentConfig,
    issuer: Issuer<'static, KeyPair>,
    ca_pem: String,
    /// One enrolment at a time: invites and the enrolled file are
    /// rewritten as a whole.
    writing: Mutex<()>,
}

/// Why an enrolment was refused (logged; the client sees one reason).
pub(crate) type Refusal = String;

impl Enrolment {
    /// Reads the CA key and the CA certificate (`client_ca`'s first).
    pub(crate) fn load(config: &EnrolmentConfig, client_ca: &str) -> Result<Self, String> {
        let key_pem = std::fs::read_to_string(&config.ca_key)
            .map_err(|error| format!("cannot read bnp.enrolment.ca_key: {error}"))?;
        let key = KeyPair::from_pem(&key_pem)
            .map_err(|error| format!("invalid bnp.enrolment.ca_key: {error}"))?;
        let ca_text = std::fs::read_to_string(client_ca)
            .map_err(|error| format!("cannot read bnp.client_ca: {error}"))?;
        let ca_pem = first_certificate(&ca_text).ok_or("bnp.client_ca holds no certificate")?;
        let issuer = Issuer::from_ca_cert_pem(&ca_pem, key)
            .map_err(|error| format!("bnp.enrolment.ca_key does not fit bnp.client_ca: {error}"))?;
        Ok(Self {
            config: config.clone(),
            issuer,
            ca_pem,
            writing: Mutex::new(()),
        })
    }

    /// Every entry enrolled so far (none before the first enrolment).
    pub(crate) fn enrolled(&self) -> Result<Vec<BnpRosterEntry>, String> {
        match std::fs::read_to_string(&self.config.enrolled) {
            Ok(text) => serde_json::from_str::<EnrolledFile>(&text)
                .map(|file| file.roster)
                .map_err(|error| format!("invalid bnp.enrolment.enrolled: {error}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(format!("cannot read bnp.enrolment.enrolled: {error}")),
        }
    }

    /// Spends the invite whose secret is `secret` on `csr_pem`: issues the
    /// certificate, records it, adds it to `roster`, and returns the
    /// participant and the certificate PEM with the CA PEM.
    pub(crate) fn enroll(
        &self,
        secret: &str,
        csr_pem: &str,
        roster: &RwLock<Roster>,
    ) -> Result<(u128, String, String), Refusal> {
        let _writing = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let mut invites: InvitesFile = std::fs::read_to_string(&self.config.invites)
            .map_err(|error| format!("cannot read invites: {error}"))
            .and_then(|text| {
                serde_json::from_str(&text).map_err(|error| format!("invalid invites: {error}"))
            })?;
        let digest = hex(&Sha256::digest(secret.as_bytes()));
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let invite = invites
            .invites
            .iter_mut()
            .find(|invite| invite.secret_sha256 == digest)
            .ok_or("unknown invite")?;
        if invite.used {
            return Err("invite already used".to_owned());
        }
        if invite.expires_unix.is_some_and(|expires| now >= expires) {
            return Err("invite expired".to_owned());
        }
        let participant_id = invite.participant_id;
        let mut enrolled = self.enrolled()?;
        if enrolled
            .iter()
            .any(|entry| entry.participant_id == participant_id)
        {
            return Err(format!("participant {participant_id} already enrolled"));
        }
        if enrolled.len() >= self.config.max_enrolled {
            return Err("bnp.enrolment.max_enrolled reached".to_owned());
        }
        let mut request = CertificateSigningRequestParams::from_pem(csr_pem)
            .map_err(|error| format!("invalid certificate request: {error}"))?;
        let mut params = CertificateParams::default();
        params.distinguished_name.push(
            DnType::CommonName,
            format!("Bunting participant {participant_id}"),
        );
        params.is_ca = IsCa::NoCa;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ClientAuth];
        let (from, until) = validity(now, self.config.certificate_days);
        params.not_before = rcgen::date_time_ymd(from.0, from.1, from.2);
        params.not_after = rcgen::date_time_ymd(until.0, until.1, until.2);
        request.params = params;
        let certificate = request
            .signed_by(&self.issuer)
            .map_err(|error| format!("cannot issue a certificate: {error}"))?;
        let fingerprint: [u8; 32] = Sha256::digest(certificate.der().as_ref()).into();
        let entry = BnpRosterEntry {
            certificate_sha256: hex(&fingerprint),
            participant_id,
            role: ActorRole::Participant,
        };
        // Record first: a certificate the file does not hold would not
        // survive a restart.
        enrolled.push(entry.clone());
        write_atomic(
            &self.config.enrolled,
            &EnrolledFile {
                version: 1,
                roster: enrolled,
            },
        )?;
        invite.used = true;
        write_atomic(&self.config.invites, &invites)?;
        roster
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(fingerprint, entry);
        Ok((participant_id, certificate.pem(), self.ca_pem.clone()))
    }
}

/// The first PEM certificate block in `text`.
fn first_certificate(text: &str) -> Option<String> {
    const END: &str = "-----END CERTIFICATE-----";
    let start = text.find("-----BEGIN CERTIFICATE-----")?;
    let end = text[start..].find(END)? + start + END.len();
    Some(format!("{}\n", &text[start..end]))
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// `(year, month, day)` of the day before `now` and of `days` after it
/// (UTC): the day before absorbs clock skew between venue and client.
fn validity(now_unix: u64, days: u32) -> ((i32, u8, u8), (i32, u8, u8)) {
    let today = i64::try_from(now_unix / 86_400).unwrap_or(0);
    (
        civil_from_days(today - 1),
        civil_from_days(today + i64::from(days)),
    )
}

/// Gregorian date of a day count since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i32, u8, u8) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u8::try_from(month).unwrap_or(1),
        u8::try_from(day).unwrap_or(1),
    )
}

/// Writes `value` as JSON beside `path`, owner-only, and renames it into
/// place.
fn write_atomic(path: &str, value: &impl Serialize) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let path = Path::new(path);
    let partial = path.with_extension("json.partial");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(&partial)
        .and_then(|mut file| std::io::Write::write_all(&mut file, text.as_bytes()))
        .and_then(|()| std::fs::rename(&partial, path))
        .map_err(|error| format!("cannot write {}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_map_to_gregorian_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
        // 2026-10-22 is day 20,748.
        assert_eq!(civil_from_days(20_748), (2026, 10, 22));
        assert_eq!(
            validity(20_748 * 86_400 + 5, 365),
            ((2026, 10, 21), (2027, 10, 22))
        );
    }

    #[test]
    fn the_first_certificate_is_taken_from_a_bundle() {
        let bundle = "-----BEGIN CERTIFICATE-----\nAA\n-----END CERTIFICATE-----\n-----BEGIN CERTIFICATE-----\nBB\n-----END CERTIFICATE-----\n";
        assert_eq!(
            first_certificate(bundle).as_deref(),
            Some("-----BEGIN CERTIFICATE-----\nAA\n-----END CERTIFICATE-----\n")
        );
        assert_eq!(first_certificate("none"), None);
    }
}
