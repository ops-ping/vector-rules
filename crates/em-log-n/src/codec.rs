//! Pluggable value codec.
//!
//! Two built-in codecs ship behind cargo features:
//!
//! - `codec-prost` — protobuf via `prost`. Stable wire format, language-
//!   portable, schema files survive code changes. **Default for cold-tier
//!   value storage.** Schema is declared in-tree with the `prost::Message`
//!   derive macro (no `protoc` dependency); the field tag numbers ARE the
//!   schema and are the only thing that must stay stable over time.
//! - `codec-rkyv` — zero-copy archive via `rkyv`. Hot-path-friendly
//!   (µs-class decode, lets a reader inspect fields without copying the
//!   bytes), Rust-only, format tied to rkyv version. Used by latency-
//!   critical readers (e.g. in-stream PII detection).
//!
//! Callers may plug a third codec by implementing [`Codec`] directly.
//!
//! Schema-evolution rules for the prost default (enforced by review, not by
//! the compiler):
//!
//! - Adding fields: always OK (assign a fresh, never-before-used tag number).
//! - Renaming fields: OK — wire format depends on tag number, not name.
//! - Changing a field's type: NOT OK. Deprecate the old tag and add a new one.
//! - Reusing a tag number after a field is removed: NOT OK.

use std::collections::BTreeMap;

use crate::error::Result;

/// Encode/decode for typed values stored in fjall.
///
/// Implementations must be deterministic (`encode` of equal `T` produces equal
/// bytes) so content hashes and round-trip tests are stable.
pub trait Codec {
    /// The logical value type carried over the wire.
    type Value;

    /// Encode a value to bytes.
    ///
    /// # Errors
    /// Returns a `Codec` error if the value can't be serialized.
    fn encode(value: &Self::Value) -> Result<Vec<u8>>;

    /// Decode bytes to a value.
    ///
    /// # Errors
    /// Returns a `Codec` error if the bytes are malformed.
    fn decode(bytes: &[u8]) -> Result<Self::Value>;
}

/// The canonical record stored per row.
///
/// This is the **logical** shape callers see; on-disk wire formats are owned
/// by individual codec impls and can differ in field order, optionality, etc.
/// All fields are public because the type is a pure data carrier.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogValue {
    /// Original event text. The reverse-ts row key was derived from this.
    pub text: String,
    /// Event timestamp in nanoseconds since Unix epoch.
    pub ts_nanos: u64,
    /// Originating domain (e.g. `system-core`, `ui`, `llm-traces`). Carried
    /// in the value (not the key) so domain-scoped scans use partition
    /// iteration rather than prefix matching.
    pub domain: String,
    /// Free-form string tags. `BTreeMap` so encoding is deterministic.
    pub tags: BTreeMap<String, String>,
    /// Names of usearch indexes this row contributes a vector to (e.g.
    /// `["text", "fused"]`). The actual vectors live in the indexes, not
    /// here.
    pub embedding_ids: Vec<String>,
    /// Optional opaque payload (e.g. a serialized tool-use trace, a binary
    /// PII-redaction mask). Codec-agnostic.
    pub metadata: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Prost (protobuf) codec — default for cold-tier value storage.
// ---------------------------------------------------------------------------

#[cfg(feature = "codec-prost")]
pub use prost_codec::ProstCodec;

#[cfg(feature = "codec-prost")]
mod prost_codec {
    use super::{Codec, LogValue, Result};
    use crate::error::Error;
    use prost::Message;
    use std::collections::BTreeMap;

    /// Wire schema for [`LogValue`]. **Tag numbers are the schema.** Never
    /// reuse a tag after deprecation; never change a tag's type. Adding new
    /// tags is always safe.
    #[derive(Clone, PartialEq, Message)]
    struct LogValueWire {
        #[prost(string, tag = "1")]
        text: String,
        #[prost(uint64, tag = "2")]
        ts_nanos: u64,
        #[prost(string, tag = "3")]
        domain: String,
        // Encoded as a repeated TagEntry message rather than `map<string,string>`
        // so order is deterministic at encode time (BTreeMap iteration).
        #[prost(message, repeated, tag = "4")]
        tags: Vec<TagEntry>,
        #[prost(string, repeated, tag = "5")]
        embedding_ids: Vec<String>,
        #[prost(bytes = "vec", tag = "6")]
        metadata: Vec<u8>,
    }

