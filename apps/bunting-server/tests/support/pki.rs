//! A throwaway operator CA for the BNP mutual-TLS tests: a server
//! certificate for `localhost`, participant certificates, one revoked
//! certificate with its CRL, and a certificate from an unrelated CA.
//! Everything is generated per test run and written to a temporary
//! directory; no key material is committed.

use rcgen::{
    BasicConstraints, CertificateParams, CertificateRevocationListParams, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyIdMethod, KeyPair, KeyUsagePurpose, RevocationReason,
    RevokedCertParams, SerialNumber,
};
use std::path::{Path, PathBuf};
use time::{Duration, OffsetDateTime};

/// One issued certificate and its key, as PEM.
#[derive(Clone)]
pub struct Issued {
    pub certificate_pem: String,
    pub key_pem: String,
}

impl Issued {
    /// Lowercase hex SHA-256 of the certificate, as the roster wants it.
    pub fn fingerprint(&self) -> Result<String, String> {
        bunting_client::certificate_fingerprint(self.certificate_pem.as_bytes())
            .map_err(|error| error.to_string())
    }
}

pub struct Authority {
    issuer: Issuer<'static, KeyPair>,
    pub ca_pem: String,
    next_serial: u64,
}

impl Authority {
    pub fn new(name: &str) -> Result<Self, String> {
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let mut params =
            CertificateParams::new(Vec::<String>::new()).map_err(|error| error.to_string())?;
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        params.distinguished_name.push(DnType::CommonName, name);
        let certificate = params
            .self_signed(&key)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            issuer: Issuer::new(params, key),
            ca_pem: certificate.pem(),
            next_serial: 1,
        })
    }

    fn issue(
        &mut self,
        names: Vec<String>,
        common_name: &str,
        usage: ExtendedKeyUsagePurpose,
    ) -> Result<(Issued, u64), String> {
        let key = KeyPair::generate().map_err(|error| error.to_string())?;
        let mut params = CertificateParams::new(names).map_err(|error| error.to_string())?;
        let serial = self.next_serial;
        self.next_serial += 1;
        params.serial_number = Some(SerialNumber::from(serial));
        params
            .distinguished_name
            .push(DnType::CommonName, common_name);
        params.extended_key_usages = vec![usage];
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        let certificate = params
            .signed_by(&key, &self.issuer)
            .map_err(|error| error.to_string())?;
        Ok((
            Issued {
                certificate_pem: certificate.pem(),
                key_pem: key.serialize_pem(),
            },
            serial,
        ))
    }

    pub fn server(&mut self) -> Result<Issued, String> {
        self.issue(
            vec!["localhost".to_owned()],
            "Bunting venue",
            ExtendedKeyUsagePurpose::ServerAuth,
        )
        .map(|(issued, _)| issued)
    }

    /// A participant certificate and its serial number.
    pub fn client(&mut self, team: &str) -> Result<(Issued, u64), String> {
        self.issue(Vec::new(), team, ExtendedKeyUsagePurpose::ClientAuth)
    }

    /// A CRL revoking `serials`.
    pub fn revocation_list(&self, serials: &[u64]) -> Result<String, String> {
        let now = OffsetDateTime::now_utc();
        CertificateRevocationListParams {
            this_update: now - Duration::minutes(1),
            next_update: now + Duration::days(1),
            crl_number: SerialNumber::from(1_u64),
            issuing_distribution_point: None,
            revoked_certs: serials
                .iter()
                .map(|&serial| RevokedCertParams {
                    serial_number: SerialNumber::from(serial),
                    revocation_time: now - Duration::minutes(1),
                    reason_code: Some(RevocationReason::KeyCompromise),
                    invalidity_date: None,
                })
                .collect(),
            key_identifier_method: KeyIdMethod::Sha256,
        }
        .signed_by(&self.issuer)
        .map_err(|error| error.to_string())?
        .pem()
        .map_err(|error| error.to_string())
    }
}

/// Writes `contents` to `directory/name` and returns the path as a string.
pub fn write(directory: &Path, name: &str, contents: &str) -> Result<String, String> {
    let path: PathBuf = directory.join(name);
    std::fs::write(&path, contents).map_err(|error| error.to_string())?;
    Ok(path.display().to_string())
}
