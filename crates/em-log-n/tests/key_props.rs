//! Property tests for `em_log_n::key`.
//!
//! The headline property — **lexicographic byte order on the encoded row key
//! is the same as logical newest-first order on the original timestamp** — is
//! what justifies fjall's `DoubleEndedIterator` / `.rev()` giving us
//! newest-first scans for free. If this ever breaks, the entire recency
//! contract of the store is broken, so it gets a proptest, not just a unit
//! test.

use em_log_n::key::{time_range_bytes, KeyBuilder, KeyParts, DEFAULT_HASH_LEN};
use proptest::prelude::*;

proptest! {
    /// Newer timestamps must encode to lexicographically-smaller key bytes
    /// (so an ascending scan yields newest-first).
    #[test]
    fn newer_ts_sorts_first(a in any::<u64>(), b in any::<u64>(), text in any::<Vec<u8>>()) {
        let ka = KeyBuilder::new(a, &text).build();
        let kb = KeyBuilder::new(b, &text).build();
        match a.cmp(&b) {
            std::cmp::Ordering::Less    => prop_assert!(ka.as_bytes() >  kb.as_bytes()),
            std::cmp::Ordering::Equal   => prop_assert!(ka.as_bytes() == kb.as_bytes()),
            std::cmp::Ordering::Greater => prop_assert!(ka.as_bytes() <  kb.as_bytes()),
        }
    }

    /// Round-trip: KeyBuilder → KeyParts recovers ts_nanos exactly.
    #[test]
    fn ts_round_trip(ts in any::<u64>(), text in any::<Vec<u8>>()) {
        let k = KeyBuilder::new(ts, &text).build();
        let parts = KeyParts::parse(k.as_bytes(), DEFAULT_HASH_LEN).unwrap();
        prop_assert_eq!(parts.ts_nanos, ts);
        prop_assert!(parts.tiebreaker.is_none());
        prop_assert_eq!(parts.hash.len(), DEFAULT_HASH_LEN);
    }

    /// Round-trip with tiebreaker.
    #[test]
    fn ts_tiebreaker_round_trip(ts in any::<u64>(), tb in any::<u64>(), text in any::<Vec<u8>>()) {
        let k = KeyBuilder::new(ts, &text).with_tiebreaker(tb).build();
        let parts = KeyParts::parse(k.as_bytes(), DEFAULT_HASH_LEN).unwrap();
        prop_assert_eq!(parts.ts_nanos, ts);
        prop_assert_eq!(parts.tiebreaker, Some(tb));
    }

    /// At equal ts, distinct text ⇒ distinct keys (no hash collision in the
    /// 128-bit truncated blake3 over short test strings).
    #[test]
    fn equal_ts_distinct_text_no_collision(
        ts in any::<u64>(),
        a in proptest::collection::vec(any::<u8>(), 1..64),
        b in proptest::collection::vec(any::<u8>(), 1..64),
    ) {
        prop_assume!(a != b);
        let ka = KeyBuilder::new(ts, &a).build();
        let kb = KeyBuilder::new(ts, &b).build();
        prop_assert_ne!(ka.as_bytes(), kb.as_bytes());
        // ts prefix is identical
        prop_assert_eq!(&ka.as_bytes()[..8], &kb.as_bytes()[..8]);
    }

    /// `time_range_bytes(t_lo, t_hi)` returns (start, end) such that
    /// start <= encoded(ts)[..8] <= end iff ts in [t_lo, t_hi).
    #[test]
    fn time_range_bounds_are_correct(
        t_lo in 1u64..u64::MAX,
        span in 1u64..1_000_000,
        probe_delta in -1_000_000i64..1_000_000,
    ) {
        let t_hi = t_lo.saturating_add(span);
        let (start, end) = time_range_bytes(t_lo, t_hi);

        // Probe ts is t_lo plus an offset that may land inside or outside the window.
        let probe = if probe_delta >= 0 {
            t_lo.saturating_add(probe_delta as u64)
        } else {
            t_lo.saturating_sub((-probe_delta) as u64)
        };
        let prefix = (u64::MAX - probe).to_be_bytes();
        let in_window = probe >= t_lo && probe < t_hi;
        let in_byte_range = prefix.as_slice() >= start.as_slice()
                         && prefix.as_slice() <= end.as_slice();
        prop_assert_eq!(in_window, in_byte_range);
    }
}