    #[derive(Clone, PartialEq, Message)]
    struct TagEntry {
        #[prost(string, tag = "1")]
        k: String,
        #[prost(string, tag = "2")]
        v: String,
    }

    impl From<&LogValue> for LogValueWire {
        fn from(v: &LogValue) -> Self {
            Self {
                text: v.text.clone(),
                ts_nanos: v.ts_nanos,
                domain: v.domain.clone(),
                tags: v
                    .tags
                    .iter()
                    .map(|(k, v)| TagEntry {
                        k: k.clone(),
                        v: v.clone(),
                    })
                    .collect(),
                embedding_ids: v.embedding_ids.clone(),
                metadata: v.metadata.clone(),
            }
        }
    }

    impl From<LogValueWire> for LogValue {
        fn from(w: LogValueWire) -> Self {
            let mut tags = BTreeMap::new();
            for t in w.tags {
                tags.insert(t.k, t.v);
            }
            Self {
                text: w.text,
                ts_nanos: w.ts_nanos,
                domain: w.domain,
                tags,
                embedding_ids: w.embedding_ids,
                metadata: w.metadata,
            }
        }
    }

    /// Protobuf-backed [`Codec`].
    pub struct ProstCodec;

    impl Codec for ProstCodec {
        type Value = LogValue;

        fn encode(value: &LogValue) -> Result<Vec<u8>> {
            let wire = LogValueWire::from(value);
            let mut buf = Vec::with_capacity(wire.encoded_len());
            wire.encode(&mut buf)
                .map_err(|e| Error::Codec(format!("prost encode: {e}")))?;
            Ok(buf)
        }

        fn decode(bytes: &[u8]) -> Result<LogValue> {
            let wire = LogValueWire::decode(bytes)
                .map_err(|e| Error::Codec(format!("prost decode: {e}")))?;
            Ok(wire.into())
        }
    }
}

// ---------------------------------------------------------------------------
// Rkyv (zero-copy archive) codec — hot-path-friendly.
// ---------------------------------------------------------------------------

#[cfg(feature = "codec-rkyv")]
pub use rkyv_codec::RkyvCodec;

#[cfg(feature = "codec-rkyv")]
mod rkyv_codec {
    use super::{Codec, LogValue, Result};
    use crate::error::Error;
    use rkyv::rancor::Error as RkyvError;
    use rkyv::util::AlignedVec;
    use std::collections::BTreeMap;

    #[derive(Clone, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
    struct LogValueRkyv {
        text: String,
        ts_nanos: u64,
        domain: String,
        tags: Vec<(String, String)>,
        embedding_ids: Vec<String>,
        metadata: Vec<u8>,
    }

    impl From<&LogValue> for LogValueRkyv {
        fn from(v: &LogValue) -> Self {
            Self {
                text: v.text.clone(),
                ts_nanos: v.ts_nanos,
                domain: v.domain.clone(),
                tags: v.tags.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                embedding_ids: v.embedding_ids.clone(),
                metadata: v.metadata.clone(),
            }
        }
    }

    impl From<LogValueRkyv> for LogValue {
        fn from(r: LogValueRkyv) -> Self {
            let mut tags = BTreeMap::new();
            for (k, v) in r.tags {
                tags.insert(k, v);
            }
            Self {
                text: r.text,
                ts_nanos: r.ts_nanos,
                domain: r.domain,
                tags,
                embedding_ids: r.embedding_ids,
                metadata: r.metadata,
            }
        }
    }

