//! # vrules-canon
//!
//! Deterministic, **pluggable** canonicalization of text and JSON to maximize
//! embedding-cache hit rates without compromising recall — plus the vrules
//! **execution-audit** canonical form, because an audit cache key is just
//! another canonicalization of a request.
//!
//! Two layers, feature-gated so the canon core stays pure (zero-dependency):
//! - **canon core** (default): the [`Canonicalizer`] strategies ([`LogMask`],
//!   `JsonHybrid`, `SchemaFingerprint`, [`Identity`]), [`SimHash64`]
//!   near-duplicate detection, and [`fnv1a_64`].
//! - **audit** (`audit` feature): `ExecutionRecord` + `ExecCanonicalizer` — the
//!   serde-serializable execution log line and its request/backend cache key
//!   (the em-log-n sink), expressed as a [`Canonicalizer`] over the core.
//!
//! ## Why
//!
//! Embedding is ~150× slower than the rest of the em-log-n storage layer, and
//! its content-addressed [`EmbedCache`] keys by `blake3(text)`. Two log lines
//! that differ only in variables (IDs, timestamps, IPs) miss the cache and pay
//! a full embed. Canonicalizing the *variable* parts away (`User 42 login` →
//! `User <*> login`) collapses recurring patterns into one cache key.
//!
//! ## Determinism is a hard requirement
//!
//! A content-addressed cache REQUIRES that canonicalization be a **pure,
//! deterministic, stateless function** — `canon(x)` must never depend on
//! process state or ingest order, or the cache is silently poisoned. Every
//! [`Canonicalizer`] in this crate is pure. (Adaptive template learners such as
//! Drain are intentionally excluded from v0 for this reason; if added later they
//! must be frozen/read-only at inference time.)
//!
//! ## Pluggable per type
//!
//! Different indexed types need different canonicalization (free-form logs vs.
//! structured JSON vs. opaque blobs). Strategy is selected per type via the
//! [`Canonicalizer`] trait rather than one global auto-detect:
//!
//! - [`LogMask`] — stateless masking of ints/hex/UUID/IP/timestamp/path.
//! - [`JsonHybrid`] — keep string values (the signal), mask numbers/ids, sort
//!   keys (feature `json`).
//! - [`SchemaFingerprint`] — strip ALL values to type tokens; for *dedup
//!   detection only*, never as the thing you embed (feature `json`).
//! - [`Identity`] — no-op passthrough.
//!
//! Each strategy reports a stable [`Canonicalizer::id`] and
//! [`Canonicalizer::version`]; downstream cache keys must namespace by both so
//! different strategies (or a rule change) never collide on the same text.

#![forbid(unsafe_code)]

pub mod mask;
pub mod sim;

#[cfg(feature = "json")]
pub mod json;

/// The execution-audit canonical form: the [`ExecutionRecord`] log line and the
/// [`ExecCanonicalizer`] request/backend cache key. Gated on `audit` (pulls
/// serde) so the canon core stays zero-dependency.
#[cfg(feature = "audit")]
pub mod audit;

pub use mask::LogMask;
pub use sim::{hamming_distance, NearDupChecker, SimHash64};

#[cfg(feature = "json")]
pub use json::{JsonHybrid, SchemaFingerprint};

#[cfg(feature = "audit")]
pub use audit::{CacheState, ExecCanonicalizer, ExecutionRecord};

/// Which canonicalization produced a [`CanonResult`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonMode {
    /// Free-form text masking ([`LogMask`]).
    Log,
    /// Structured hybrid JSON ([`JsonHybrid`]).
    Json,
    /// Pure schema fingerprint ([`SchemaFingerprint`]) — dedup only.
    Schema,
    /// No-op ([`Identity`]).
    Identity,
}

/// Result of canonicalizing one input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonResult {
    /// The canonical string — this is what you embed / hash for the cache key.
    pub canonical: String,
    /// 64-bit FNV-1a hash of `canonical`. A cheap, dependency-free stable id
    /// for template grouping. NOT a cryptographic digest.
    pub id: u64,
    /// The masked-out variable substrings, in order of appearance. Empty for
    /// [`Identity`] and for inputs with no variable parts.
    pub vars: Vec<String>,
    /// Which strategy produced this result.
    pub mode: CanonMode,
}

