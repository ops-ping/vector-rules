//! Content-addressed embedding cache backed by a dedicated fjall keyspace.
//!
//! See [`docs/design.md`](https://github.com/ops-ping/em-log-n/blob/main/docs/design.md)
//! "Embedding cache" section for the rationale.
//!
//! ## Why a cache
//!
//! Embedding is ~150× slower than the rest of the storage layer
//! (see [`docs/benchmarks.md`]). Many real workloads embed the same
//! string repeatedly — recurring log lines, identical prompts, error
//! messages. Caching by content hash turns those repeats into
//! microsecond lookups.
//!
//! ## Key + value layout
//!
//! Cache key (fixed-layout big-endian bytes; no varints so range scans
//! by namespace work):
//!
//! ```text
//! | 8 bytes: blake3(model_id)[..8] | 16 bytes: blake3(text)[..16] |
//! ```
//!
//! Cache value:
//!
//! ```text
//! | 4 bytes BE: dim | 4 * dim bytes: f32 vector (lossless) |
//! ```
//!
//! The `dim` prefix lets the loader notice a mismatch (e.g. someone
//! changed the model under the same `model_id` — should not happen, but
//! we degrade to a cache miss rather than serving a wrong-shape vector).
//!
//! ## Concurrency
//!
//! Two writers racing on the same uncached text may both call the
//! inner embedder. That's fine — both produce identical bytes (the
//! embedder is deterministic and so is the f32 encoding), so the second
//! `put` is an idempotent overwrite. Single-flight dedup is explicitly
//! out of scope; if it becomes a measured cost it can land later
//! without breaking the API.

#![cfg(feature = "fjall-backend")]

use std::path::Path;
use std::sync::Arc;

use fjall::{Database, Keyspace, KeyspaceCreateOptions};

use crate::embed::Embedder;
use crate::error::{Error, Result};
use crate::key::{ContentHashInput, ContentHashKey, KeyFormat};
use crate::retention::{KeepAll, RetentionPolicy};

const VALUE_DIM_PREFIX: usize = 4;

/// Content-addressed embedding cache.
pub struct EmbedCache {
    #[allow(dead_code)] // retained for keyspace lifetime
    db: Database,
    keyspace: Keyspace,
    key_fmt: ContentHashKey,
    dim: usize,
    #[allow(dead_code)] // retention is wired into compaction-filter-factory in a follow-up
    retention: Arc<dyn RetentionPolicy>,
}

impl EmbedCache {
    /// Open (or create) an embedding cache at `base_dir`.
    ///
    /// - `model_id` identifies the embedder; cache entries from
    ///   different model ids are namespaced and cannot collide.
    /// - `dim` is the embedder's output dimension; entries with a
    ///   different stored dim are treated as misses.
    ///
    /// # Errors
    /// Surfaces fjall open errors.
    pub fn open(
        base_dir: impl AsRef<Path>,
        model_id: impl Into<String>,
        dim: usize,
    ) -> Result<Self> {
        Self::open_inner(base_dir, model_id.into(), dim)
    }

    /// Open a cache whose namespace is additionally bound to a canonicalization
    /// strategy. Use this for a [`CanonicalizingEmbedder`] so that entries
    /// produced under one canonicalizer (or one canonicalizer *version*) can
    /// never be served to another — a rule change yields a clean miss instead
    /// of a vector computed under the old rules.
    ///
    /// `canon_ns` is typically [`CanonicalizingEmbedder::cache_namespace`].
    ///
    /// # Errors
    /// Surfaces fjall open errors; `Invariant` if `dim == 0`.
    pub fn open_namespaced(
        base_dir: impl AsRef<Path>,
        model_id: impl AsRef<str>,
        canon_ns: impl AsRef<str>,
        dim: usize,
    ) -> Result<Self> {
        // U+001F (unit separator) can't appear in a model id or canon id, so the
        // composed namespace is unambiguous.
        let composed = format!("{}\u{1f}{}", model_id.as_ref(), canon_ns.as_ref());
        Self::open_inner(base_dir, composed, dim)
    }

