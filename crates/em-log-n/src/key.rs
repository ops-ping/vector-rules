//! Typed row-key construction with strict lexicographic = logical ordering.
//!
//! The recommended ergonomic facade for the inverse-timestamp format is
//! [`KeyBuilder`] (kept for backward compatibility). Internally everything
//! routes through the [`KeyFormat`] trait, which lets callers plug their
//! own row-key layouts.
//!
//! ## Two key formats ship in the box
//!
//! 1. [`InverseTimestampKey`] — used by `Shard`. Layout:
//!    `reverse_ts_be (8B) || blake3(text)[..N] || optional tiebreaker`.
//!    Lex byte order = logical newest-first, so `DoubleEndedIterator`
//!    delivers reverse-chronological scans for free.
//! 2. [`ContentHashKey`] — used by `EmbedCache`. Layout:
//!    `blake3(model_id)[..8] || blake3(text)[..text_hash_len]`. No
//!    ordering semantics; a pure namespaced hash for content-addressed
//!    lookups.
//!
//! A third caller-supplied implementation is shown in `tests/keyformat.rs`.
//!
//! ## Layout invariant
//!
//! All formats produce **fixed-length, big-endian byte strings with no
//! varint or length-prefixed fields.** Lex byte order must equal the
//! intended logical order (or be irrelevant, as for [`ContentHashKey`]).
//! Any encoding that breaks this — varints, length-prefixed protobuf,
//! bincode, postcard — is forbidden for keys.

use crate::error::{Error, Result};

/// Default hash suffix length (bytes). 16 bytes ⇒ 128-bit collision space:
/// well past the birthday bound for any plausible per-ns write rate.
pub const DEFAULT_HASH_LEN: usize = 16;

/// Fully-encoded row key, owned.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowKey(Vec<u8>);

impl RowKey {
    /// Raw bytes (what fjall sees).
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Move out the inner buffer.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// Wrap an already-encoded buffer.
    ///
    /// # Errors
    /// Returns `BadKey` if the buffer is shorter than the minimum header
    /// of the default ([`InverseTimestampKey`]) layout. Callers using a
    /// custom [`KeyFormat`] with a shorter total length should construct
    /// `RowKey` via [`RowKey::from_bytes_unchecked`] instead.
    pub fn from_bytes(bytes: impl Into<Vec<u8>>) -> Result<Self> {
        let v = bytes.into();
        if v.len() < 8 + DEFAULT_HASH_LEN {
            return Err(Error::BadKey("row key shorter than 8 + DEFAULT_HASH_LEN"));
        }
        Ok(Self(v))
    }

    /// Wrap an already-encoded buffer without length validation. Use
    /// when constructing keys from custom [`KeyFormat`] impls whose
    /// total length differs from the default inverse-timestamp layout.
    #[must_use]
    pub fn from_bytes_unchecked(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }
}

// ---------------------------------------------------------------------------
// KeyFormat trait
// ---------------------------------------------------------------------------

/// Strategy for encoding caller inputs into fixed-layout key bytes.
///
/// Implementations document whether their byte order is meaningful (lex
/// order = newest-first for [`InverseTimestampKey`]; no meaningful order
/// for [`ContentHashKey`]).
///
/// `Input` is a GAT (generic associated type) so impls can take borrowed
/// inputs without forcing allocations on the hot path.
pub trait KeyFormat {
    /// Caller-supplied input the format encodes.
    type Input<'a>: 'a;

    /// Encoded key length in bytes. Fixed, not data-dependent.
    fn key_len(&self) -> usize;

    /// Encode `input` to a [`RowKey`].
    fn encode(&self, input: Self::Input<'_>) -> RowKey;
}

// ---------------------------------------------------------------------------
// Implementation 1: InverseTimestampKey (Shard's row key)
// ---------------------------------------------------------------------------

/// Inverse-timestamp + content-hash row key, used by `Shard`.
///
/// Layout: `reverse_ts_be (8B) || blake3(text)[..hash_len] || optional u64
/// tiebreaker`. Lex byte order = logical newest-first.
#[derive(Debug, Clone, Copy)]
pub struct InverseTimestampKey {
    /// Byte length of the blake3 truncation. 1..=32.
    pub hash_len: usize,
    /// If true, every encoded key carries an 8-byte tiebreaker suffix
    /// supplied at encode time.
    pub with_tiebreaker: bool,
}

impl InverseTimestampKey {
    /// New encoder with the default hash length and no tiebreaker.
    #[must_use]
    pub fn new() -> Self {
        Self {
            hash_len: DEFAULT_HASH_LEN,
            with_tiebreaker: false,
        }
    }