impl CanonResult {
    /// Build a result, computing the FNV-1a id from `canonical`.
    #[must_use]
    pub fn new(canonical: String, vars: Vec<String>, mode: CanonMode) -> Self {
        let id = fnv1a_64(canonical.as_bytes());
        Self {
            canonical,
            id,
            vars,
            mode,
        }
    }
}

/// A pure, deterministic canonicalization strategy.
///
/// Implementations MUST be referentially transparent: `canon(x)` depends only
/// on `x`. No global mutable state, no ingest-order dependence. This is what
/// makes the downstream content-addressed cache correct.
pub trait Canonicalizer {
    /// Stable short identifier (e.g. `"log-mask"`). Used to namespace cache
    /// keys so different strategies on the same text don't collide.
    fn id(&self) -> &str;

    /// Strategy version. **Bump whenever the canonicalization rules change** so
    /// existing cache entries cleanly miss instead of silently serving a vector
    /// computed under the old rules.
    fn version(&self) -> u32;

    /// Canonicalize one input. Pure and deterministic.
    fn canon(&self, input: &str) -> CanonResult;
}

/// Crate-wide canonicalization epoch. Bump on any cross-strategy change to the
/// shared masking primitives in [`mask`]. Per-strategy [`Canonicalizer::version`]
/// is the finer-grained knob; this is the coarse backstop.
#[must_use]
pub const fn canon_version() -> u32 {
    1
}

/// Convenience auto-detect: structured input (`json` feature, trimmed starts
/// with `{` or `[`) → [`JsonHybrid`]; otherwise → [`LogMask`].
///
/// Explicit strategy selection via a concrete [`Canonicalizer`] is the primary
/// API; this helper exists for callers that genuinely don't know the type.
#[must_use]
pub fn canonicalize(text: &str) -> CanonResult {
    let trimmed = text.trim_start();
    #[cfg(feature = "json")]
    {
        if trimmed.starts_with('{') || trimmed.starts_with('[') {
            if let Some(r) = JsonHybrid.try_canon(text) {
                return r;
            }
        }
    }
    let _ = trimmed;
    LogMask.canon(text)
}

/// No-op canonicalizer: returns the input unchanged. Useful for opaque types
/// where masking would destroy meaning, or to A/B against canonicalization.
#[derive(Debug, Default, Clone, Copy)]
pub struct Identity;

impl Canonicalizer for Identity {
    fn id(&self) -> &str {
        "identity"
    }
    fn version(&self) -> u32 {
        1
    }
    fn canon(&self, input: &str) -> CanonResult {
        CanonResult::new(input.to_owned(), Vec::new(), CanonMode::Identity)
    }
}

/// 64-bit FNV-1a. Dependency-free, deterministic, fast. Used for stable
/// template ids and as the SimHash per-token mixer — NOT a security hash.
#[must_use]
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h = OFFSET;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_passes_through() {
        let r = Identity.canon("anything 123");
        assert_eq!(r.canonical, "anything 123");
        assert!(r.vars.is_empty());
        assert_eq!(r.mode, CanonMode::Identity);
    }

    #[test]
    fn canon_result_id_is_deterministic() {
        let a = CanonResult::new("User <*> login".into(), vec![], CanonMode::Log);
        let b = CanonResult::new("User <*> login".into(), vec![], CanonMode::Log);
        assert_eq!(a.id, b.id);
        let c = CanonResult::new("User <*> logout".into(), vec![], CanonMode::Log);
        assert_ne!(a.id, c.id);
    }

    #[test]
    fn fnv_known_vector() {
        // FNV-1a 64-bit of "" is the offset basis; of "a" is a known constant.
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
    }

    #[test]
    fn auto_detect_uses_log_for_plain_text() {
        let r = canonicalize("User 42 logged in");
        assert_eq!(r.mode, CanonMode::Log);
        assert_eq!(r.canonical, "User <*> logged in");
    }
}