    fn open_inner(base_dir: impl AsRef<Path>, model_id: String, dim: usize) -> Result<Self> {
        if dim == 0 {
            return Err(Error::Invariant("embed cache dim must be > 0"));
        }
        let db = Database::builder(base_dir.as_ref())
            .open()
            .map_err(|e| Error::KvBackend(format!("open cache db: {e}")))?;
        let keyspace = db
            .keyspace("embed_cache", KeyspaceCreateOptions::default)
            .map_err(|e| Error::KvBackend(format!("open embed_cache keyspace: {e}")))?;
        let key_fmt = ContentHashKey::new(&model_id, 16)?;
        Ok(Self {
            db,
            keyspace,
            key_fmt,
            dim,
            retention: Arc::new(KeepAll),
        })
    }

    /// Swap in a custom retention policy. Default is [`KeepAll`].
    #[must_use]
    pub fn with_retention(mut self, policy: Arc<dyn RetentionPolicy>) -> Self {
        self.retention = policy;
        self
    }

    /// Embedder dimension this cache was opened against.
    #[must_use]
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Look up a cached vector. Returns `None` on miss OR if a stored
    /// entry has a mismatched `dim` (in which case the entry is treated
    /// as missing and will be overwritten by the next `put`).
    ///
    /// # Errors
    /// Surfaces fjall errors.
    pub fn get(&self, text: &str) -> Result<Option<Vec<f32>>> {
        let key = self.key_fmt.encode(ContentHashInput {
            text: text.as_bytes(),
        });
        let Some(raw) = self
            .keyspace
            .get(key.as_bytes())
            .map_err(|e| Error::KvBackend(format!("cache get: {e}")))?
        else {
            return Ok(None);
        };
        decode_value(raw.as_ref(), self.dim)
    }

    /// Insert (or overwrite) a vector. Vector length must equal `self.dim()`.
    ///
    /// # Errors
    /// `Invariant` for dim mismatch; surfaces fjall errors.
    pub fn put(&self, text: &str, vector: &[f32]) -> Result<()> {
        if vector.len() != self.dim {
            return Err(Error::Invariant("vector dim mismatch in EmbedCache::put"));
        }
        let key = self.key_fmt.encode(ContentHashInput {
            text: text.as_bytes(),
        });
        let value = encode_value(vector);
        self.keyspace
            .insert(key.as_bytes(), value)
            .map_err(|e| Error::KvBackend(format!("cache insert: {e}")))?;
        Ok(())
    }
}

/// Drop-in [`Embedder`] wrapper that consults an [`EmbedCache`] first
/// and only falls through to the inner embedder on miss.
///
/// Cache writes are **best-effort**: a failure to insert into the cache
/// is logged-via-return-error-from-`put` but does NOT fail `embed`. The
/// caller still gets the freshly-computed vector.
pub struct CachingEmbedder<E: Embedder> {
    inner: E,
    cache: EmbedCache,
}

impl<E: Embedder> CachingEmbedder<E> {
    /// Wrap `inner` with `cache`. The two MUST report the same `dim()`;
    /// otherwise this is a programming error caught at construction.
    ///
    /// # Errors
    /// `Invariant` if `inner.dim() != cache.dim()`.
    pub fn new(inner: E, cache: EmbedCache) -> Result<Self> {
        if inner.dim() != cache.dim() {
            return Err(Error::Invariant("CachingEmbedder dim mismatch"));
        }
        Ok(Self { inner, cache })
    }

    /// Borrow the underlying cache (e.g. to swap retention policy).
    pub fn cache(&self) -> &EmbedCache {
        &self.cache
    }
}

impl<E: Embedder> Embedder for CachingEmbedder<E> {
    fn dim(&self) -> usize {
        self.inner.dim()
    }

    fn model_id(&self) -> crate::embed::ModelId {
        self.inner.model_id()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if let Some(v) = self.cache.get(text)? {
            return Ok(v);
        }
        let v = self.inner.embed(text)?;
        // Best-effort cache write — never fail the embed because the
        // cache write failed.
        let _ = self.cache.put(text, &v);
        Ok(v)
    }

