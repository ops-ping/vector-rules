//! 64-bit SimHash and Hamming-distance near-duplicate detection.
//!
//! SimHash maps a token bag to a 64-bit fingerprint such that *similar* inputs
//! have *small* Hamming distance. It is the cheap pre-filter that lets the cache
//! reuse an embedding for near-duplicate text that canonicalization didn't make
//! byte-identical (e.g. reordered fields, a stray extra word).
//!
//! Dependency-free: per-token mixing uses the crate's FNV-1a.

use crate::fnv1a_64;

/// A computed 64-bit SimHash fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SimHash64(pub u64);

impl SimHash64 {
    /// Compute the SimHash of a token bag. Each token contributes ±1 to every
    /// bit column according to its FNV-1a hash; the sign of each column's sum
    /// becomes the output bit. Order-independent (it's a bag).
    #[must_use]
    pub fn compute<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut acc = [0i32; 64];
        for tok in tokens {
            let h = fnv1a_64(tok.as_ref().as_bytes());
            for (bit, slot) in acc.iter_mut().enumerate() {
                if (h >> bit) & 1 == 1 {
                    *slot += 1;
                } else {
                    *slot -= 1;
                }
            }
        }
        let mut out = 0u64;
        for (bit, &slot) in acc.iter().enumerate() {
            if slot > 0 {
                out |= 1 << bit;
            }
        }
        Self(out)
    }

    /// Convenience: SimHash of whitespace-split tokens of `text`.
    #[must_use]
    pub fn of_text(text: &str) -> Self {
        Self::compute(text.split_whitespace())
    }

    /// Hamming distance to another fingerprint (0 = identical).
    #[must_use]
    pub fn distance(self, other: Self) -> u32 {
        (self.0 ^ other.0).count_ones()
    }
}

/// Hamming distance between two raw 64-bit fingerprints.
#[must_use]
pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Tracks seen fingerprints and answers "is this a near-duplicate of something
/// I've already seen?" within a Hamming `threshold`.
///
/// Linear scan — fine for the modest working sets a per-process embed cache
/// sees. Swap in an LSH banding index if the seen-set grows large.
#[derive(Debug, Clone)]
pub struct NearDupChecker {
    threshold: u32,
    seen: Vec<u64>,
}

impl Default for NearDupChecker {
    fn default() -> Self {
        Self::new(3)
    }
}

impl NearDupChecker {
    /// New checker treating fingerprints within `threshold` bits as duplicates.
    #[must_use]
    pub fn new(threshold: u32) -> Self {
        Self {
            threshold,
            seen: Vec::new(),
        }
    }

    /// Return the first seen fingerprint within `threshold` of `h`, if any.
    #[must_use]
    pub fn nearest(&self, h: SimHash64) -> Option<u64> {
        self.seen
            .iter()
            .copied()
            .find(|&s| hamming_distance(s, h.0) <= self.threshold)
    }

    /// Record `h` as seen. Returns the near-duplicate it matched (if any) BEFORE
    /// inserting, so callers can reuse that entry's cached vector.
    pub fn insert(&mut self, h: SimHash64) -> Option<u64> {
        let hit = self.nearest(h);
        self.seen.push(h.0);
        hit
    }

    /// Number of distinct fingerprints recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether no fingerprints have been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_zero_distance() {
        let a = SimHash64::of_text("the quick brown fox jumps");
        let b = SimHash64::of_text("the quick brown fox jumps");
        assert_eq!(a, b);
        assert_eq!(a.distance(b), 0);
    }

    #[test]
    fn bag_is_order_independent() {
        let a = SimHash64::of_text("alpha beta gamma delta");
        let b = SimHash64::of_text("delta gamma beta alpha");
        assert_eq!(a, b);
    }

    #[test]
    fn near_dup_small_distance_unrelated_large() {
        let base = SimHash64::of_text("user login from console session active");
        let near = SimHash64::of_text("user login from console session idle");
        let far = SimHash64::of_text("disk failure replication halted urgently now");
        assert!(base.distance(near) < base.distance(far));
    }

    #[test]
    fn checker_flags_near_duplicate() {
        let mut chk = NearDupChecker::new(8);
        let a = SimHash64::of_text("payment processed for order alpha beta gamma");
        let b = SimHash64::of_text("payment processed for order alpha beta delta");
        assert_eq!(chk.insert(a), None, "first insert has no prior match");
        assert!(chk.insert(b).is_some(), "near-dup should match prior");
        assert_eq!(chk.len(), 2);
    }

    #[test]
    fn checker_rejects_unrelated() {
        let mut chk = NearDupChecker::new(3);
        chk.insert(SimHash64::of_text("alpha beta gamma delta epsilon zeta"));
        let unrelated = SimHash64::of_text("completely different words here entirely now");
        assert_eq!(chk.nearest(unrelated), None);
    }

    #[test]
    fn raw_hamming_helper() {
        assert_eq!(hamming_distance(0b1011, 0b1110), 2);
        assert_eq!(hamming_distance(0, 0), 0);
    }
}