    /// Rkyv-backed [`Codec`]. Cheap encode, very fast decode.
    ///
    /// (Zero-copy archived access via `rkyv::access` is possible but requires
    /// the `bytecheck` validation path, which we leave to a follow-up
    /// hot-path PR — current API just round-trips to a fully-owned
    /// [`LogValue`].)
    pub struct RkyvCodec;

    impl Codec for RkyvCodec {
        type Value = LogValue;

        fn encode(value: &LogValue) -> Result<Vec<u8>> {
            let r = LogValueRkyv::from(value);
            let bytes: AlignedVec = rkyv::to_bytes::<RkyvError>(&r)
                .map_err(|e| Error::Codec(format!("rkyv encode: {e}")))?;
            Ok(bytes.into_vec())
        }

        fn decode(bytes: &[u8]) -> Result<LogValue> {
            let r: LogValueRkyv = rkyv::from_bytes::<LogValueRkyv, RkyvError>(bytes)
                .map_err(|e| Error::Codec(format!("rkyv decode: {e}")))?;
            Ok(r.into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;

    /// A trivial in-tree codec used to prove the trait is genuinely pluggable
    /// (i.e. callers can ship their own without depending on prost or rkyv).
    struct PassthroughCodec;

    impl Codec for PassthroughCodec {
        type Value = Vec<u8>;
        fn encode(value: &Vec<u8>) -> Result<Vec<u8>> {
            Ok(value.clone())
        }
        fn decode(bytes: &[u8]) -> Result<Vec<u8>> {
            Ok(bytes.to_vec())
        }
    }

    #[test]
    fn pluggable_third_codec_round_trip() {
        let v = b"hello".to_vec();
        let bytes = PassthroughCodec::encode(&v).unwrap();
        let back = PassthroughCodec::decode(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn codec_error_path_is_reachable() {
        // Compile-time sanity: an impl can return a Codec error.
        struct AlwaysFail;
        impl Codec for AlwaysFail {
            type Value = ();
            fn encode(_: &()) -> Result<Vec<u8>> {
                Err(Error::Codec("nope".into()))
            }
            fn decode(_: &[u8]) -> Result<()> {
                Err(Error::Codec("nope".into()))
            }
        }
        assert!(AlwaysFail::encode(&()).is_err());
    }

    fn sample_value() -> LogValue {
        let mut tags = BTreeMap::new();
        tags.insert("trace_id".into(), "abc-123".into());
        tags.insert("severity".into(), "info".into());
        LogValue {
            text: "user clicked button".into(),
            ts_nanos: 1_700_000_000_123_456_789,
            domain: "ui".into(),
            tags,
            embedding_ids: vec!["text".into(), "fused".into()],
            metadata: vec![0xDE, 0xAD, 0xBE, 0xEF],
        }
    }

    #[cfg(feature = "codec-prost")]
    #[test]
    fn prost_round_trip() {
        let v = sample_value();
        let bytes = ProstCodec::encode(&v).unwrap();
        let back = ProstCodec::decode(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[cfg(feature = "codec-prost")]
    #[test]
    fn prost_is_deterministic() {
        let v = sample_value();
        let a = ProstCodec::encode(&v).unwrap();
        let b = ProstCodec::encode(&v).unwrap();
        assert_eq!(
            a, b,
            "prost encode must be deterministic for content hashing"
        );
    }

    #[cfg(feature = "codec-prost")]
    #[test]
    fn prost_default_value_round_trip() {
        let v = LogValue::default();
        let bytes = ProstCodec::encode(&v).unwrap();
        let back = ProstCodec::decode(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[cfg(feature = "codec-rkyv")]
    #[test]
    fn rkyv_round_trip() {
        let v = sample_value();
        let bytes = RkyvCodec::encode(&v).unwrap();
        let back = RkyvCodec::decode(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[cfg(feature = "codec-rkyv")]
    #[test]
    fn rkyv_is_deterministic() {
        let v = sample_value();
        let a = RkyvCodec::encode(&v).unwrap();
        let b = RkyvCodec::encode(&v).unwrap();
        assert_eq!(
            a, b,
            "rkyv encode must be deterministic for content hashing"
        );
    }
}