    /// Cache-aware batch: look up every item in the [`EmbedCache`], forward
    /// **only the misses** to the inner embedder in a single
    /// [`Embedder::embed_batch`] call, write them back, and reassemble the
    /// results in caller order. A mostly-cached batch therefore costs ~1 µs
    /// per hit plus one packed forward pass for the few misses.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        // Pre-fill results with cache hits; record indices + texts of misses.
        let mut results: Vec<Option<Vec<f32>>> = Vec::with_capacity(texts.len());
        let mut miss_idx: Vec<usize> = Vec::new();
        let mut miss_text: Vec<&str> = Vec::new();
        for (i, t) in texts.iter().enumerate() {
            match self.cache.get(t)? {
                Some(v) => results.push(Some(v)),
                None => {
                    results.push(None);
                    miss_idx.push(i);
                    miss_text.push(t);
                }
            }
        }

        if !miss_text.is_empty() {
            let embedded = self.inner.embed_batch(&miss_text)?;
            if embedded.len() != miss_idx.len() {
                return Err(Error::Embed(
                    "inner embed_batch returned wrong count".into(),
                ));
            }
            for (&i, v) in miss_idx.iter().zip(embedded.into_iter()) {
                let _ = self.cache.put(texts[i], &v);
                results[i] = Some(v);
            }
        }

        Ok(results
            .into_iter()
            .map(|o| o.expect("every slot filled (hit or miss)"))
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Lossless f32 pack / unpack
// ---------------------------------------------------------------------------

/// Encode an f32 vector as `[dim:u32 BE | f32 bytes BE]`.
///
/// Lossless — the full IEEE-754 f32 is stored, never quantized: a lossy pack
/// would be irreversible and would corrupt chained `v:add`/`v:sub` vector
/// arithmetic.
fn encode_value(v: &[f32]) -> Vec<u8> {
    let dim = v.len();
    let mut out = Vec::with_capacity(VALUE_DIM_PREFIX + 4 * dim);
    out.extend_from_slice(&(dim as u32).to_be_bytes());
    for &x in v {
        out.extend_from_slice(&x.to_bits().to_be_bytes());
    }
    out
}

fn decode_value(bytes: &[u8], expected_dim: usize) -> Result<Option<Vec<f32>>> {
    if bytes.len() < VALUE_DIM_PREFIX {
        return Err(Error::Codec("cache value truncated at dim prefix".into()));
    }
    let mut dim_buf = [0u8; 4];
    dim_buf.copy_from_slice(&bytes[..VALUE_DIM_PREFIX]);
    let stored_dim = u32::from_be_bytes(dim_buf) as usize;
    if stored_dim != expected_dim {
        // Treat as a miss — caller will overwrite via put. An entry whose stored
        // length disagrees with the current dim thus falls out of use.
        return Ok(None);
    }
    let needed = VALUE_DIM_PREFIX + 4 * stored_dim;
    if bytes.len() != needed {
        return Err(Error::Codec(
            "cache value length doesn't match stored dim".into(),
        ));
    }
    let mut out = Vec::with_capacity(stored_dim);
    for chunk in bytes[VALUE_DIM_PREFIX..].chunks_exact(4) {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(chunk);
        out.push(f32::from_bits(u32::from_be_bytes(buf)));
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Embedder that returns a deterministic vector derived from text
    /// and counts how many times `embed` was called.
    struct CountingEmbedder {
        dim: usize,
        calls: AtomicUsize,
    }
    impl CountingEmbedder {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                calls: AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Relaxed)
        }
    }
    impl Embedder for CountingEmbedder {
        fn dim(&self) -> usize {
            self.dim
        }
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let mut v = vec![0.0f32; self.dim];
            for (i, b) in text.bytes().take(self.dim).enumerate() {
                v[i] = (b as f32) / 255.0;
            }
            Ok(v)
        }
    }

