//! Bounded, checksummed, append-only native commit records.
//!
//! A complete synced record is the recovery boundary. An incomplete final
//! record is a crash tail and is truncated; a complete bad checksum is fatal.
use bunting_origin_store::{CommitRequest, OriginError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAGIC: [u8; 8] = *b"BUNTWAL1";
const HEADER_SIZE: u64 = 48;
const VERSION: u16 = 1;
const MAX_RECORD_BYTES: u64 = 256 * 1024 * 1024;

/// Checkpoint every 128 successfully committed requests, not on each order.
pub(crate) const CHECKPOINT_INTERVAL: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalRecord {
    version: u16,
    request: CommitRequest,
}

#[derive(Serialize)]
struct RecordRef<'a> {
    version: u16,
    request: &'a CommitRequest,
}

pub(crate) fn path_for(checkpoint: &Path) -> PathBuf {
    checkpoint.with_extension("wal")
}

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

/// Appends and synchronizes exactly one complete command/state/event record.
/// No in-memory state may be published before this function succeeds.
pub(crate) fn append(path: &Path, request: &CommitRequest) -> Result<(), OriginError> {
    if let Some(parent) = path.parent().filter(|item| !item.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|_| OriginError::Unavailable)?;
    }
    let existed = path.exists();
    let bytes = serde_json::to_vec(&RecordRef {
        version: VERSION,
        request,
    })
    .map_err(|_| OriginError::InvalidCommit)?;
    let length = u64::try_from(bytes.len()).map_err(|_| OriginError::InvalidCommit)?;
    if length == 0 || length > MAX_RECORD_BYTES {
        return Err(OriginError::InvalidCommit);
    }
    let digest = Sha256::digest(&bytes);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|_| OriginError::Unavailable)?;
    file.write_all(&MAGIC)
        .and_then(|()| file.write_all(&length.to_le_bytes()))
        .and_then(|()| file.write_all(&digest))
        .and_then(|()| file.write_all(&bytes))
        .and_then(|()| file.sync_all())
        .map_err(|_| OriginError::Unavailable)?;
    if !existed {
        sync_parent(path)?;
    }
    Ok(())
}

/// Replays the complete durable prefix and cuts off only an incomplete tail.
///
/// A complete malformed or hash-mismatched record always aborts recovery.
pub(crate) fn replay(
    path: &Path,
    mut apply: impl FnMut(CommitRequest) -> Result<(), OriginError>,
) -> Result<(), OriginError> {
    if !path.exists() {
        return Ok(());
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|_| OriginError::Unavailable)?;
    let end = file.metadata().map_err(|_| OriginError::Unavailable)?.len();
    let mut offset = 0_u64;
    while offset < end {
        if end - offset < HEADER_SIZE {
            truncate_tail(&file, offset)?;
            return Ok(());
        }
        let mut header = [0_u8; HEADER_SIZE as usize];
        file.read_exact(&mut header)
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
            truncate_tail(&file, offset)?;
            return Ok(());
        }
        let mut bytes =
            vec![0_u8; usize::try_from(length).map_err(|_| OriginError::InvalidCommit)?];
        file.read_exact(&mut bytes)
            .map_err(|_| OriginError::Unavailable)?;
        let digest = Sha256::digest(&bytes);
        if digest[..] != header[16..] {
            return Err(OriginError::InvalidCommit);
        }
        let record: JournalRecord =
            serde_json::from_slice(&bytes).map_err(|_| OriginError::InvalidCommit)?;
        if record.version != VERSION {
            return Err(OriginError::InvalidCommit);
        }
        apply(record.request)?;
        offset = offset
            .checked_add(HEADER_SIZE)
            .and_then(|value| value.checked_add(length))
            .ok_or(OriginError::InvalidCommit)?;
    }
    Ok(())
}

fn truncate_tail(file: &File, offset: u64) -> Result<(), OriginError> {
    file.set_len(offset)
        .and_then(|()| file.sync_all())
        .map_err(|_| OriginError::Unavailable)
}

/// Clears the already-checkpointed journal only after checkpoint durability.
/// Duplicate records left by a failed clear are harmless and checked on replay.
pub(crate) fn clear(path: &Path) -> Result<(), OriginError> {
    if !path.exists() {
        return Ok(());
    }
    let file = OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|_| OriginError::Unavailable)?;
    file.set_len(0)
        .and_then(|()| file.sync_all())
        .map_err(|_| OriginError::Unavailable)
}
