//! Bounded, checksummed, append-only native journal (format 3).
//!
//! The journal is the run's complete authoritative history: a genesis
//! snapshot per run followed by one command-sourced record per committed
//! input. It is never truncated except for an incomplete crash tail; a
//! complete record with a bad checksum or shape is fatal.
use bunting_engine::EngineSnapshotEnvelope;
use bunting_origin_store::{CommandRecord, OriginError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: [u8; 8] = *b"BUNTWAL3";
const HEADER_SIZE: u64 = 48;
const MAX_RECORD_BYTES: u64 = 256 * 1024 * 1024;

/// One journal entry.
#[derive(Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum JournalEntry {
    Genesis {
        snapshot: Box<EngineSnapshotEnvelope>,
    },
    Command(Box<CommandRecord>),
}

/// Borrowed form of [`JournalEntry`] with the identical encoding.
#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EntryRef<'a> {
    Genesis {
        snapshot: &'a EngineSnapshotEnvelope,
    },
    Command(&'a CommandRecord),
}

pub(crate) fn path_for(checkpoint: &Path) -> PathBuf {
    checkpoint.with_extension("wal")
}

// Windows has no portable directory sync; NTFS journals the rename itself.
#[cfg_attr(not(unix), allow(clippy::unnecessary_wraps))]
fn sync_parent(path: &Path) -> Result<(), OriginError> {
    #[cfg(unix)]
    {
        let parent = path
            .parent()
            .filter(|item| !item.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        File::open(parent)
            .and_then(|parent| parent.sync_all())
            .map_err(|_| OriginError::Unavailable)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// The open append handle of the single writer.
#[derive(Debug)]
pub(crate) struct JournalWriter {
    file: File,
}

impl JournalWriter {
    pub(crate) fn open(path: &Path) -> Result<Self, OriginError> {
        if let Some(parent) = path.parent().filter(|item| !item.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|_| OriginError::Unavailable)?;
        }
        let existed = path.exists();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|_| OriginError::Unavailable)?;
        if !existed {
            sync_parent(path)?;
        }
        Ok(Self { file })
    }

    /// Appends and synchronizes exactly one complete entry. Nothing may be
    /// acknowledged or published before this returns `Ok`; on `Err` the
    /// frame may or may not be durable, so the caller must stop serving.
    pub(crate) fn append(&mut self, entry: &EntryRef<'_>) -> Result<(), OriginError> {
        let bytes = serde_json::to_vec(entry).map_err(|_| OriginError::InvalidCommit)?;
        let length = u64::try_from(bytes.len()).map_err(|_| OriginError::InvalidCommit)?;
        if length == 0 || length > MAX_RECORD_BYTES {
            return Err(OriginError::InvalidCommit);
        }
        let mut frame = Vec::with_capacity(bytes.len().saturating_add(48));
        frame.extend_from_slice(&MAGIC);
        frame.extend_from_slice(&length.to_le_bytes());
        frame.extend_from_slice(&Sha256::digest(&bytes));
        frame.extend_from_slice(&bytes);
        self.file
            .write_all(&frame)
            .and_then(|()| self.file.sync_data())
            .map_err(|_| OriginError::Unavailable)
    }
}

/// Visits every complete entry in order. With `repair`, an incomplete final
/// frame (a crash tail) is cut off durably; without it, scanning stops there.
///
/// A complete malformed or hash-mismatched frame always aborts.
pub(crate) fn scan(
    path: &Path,
    repair: bool,
    mut visit: impl FnMut(JournalEntry) -> Result<(), OriginError>,
) -> Result<(), OriginError> {
    if !path.exists() {
        return Ok(());
    }
    let file = OpenOptions::new()
        .read(true)
        .write(repair)
        .open(path)
        .map_err(|_| OriginError::Unavailable)?;
    let end = file.metadata().map_err(|_| OriginError::Unavailable)?.len();
    let mut reader = std::io::BufReader::new(&file);
    let mut offset = 0_u64;
    while offset < end {
        if end - offset < HEADER_SIZE {
            return cut_tail(&file, offset, repair);
        }
        let mut header = [0_u8; 48];
        reader
            .read_exact(&mut header)
            .map_err(|_| OriginError::Unavailable)?;
        if header[..8] != MAGIC {
            return Err(OriginError::InvalidCommit);
        }
        let length_bytes: [u8; 8] = header[8..16]
            .try_into()
            .map_err(|_| OriginError::InvalidCommit)?;
        let length = u64::from_le_bytes(length_bytes);
        if length == 0 || length > MAX_RECORD_BYTES {
            return Err(OriginError::InvalidCommit);
        }
        if length > end - offset - HEADER_SIZE {
            return cut_tail(&file, offset, repair);
        }
        let mut bytes =
            vec![0_u8; usize::try_from(length).map_err(|_| OriginError::InvalidCommit)?];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| OriginError::Unavailable)?;
        if Sha256::digest(&bytes)[..] != header[16..] {
            return Err(OriginError::InvalidCommit);
        }
        visit(serde_json::from_slice(&bytes).map_err(|_| OriginError::InvalidCommit)?)?;
        offset = offset
            .checked_add(HEADER_SIZE)
            .and_then(|value| value.checked_add(length))
            .ok_or(OriginError::InvalidCommit)?;
    }
    Ok(())
}

fn cut_tail(file: &File, offset: u64, repair: bool) -> Result<(), OriginError> {
    if !repair {
        return Ok(());
    }
    file.set_len(offset)
        .and_then(|()| file.sync_all())
        .map_err(|_| OriginError::Unavailable)
}