    #[test]
    fn f32_round_trip_is_bit_exact() {
        // Arbitrary values — including ones a lossy pack would mangle — must
        // round-trip bit-for-bit, proving the cache is lossless.
        let inputs: Vec<f32> = vec![
            0.0,
            -0.0,
            1.0,
            -1.0,
            1.0 / 3.0,
            std::f32::consts::PI,
            0.1,
            1e-20,
            1e20,
        ];
        let bytes = encode_value(&inputs);
        let out = decode_value(&bytes, inputs.len()).unwrap().unwrap();
        for (a, b) in inputs.iter().zip(out.iter()) {
            assert_eq!(
                a.to_bits(),
                b.to_bits(),
                "{a} should round-trip bit-exact, got {b}"
            );
        }
    }

    #[test]
    fn f32_record_is_fixed_width() {
        let inputs: Vec<f32> = (0..16).map(|i| (i as f32) * 0.123_456_7).collect();
        let bytes = encode_value(&inputs);
        assert_eq!(bytes.len(), VALUE_DIM_PREFIX + 4 * inputs.len());
        let out = decode_value(&bytes, inputs.len()).unwrap().unwrap();
        assert_eq!(out.len(), inputs.len());
        for (a, b) in inputs.iter().zip(out.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn decode_dim_mismatch_returns_none() {
        let inputs = vec![1.0f32, 2.0, 3.0];
        let bytes = encode_value(&inputs);
        // Ask for a different dim → cache treats as miss, not as error.
        let out = decode_value(&bytes, 5).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn decode_truncated_value_is_error() {
        let mut bytes = encode_value(&[1.0f32, 2.0]);
        bytes.truncate(bytes.len() - 1);
        assert!(decode_value(&bytes, 2).is_err());
    }

    #[test]
    fn hit_miss_round_trip_against_fjall() {
        let tmp = tempfile::tempdir().unwrap();
        let dim = 8;
        let cache = EmbedCache::open(tmp.path(), "test-model", dim).unwrap();

        assert!(cache.get("hello").unwrap().is_none());
        // A value a lossy pack would mangle — the cache must return it bit-for-bit.
        let v = vec![0.1f32; dim];
        cache.put("hello", &v).unwrap();
        let got = cache.get("hello").unwrap().unwrap();
        for (a, b) in v.iter().zip(got.iter()) {
            assert_eq!(a.to_bits(), b.to_bits());
        }
    }

    #[test]
    fn put_rejects_dim_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = EmbedCache::open(tmp.path(), "m", 4).unwrap();
        assert!(cache.put("x", &[0.0; 3]).is_err());
        assert!(cache.put("x", &[0.0; 5]).is_err());
    }

    #[test]
    fn model_id_namespaces_isolate_entries() {
        let tmp = tempfile::tempdir().unwrap();
        const DIM: usize = 4;
        let cache_a = EmbedCache::open(tmp.path(), "model-a", DIM).unwrap();
        // Both caches share the same fjall directory but use distinct
        // model_id namespaces inside the keyspace.
        cache_a.put("x", &[1.0; DIM]).unwrap();
        drop(cache_a);
        let cache_b = EmbedCache::open(tmp.path(), "model-b", DIM).unwrap();
        assert!(
            cache_b.get("x").unwrap().is_none(),
            "model-b must not see model-a entries"
        );
        // Re-open model-a and confirm the entry is still there.
        drop(cache_b);
        let cache_a2 = EmbedCache::open(tmp.path(), "model-a", DIM).unwrap();
        assert!(cache_a2.get("x").unwrap().is_some());
    }

    #[test]
    fn canon_namespace_isolates_and_differs_from_plain_open() {
        let tmp = tempfile::tempdir().unwrap();
        const DIM: usize = 4;
        // Same model_id, two different canonicalizer namespaces → isolated.
        let c_v1 = EmbedCache::open_namespaced(tmp.path(), "m", "log-mask/v1", DIM).unwrap();
        c_v1.put("x", &[1.0; DIM]).unwrap();
        drop(c_v1);
        let c_v2 = EmbedCache::open_namespaced(tmp.path(), "m", "log-mask/v2", DIM).unwrap();
        assert!(
            c_v2.get("x").unwrap().is_none(),
            "a canon-version bump must miss, not serve stale vectors"
        );
        // And a plain open under the same model_id is its own namespace too.
        drop(c_v2);
        let plain = EmbedCache::open(tmp.path(), "m", DIM).unwrap();
        assert!(plain.get("x").unwrap().is_none());
        // Re-opening the original namespace still sees the entry.
        drop(plain);
        let c_v1b = EmbedCache::open_namespaced(tmp.path(), "m", "log-mask/v1", DIM).unwrap();
        assert!(c_v1b.get("x").unwrap().is_some());
    }

    #[test]
    fn caching_embedder_hits_after_first_call() {
        let tmp = tempfile::tempdir().unwrap();
        let dim = 16;
        let inner = CountingEmbedder::new(dim);
        let cache = EmbedCache::open(tmp.path(), "test", dim).unwrap();
        let wrapped = CachingEmbedder::new(inner, cache).unwrap();

        let _ = wrapped.embed("hello").unwrap();
        assert_eq!(wrapped.inner.calls(), 1, "first call is a miss");
        let _ = wrapped.embed("hello").unwrap();
        assert_eq!(wrapped.inner.calls(), 1, "second call must hit cache");
        let _ = wrapped.embed("world").unwrap();
        assert_eq!(wrapped.inner.calls(), 2, "different text re-embeds");
        let _ = wrapped.embed("hello").unwrap();
        assert_eq!(wrapped.inner.calls(), 2, "third hello still a hit");
    }

    #[test]
    fn caching_embedder_rejects_dim_mismatch_at_construction() {
        let tmp = tempfile::tempdir().unwrap();
        let inner = CountingEmbedder::new(8);
        let cache = EmbedCache::open(tmp.path(), "test", 16).unwrap();
        assert!(CachingEmbedder::new(inner, cache).is_err());
    }

    #[test]
    fn caching_embedder_batch_only_forwards_misses_and_preserves_order() {
        let tmp = tempfile::tempdir().unwrap();
        let dim = 16;
        let inner = CountingEmbedder::new(dim);
        let cache = EmbedCache::open(tmp.path(), "test", dim).unwrap();
        let wrapped = CachingEmbedder::new(inner, cache).unwrap();

        // Pre-warm "b" and "d" so they're cache hits.
        let _ = wrapped.embed("b").unwrap();
        let _ = wrapped.embed("d").unwrap();
        assert_eq!(wrapped.inner.calls(), 2);

        // Batch of 5: b, d are hits; a, c, e are misses.
        let batch = ["a", "b", "c", "d", "e"];
        let out = wrapped.embed_batch(&batch).unwrap();

        // Only the 3 misses were forwarded to the inner embedder.
        assert_eq!(wrapped.inner.calls(), 2 + 3, "only misses forwarded");
        assert_eq!(out.len(), 5);

        // Order preserved: each result equals a direct embed of that text.
        for (i, t) in batch.iter().enumerate() {
            let direct = CountingEmbedder::new(dim).embed(t).unwrap();
            for (a, b) in out[i].iter().zip(direct.iter()) {
                assert!((a - b).abs() <= 1.0 / 128.0, "mismatch at {i}:{t}");
            }
        }

        // A fully-cached re-run forwards nothing.
        let calls_before = wrapped.inner.calls();
        let _ = wrapped.embed_batch(&batch).unwrap();
        assert_eq!(
            wrapped.inner.calls(),
            calls_before,
            "all cached, no forwards"
        );
    }

    #[test]
    fn null_embedder_default_batch_impl() {
        let e = crate::embed::NullEmbedder { dim: 4 };
        let out = e.embed_batch(&["x", "y", "z"]).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|v| v == &vec![0.0; 4]));
        assert!(e.embed_batch(&[]).unwrap().is_empty());
    }
}