    /// Override the hash-suffix length.
    ///
    /// # Errors
    /// Returns `Invariant` if `hash_len` is outside `1..=32`.
    pub fn with_hash_len(mut self, hash_len: usize) -> Result<Self> {
        if !(1..=32).contains(&hash_len) {
            return Err(Error::Invariant("hash_len must be in 1..=32"));
        }
        self.hash_len = hash_len;
        Ok(self)
    }

    /// Configure this encoder to always require a tiebreaker.
    #[must_use]
    pub fn with_tiebreaker_field(mut self) -> Self {
        self.with_tiebreaker = true;
        self
    }
}

impl Default for InverseTimestampKey {
    fn default() -> Self {
        Self::new()
    }
}

/// Input for [`InverseTimestampKey::encode`].
#[derive(Debug, Clone, Copy)]
pub struct InverseTimestampInput<'a> {
    /// Event timestamp in nanoseconds since the Unix epoch.
    pub ts_nanos: u64,
    /// Original event bytes (the hash is taken over these).
    pub text: &'a [u8],
    /// Optional 8-byte BE tiebreaker. Required when the encoder was
    /// built with `with_tiebreaker_field()`.
    pub tiebreaker: Option<u64>,
}

impl KeyFormat for InverseTimestampKey {
    type Input<'a> = InverseTimestampInput<'a>;

    fn key_len(&self) -> usize {
        8 + self.hash_len + if self.with_tiebreaker { 8 } else { 0 }
    }

    fn encode(&self, input: InverseTimestampInput<'_>) -> RowKey {
        let mut out = Vec::with_capacity(self.key_len());
        let rev = u64::MAX - input.ts_nanos;
        out.extend_from_slice(&rev.to_be_bytes());
        let h = blake3::hash(input.text);
        out.extend_from_slice(&h.as_bytes()[..self.hash_len]);
        if self.with_tiebreaker {
            // The encoder was configured to require a tiebreaker; supply
            // 0 if the caller omitted one (the encoder is the source of
            // truth on layout, not the caller).
            let tb = input.tiebreaker.unwrap_or(0);
            out.extend_from_slice(&tb.to_be_bytes());
        } else if let Some(tb) = input.tiebreaker {
            // Encoder didn't ask for a tiebreaker but caller passed one;
            // honour it for backward-compat with the original KeyBuilder.
            out.extend_from_slice(&tb.to_be_bytes());
        }
        RowKey(out)
    }
}

// ---------------------------------------------------------------------------
// Implementation 2: ContentHashKey (EmbedCache's key)
// ---------------------------------------------------------------------------

/// Content-addressed key used by `EmbedCache`.
///
/// Layout: `blake3(model_id)[..8] || blake3(text)[..text_hash_len]`.
/// Byte order has no meaning; this is a hash-only namespaced lookup
/// key.
#[derive(Debug, Clone, Copy)]
pub struct ContentHashKey {
    /// Precomputed 8-byte prefix from the model id. Acts as a namespace
    /// so entries from different models never collide.
    pub model_id_hash: [u8; 8],
    /// Byte length of the blake3 truncation over the input text. 1..=32.
    pub text_hash_len: usize,
}

impl ContentHashKey {
    /// New encoder for the given `model_id` and text-hash length.
    ///
    /// # Errors
    /// Returns `Invariant` if `text_hash_len` is outside `1..=32`.
    pub fn new(model_id: &str, text_hash_len: usize) -> Result<Self> {
        if !(1..=32).contains(&text_hash_len) {
            return Err(Error::Invariant("text_hash_len must be in 1..=32"));
        }
        let mut model_id_hash = [0u8; 8];
        model_id_hash.copy_from_slice(&blake3::hash(model_id.as_bytes()).as_bytes()[..8]);
        Ok(Self {
            model_id_hash,
            text_hash_len,
        })
    }
}

