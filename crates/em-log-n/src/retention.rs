//! Pluggable retention / deletion policy.
//!
//! Applied at **compaction time only** (never on the write or read hot path).
//! The default [`KeepAll`] is append-only forever, which keeps the door open
//! to GDPR-style point delete without committing to it now.

use crate::error::Result;

/// Decision returned by [`RetentionPolicy::decide`] for each row a compaction
/// considers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Carry the row forward into the new segment.
    Keep,
    /// Drop the row (it will not appear in the new segment, and any vector
    /// index entries keyed by its row key are eligible for removal by the
    /// compaction).
    Drop,
}

/// Compaction-time retention hook.
pub trait RetentionPolicy: Send + Sync {
    /// Decide what to do with one row.
    ///
    /// `key_bytes` is the raw row key (caller can parse with `KeyParts`).
    /// `value_bytes` is the raw codec-encoded value (caller decodes if needed).
    ///
    /// # Errors
    /// Returns an error if the policy cannot decide (e.g. underlying lookup
    /// failed). Errors abort the compaction safely; nothing is dropped.
    fn decide(&self, key_bytes: &[u8], value_bytes: &[u8]) -> Result<Decision>;
}

/// Default policy: keep everything forever.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeepAll;

impl RetentionPolicy for KeepAll {
    fn decide(&self, _key_bytes: &[u8], _value_bytes: &[u8]) -> Result<Decision> {
        Ok(Decision::Keep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keep_all_keeps_everything() {
        let p = KeepAll;
        assert_eq!(p.decide(b"any", b"thing").unwrap(), Decision::Keep);
    }
}
