#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]
//! Cloudflare Workers Cache adapter for immutable `OrderBook-rs` snapshots.
//!
//! Cache entries are content-addressed by run, instrument, event sequence and the
//! upstream snapshot checksum. The Cache API is a mandatory recovery accelerator,
//! not the only authoritative copy of accepted commands or participant balances.

use bunting_market_types::{EventSequence, InstrumentId, RunId};
use core::fmt;

/// Default edge TTL for immutable order-book snapshot packages.
pub const DEFAULT_SNAPSHOT_TTL_SECONDS: u32 = 300;
const CACHE_ORIGIN: &str = "https://cache.bunting.invalid";

/// Error constructing a safe cache key.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheKeyError {
    /// The checksum was not a lowercase or uppercase 64-character hex SHA-256 value.
    InvalidChecksum,
}

impl fmt::Display for CacheKeyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidChecksum => formatter.write_str("invalid snapshot checksum"),
        }
    }
}

impl std::error::Error for CacheKeyError {}

/// Immutable key for one committed snapshot package.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SnapshotCacheKey {
    run_id: RunId,
    instrument_id: InstrumentId,
    sequence: EventSequence,
    checksum: String,
}

impl SnapshotCacheKey {
    /// Creates a content-addressed snapshot cache key.
    pub fn new(
        run_id: RunId,
        instrument_id: InstrumentId,
        sequence: EventSequence,
        checksum: impl Into<String>,
    ) -> Result<Self, CacheKeyError> {
        let checksum = checksum.into();
        if checksum.len() != 64 || !checksum.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(CacheKeyError::InvalidChecksum);
        }
        Ok(Self {
            run_id,
            instrument_id,
            sequence,
            checksum,
        })
    }

    /// Returns the absolute URL required by the Workers Cache API.
    #[must_use]
    pub fn url(&self) -> String {
        format!(
            "{CACHE_ORIGIN}/v1/orderbooks/{}/{}/{}/{}",
            self.run_id.get(),
            self.instrument_id.get(),
            self.sequence.get(),
            self.checksum
        )
    }

    /// Returns the snapshot checksum used as the HTTP ETag.
    #[must_use]
    pub fn checksum(&self) -> &str {
        &self.checksum
    }

    /// Returns a standards-compliant strong HTTP ETag value.
    #[must_use]
    pub fn etag(&self) -> String {
        format!("\"{}\"", self.checksum)
    }

    /// Returns the committed event sequence represented by the snapshot.
    #[must_use]
    pub const fn sequence(&self) -> EventSequence {
        self.sequence
    }
}

/// Cache-control policy for immutable snapshot packages.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CachePolicy {
    /// Edge TTL in seconds.
    pub ttl_seconds: u32,
}

impl CachePolicy {
    /// Creates a policy with an explicit positive TTL.
    #[must_use]
    pub const fn new(ttl_seconds: u32) -> Self {
        Self { ttl_seconds }
    }

    /// Produces the header consumed by Cloudflare's Cache API.
    #[must_use]
    pub fn cache_control(self) -> String {
        format!("public, s-maxage={}, immutable", self.ttl_seconds)
    }
}

impl Default for CachePolicy {
    fn default() -> Self {
        Self::new(DEFAULT_SNAPSHOT_TTL_SECONDS)
    }
}

/// Workers Cache API operations.
pub mod cloudflare {
    use super::{CachePolicy, SnapshotCacheKey};
    use worker::{Cache, CacheDeletionOutcome, ResponseBuilder, Result};

    /// Reads an immutable snapshot package without making an origin subrequest.
    pub async fn get_json(key: &SnapshotCacheKey) -> Result<Option<String>> {
        let cache = Cache::default();
        let Some(mut response) = cache.get(key.url(), true).await? else {
            return Ok(None);
        };
        response.text().await.map(Some)
    }

    /// Stores a committed checksum-protected snapshot package.
    pub async fn put_json(
        key: &SnapshotCacheKey,
        json: String,
        policy: CachePolicy,
    ) -> Result<()> {
        let cache_control = policy.cache_control();
        let response = ResponseBuilder::new()
            .with_header("cache-control", &cache_control)?
            .with_header("content-type", "application/json")?
            .with_header("etag", &key.etag())?
            .with_header("x-bunting-event-sequence", &key.sequence().get().to_string())?
            .fixed(json.into_bytes());
        Cache::default().put(key.url(), response).await
    }

    /// Removes one cache entry. Origin state is unaffected.
    pub async fn delete(key: &SnapshotCacheKey) -> Result<CacheDeletionOutcome> {
        Cache::default().delete(key.url(), true).await
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_deterministic_and_content_addressed() {
        let key = SnapshotCacheKey::new(
            RunId::new(7),
            InstrumentId::new(11),
            EventSequence::new(19),
            "a".repeat(64),
        )
        .expect("valid test key");
        assert_eq!(
            key.url(),
            format!(
                "{CACHE_ORIGIN}/v1/orderbooks/7/11/19/{}",
                "a".repeat(64)
            )
        );
    }

    #[test]
    fn malformed_checksum_is_rejected() {
        assert_eq!(
            SnapshotCacheKey::new(
                RunId::new(1),
                InstrumentId::new(1),
                EventSequence::new(1),
                "not-a-checksum",
            ),
            Err(CacheKeyError::InvalidChecksum)
        );
    }
}