/// Input for [`ContentHashKey::encode`].
#[derive(Debug, Clone, Copy)]
pub struct ContentHashInput<'a> {
    /// Bytes whose hash forms the suffix of the encoded key.
    pub text: &'a [u8],
}

impl KeyFormat for ContentHashKey {
    type Input<'a> = ContentHashInput<'a>;

    fn key_len(&self) -> usize {
        8 + self.text_hash_len
    }

    fn encode(&self, input: ContentHashInput<'_>) -> RowKey {
        let mut out = Vec::with_capacity(self.key_len());
        out.extend_from_slice(&self.model_id_hash);
        let h = blake3::hash(input.text);
        out.extend_from_slice(&h.as_bytes()[..self.text_hash_len]);
        RowKey(out)
    }
}

// ---------------------------------------------------------------------------
// Legacy ergonomic facade for InverseTimestampKey
// ---------------------------------------------------------------------------

/// Builder for [`RowKey`] in the inverse-timestamp format.
///
/// Kept as an ergonomic façade over [`InverseTimestampKey`]; new code
/// can use the [`KeyFormat`] trait directly, but `KeyBuilder` remains
/// the recommended way to construct ad-hoc keys.
#[derive(Debug, Clone)]
pub struct KeyBuilder {
    ts_nanos: u64,
    text: Vec<u8>,
    hash_len: usize,
    tiebreaker: Option<u64>,
}

impl KeyBuilder {
    /// Start a new key for the given event timestamp (nanoseconds since
    /// the Unix epoch) and the original event text (used to derive the
    /// hash suffix).
    #[must_use]
    pub fn new(ts_nanos: u64, text: impl AsRef<[u8]>) -> Self {
        Self {
            ts_nanos,
            text: text.as_ref().to_vec(),
            hash_len: DEFAULT_HASH_LEN,
            tiebreaker: None,
        }
    }

    /// Override the hash-suffix length.
    ///
    /// # Errors
    /// Returns `Invariant` if `hash_len` is outside `1..=32`.
    pub fn with_hash_len(mut self, hash_len: usize) -> Result<Self> {
        if !(1..=32).contains(&hash_len) {
            return Err(Error::Invariant("hash_len must be in 1..=32"));
        }
        self.hash_len = hash_len;
        Ok(self)
    }

    /// Append an 8-byte BE tiebreaker.
    #[must_use]
    pub fn with_tiebreaker(mut self, tb: u64) -> Self {
        self.tiebreaker = Some(tb);
        self
    }

    /// Encode the key.
    #[must_use]
    pub fn build(self) -> RowKey {
        let fmt = InverseTimestampKey {
            hash_len: self.hash_len,
            with_tiebreaker: false,
        };
        fmt.encode(InverseTimestampInput {
            ts_nanos: self.ts_nanos,
            text: &self.text,
            tiebreaker: self.tiebreaker,
        })
    }
}

/// Decoded view of an [`InverseTimestampKey`]-encoded [`RowKey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyParts<'a> {
    /// Original event timestamp (recovered as `u64::MAX - reverse_ts`).
    pub ts_nanos: u64,
    /// Content-hash bytes (truncated blake3).
    pub hash: &'a [u8],
    /// Optional tiebreaker, if present.
    pub tiebreaker: Option<u64>,
}

