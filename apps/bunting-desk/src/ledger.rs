//! The desk's own fills and P&L kept between launches. P&L, cost and
//! realized figures are the desk's to compute from its fills; a relaunch
//! mid-run would otherwise lose the cost of lots already held. The
//! [`LedgerRecord`] is saved here, keyed to one venue run and one
//! certificate, and the next launch resumes the private stream at its
//! cursor so every later report replays on top of it.

use bunting_trading_state::LedgerRecord;
use bunting_trading_state::bunting_client::{ClientConfig, certificate_fingerprint};
use std::path::{Path, PathBuf};

/// Where one venue run's record lives.
#[derive(Clone, Debug)]
pub struct LedgerStore {
    path: PathBuf,
}

impl LedgerStore {
    /// The record for `client`'s certificate on the venue it dials;
    /// `instance` names one start of a local practice venue (empty for a
    /// hosted venue, whose run survives restarts).
    #[must_use]
    pub fn for_client(client: &ClientConfig, instance: &str) -> Option<Self> {
        let fingerprint = certificate_fingerprint(&client.certificate_pem).ok()?;
        let short = fingerprint.get(..16).unwrap_or(&fingerprint);
        let name = if instance.is_empty() {
            format!("{short}.json")
        } else {
            format!("{short}-{instance}.json")
        };
        Some(Self::at(
            crate::appearance::desk_config_dir()
                .join("ledgers")
                .join(name),
        ))
    }

    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved record, if one is readable.
    #[must_use]
    pub fn load(&self) -> Option<LedgerRecord> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        LedgerRecord::from_json(&serde_json::from_str(&text).ok()?)
    }

    /// Writes `record` beside the file and renames it into place, so a
    /// crash never leaves half a record.
    ///
    /// # Errors
    /// Returns why it could not be written.
    pub fn save(&self, record: &LedgerRecord) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let partial = self.path.with_extension("json.partial");
        std::fs::write(&partial, record.to_json().to_string())?;
        std::fs::rename(&partial, &self.path)
    }
}