impl<'a> KeyParts<'a> {
    /// Parse a key encoded with [`InverseTimestampKey`] (or, equivalently,
    /// [`KeyBuilder`]). `hash_len` must match the length used at build
    /// time. Tiebreaker presence is inferred from the remaining length
    /// (must be exactly 0 or 8).
    ///
    /// # Errors
    /// Returns `BadKey` if the buffer length doesn't match
    /// `8 + hash_len` or `8 + hash_len + 8`.
    pub fn parse(bytes: &'a [u8], hash_len: usize) -> Result<Self> {
        let head = 8 + hash_len;
        if bytes.len() != head && bytes.len() != head + 8 {
            return Err(Error::BadKey("unexpected row-key length"));
        }
        let mut rev = [0u8; 8];
        rev.copy_from_slice(&bytes[..8]);
        let rev = u64::from_be_bytes(rev);
        let ts_nanos = u64::MAX - rev;
        let hash = &bytes[8..head];
        let tiebreaker = if bytes.len() == head + 8 {
            let mut tb = [0u8; 8];
            tb.copy_from_slice(&bytes[head..]);
            Some(u64::from_be_bytes(tb))
        } else {
            None
        };
        Ok(Self {
            ts_nanos,
            hash,
            tiebreaker,
        })
    }
}

/// Build a `[lo, hi)` byte range that bounds keys whose `ts_nanos` lie in
/// `[t_lo, t_hi)`. Returned as `(start, end)` suitable for an ascending
/// lexicographic range scan that yields newest-first.
///
/// `t_lo` inclusive, `t_hi` exclusive.
#[must_use]
pub fn time_range_bytes(t_lo: u64, t_hi: u64) -> ([u8; 8], [u8; 8]) {
    let hi_excl = if t_hi == 0 { 0 } else { t_hi - 1 };
    let start = (u64::MAX - hi_excl).to_be_bytes();
    let end = (u64::MAX - t_lo).to_be_bytes();
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----- KeyBuilder backward-compat -----

    #[test]
    fn round_trip_no_tiebreaker() {
        let k = KeyBuilder::new(1_700_000_000_000_000_000, b"hello").build();
        let parts = KeyParts::parse(k.as_bytes(), DEFAULT_HASH_LEN).unwrap();
        assert_eq!(parts.ts_nanos, 1_700_000_000_000_000_000);
        assert_eq!(parts.hash.len(), DEFAULT_HASH_LEN);
        assert!(parts.tiebreaker.is_none());
    }

    #[test]
    fn round_trip_with_tiebreaker() {
        let k = KeyBuilder::new(42, b"x").with_tiebreaker(7).build();
        let parts = KeyParts::parse(k.as_bytes(), DEFAULT_HASH_LEN).unwrap();
        assert_eq!(parts.ts_nanos, 42);
        assert_eq!(parts.tiebreaker, Some(7));
    }

    #[test]
    fn newer_sorts_first_lexicographically() {
        let older = KeyBuilder::new(100, b"a").build();
        let newer = KeyBuilder::new(200, b"a").build();
        assert!(newer.as_bytes() < older.as_bytes());
    }

    #[test]
    fn equal_ts_distinct_text_distinct_keys() {
        let a = KeyBuilder::new(1, b"foo").build();
        let b = KeyBuilder::new(1, b"bar").build();
        assert_ne!(a, b);
        assert_eq!(&a.as_bytes()[..8], &b.as_bytes()[..8]);
    }

    #[test]
    fn custom_hash_len_validated() {
        assert!(KeyBuilder::new(0, b"x").with_hash_len(0).is_err());
        assert!(KeyBuilder::new(0, b"x").with_hash_len(33).is_err());
        assert!(KeyBuilder::new(0, b"x").with_hash_len(1).is_ok());
        assert!(KeyBuilder::new(0, b"x").with_hash_len(32).is_ok());
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(KeyParts::parse(&[0u8; 7], DEFAULT_HASH_LEN).is_err());
        assert!(KeyParts::parse(&[0u8; 8 + DEFAULT_HASH_LEN + 1], DEFAULT_HASH_LEN).is_err());
    }

    // ----- KeyFormat trait + InverseTimestampKey -----

    #[test]
    fn inverse_timestamp_key_matches_keybuilder_byte_for_byte() {
        let fmt = InverseTimestampKey::new();
        let via_trait = fmt.encode(InverseTimestampInput {
            ts_nanos: 1_700_000_000_123_456_789,
            text: b"hello world",
            tiebreaker: None,
        });
        let via_builder = KeyBuilder::new(1_700_000_000_123_456_789, b"hello world").build();
        assert_eq!(via_trait, via_builder);
        assert_eq!(via_trait.as_bytes().len(), fmt.key_len());
    }

    #[test]
    fn inverse_timestamp_key_with_tiebreaker_matches_keybuilder() {
        let fmt = InverseTimestampKey::new();
        let via_trait = fmt.encode(InverseTimestampInput {
            ts_nanos: 99,
            text: b"x",
            tiebreaker: Some(7),
        });
        let via_builder = KeyBuilder::new(99, b"x").with_tiebreaker(7).build();
        assert_eq!(via_trait, via_builder);
    }

    #[test]
    fn inverse_timestamp_required_tiebreaker_defaults_to_zero() {
        let fmt = InverseTimestampKey::new().with_tiebreaker_field();
        let with_explicit = fmt.encode(InverseTimestampInput {
            ts_nanos: 1,
            text: b"x",
            tiebreaker: Some(0),
        });
        let without_explicit = fmt.encode(InverseTimestampInput {
            ts_nanos: 1,
            text: b"x",
            tiebreaker: None,
        });
        assert_eq!(with_explicit, without_explicit);
        assert_eq!(with_explicit.as_bytes().len(), fmt.key_len());
        assert_eq!(fmt.key_len(), 8 + DEFAULT_HASH_LEN + 8);
    }

    // ----- ContentHashKey -----

    #[test]
    fn content_hash_key_layout() {
        let fmt = ContentHashKey::new("test-model", 16).unwrap();
        let k = fmt.encode(ContentHashInput { text: b"hello" });
        assert_eq!(k.as_bytes().len(), 8 + 16);
        // The first 8 bytes are blake3("test-model")[..8].
        let mut expected_prefix = [0u8; 8];
        expected_prefix.copy_from_slice(&blake3::hash(b"test-model").as_bytes()[..8]);
        assert_eq!(&k.as_bytes()[..8], &expected_prefix);
        // The next 16 bytes are blake3("hello")[..16].
        let mut expected_hash = [0u8; 16];
        expected_hash.copy_from_slice(&blake3::hash(b"hello").as_bytes()[..16]);
        assert_eq!(&k.as_bytes()[8..], &expected_hash);
    }

    #[test]
    fn content_hash_key_namespaces_isolate() {
        let fmt_a = ContentHashKey::new("model-a", 16).unwrap();
        let fmt_b = ContentHashKey::new("model-b", 16).unwrap();
        let k_a = fmt_a.encode(ContentHashInput { text: b"same text" });
        let k_b = fmt_b.encode(ContentHashInput { text: b"same text" });
        assert_ne!(
            k_a, k_b,
            "same text under different model_id must produce distinct keys"
        );
        // Suffix (text hash) is identical, only the prefix differs.
        assert_eq!(&k_a.as_bytes()[8..], &k_b.as_bytes()[8..]);
    }

    #[test]
    fn content_hash_key_validates_hash_len() {
        assert!(ContentHashKey::new("m", 0).is_err());
        assert!(ContentHashKey::new("m", 33).is_err());
        assert!(ContentHashKey::new("m", 1).is_ok());
        assert!(ContentHashKey::new("m", 32).is_ok());
    }

    // ----- Custom 3rd impl proves pluggability -----

    #[test]
    fn pluggable_third_key_format() {
        /// 4-byte big-endian forward-timestamp key. Toy example to prove
        /// the trait is genuinely pluggable.
        struct ForwardTsKey;
        struct ForwardTsInput {
            ts_secs: u32,
        }
        impl KeyFormat for ForwardTsKey {
            type Input<'a> = ForwardTsInput;
            fn key_len(&self) -> usize {
                4
            }
            fn encode(&self, input: ForwardTsInput) -> RowKey {
                RowKey::from_bytes_unchecked(input.ts_secs.to_be_bytes())
            }
        }

        let fmt = ForwardTsKey;
        let early = fmt.encode(ForwardTsInput { ts_secs: 100 });
        let late = fmt.encode(ForwardTsInput { ts_secs: 200 });
        // Forward order: earlier sorts first.
        assert!(early.as_bytes() < late.as_bytes());
        assert_eq!(early.as_bytes().len(), fmt.key_len());
    }
}
