//! Embedding backend trait + implementations.
//!
//! The trait is sync because the hot-path constraint forbids imposing an
//! async runtime on callers like vrules / PII detection. Implementations may
//! use internal parallelism but `embed` returns a finished vector.
//!
//! The runtime host supplies the real wllama WASI embedding component. The
//! content-addressed [`EmbedCache`](crate::embed_cache::EmbedCache) absorbs
//! repeats so this layer only ever serves genuine misses.

use crate::error::{Error, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Maps a text input to a fixed-dimension f32 vector.
pub trait Embedder: Send + Sync {
    /// Output dimension of the configured embedding model.
    fn dim(&self) -> usize;

    /// Embed a single input.
    ///
    /// # Errors
    /// Returns an `Embed` error if the backend fails (tokenization, decode,
    /// model load, etc).
    fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Embed a batch of inputs, returning vectors in the same order as
    /// `texts`. The default implementation loops [`Embedder::embed`];
    /// implementations that can pack multiple inputs into one forward pass
    /// with native batch support override this for throughput.
    ///
    /// # Errors
    /// Returns an `Embed` error if any input fails.
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// Stable identity of the underlying model: name + content digest + dim.
    /// Cache keys fold this in so a model change can never serve a stale vector.
    ///
    /// The default is a non-identifying placeholder (`"unspecified"`); the
    /// production host adapter overrides it with the GGUF's content digest, and
    /// wrappers delegate to their inner embedder.
    fn model_id(&self) -> ModelId {
        ModelId::unspecified(self.dim())
    }
}

/// Stable identity of an embedding model — the thing a cache must key on so a
/// model swap (or a re-quantized file of the same name) can never serve a vector
/// computed by a different model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ModelId {
    /// Human-readable name (typically the model file stem).
    pub name: String,
    /// Content digest of the model file's bytes.
    pub digest: [u8; 32],
    /// Output dimension.
    pub dim: usize,
}

impl ModelId {
    /// Placeholder identity for non-production embedders (null/test). NOT a real
    /// model digest — production embedders carry the model file's SHA-256.
    #[must_use]
    pub fn unspecified(dim: usize) -> Self {
        Self {
            name: "unspecified".into(),
            digest: [0u8; 32],
            dim,
        }
    }

    /// Build a model identity from a 64-character hexadecimal content digest.
    ///
    /// # Errors
    /// Returns an embedding error when `digest` is not a SHA-256 value.
    pub fn from_sha256(name: impl Into<String>, digest: &str, dim: usize) -> Result<Self> {
        if digest.len() != 64 {
            return Err(Error::Embed(
                "embedding revision is not a SHA-256 digest".into(),
            ));
        }
        let mut decoded = [0u8; 32];
        for (index, byte) in decoded.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&digest[index * 2..index * 2 + 2], 16)
                .map_err(|_| Error::Embed("invalid embedding revision".into()))?;
        }
        Ok(Self {
            name: name.into(),
            digest: decoded,
            dim,
        })
    }

    /// Filesystem/URL-safe version token `"<name>-<digest[..8] hex>"` — the
    /// cache-key model segment and the REST `{model-ver}` path part.
    #[must_use]
    pub fn token(&self) -> String {
        use std::fmt::Write as _;
        let mut s = String::with_capacity(self.name.len() + 17);
        s.push_str(&self.name);
        s.push('-');
        for b in &self.digest[..8] {
            let _ = write!(s, "{b:02x}");
        }
        s
    }
}

/// Embedder that returns a zero vector. Used in tests and as the
/// when-no-embedder-is-needed default. Never use in production.
#[derive(Debug, Clone, Copy)]
pub struct NullEmbedder {
    /// Output dimension reported by [`Embedder::dim`].
    pub dim: usize,
}

impl Embedder for NullEmbedder {
    fn dim(&self) -> usize {
        self.dim
    }
    fn embed(&self, _text: &str) -> Result<Vec<f32>> {
        Ok(vec![0.0; self.dim])
    }
}

// ---------------------------------------------------------------------------
// Canonicalizing wrapper — `vrules-canon` (always available, pure Rust).
// ---------------------------------------------------------------------------

/// Embedder wrapper that canonicalizes text through an [`vrules_canon::Canonicalizer`]
/// **before** delegating to an inner embedder.
///
/// Compose it OVER a [`CachingEmbedder`](crate::embed_cache::CachingEmbedder) so
/// every variant of a recurring line collapses to one canonical string → one
/// cache key → one embed:
///
/// ```text
/// CanonicalizingEmbedder<CachingEmbedder<ComponentEmbedder>>
///        canonicalize ─┘            cache ─┘     engine ─┘
/// ```
///
/// This is **opt-in**: the bare [`CachingEmbedder`] is unchanged, so existing
/// callers that index raw text are unaffected. When you DO use this, bind the
/// underlying [`EmbedCache`](crate::embed_cache::EmbedCache) to this wrapper's
/// [`cache_namespace`](Self::cache_namespace) (via
/// [`EmbedCache::open_namespaced`](crate::embed_cache::EmbedCache::open_namespaced))
/// so a canonicalizer change can't serve stale vectors.
pub struct CanonicalizingEmbedder<E: Embedder, C: vrules_canon::Canonicalizer + Send + Sync> {
    inner: E,
    canon: C,
}

impl<E: Embedder, C: vrules_canon::Canonicalizer + Send + Sync> CanonicalizingEmbedder<E, C> {
    /// Wrap `inner`, canonicalizing every input with `canon` first.
    pub fn new(inner: E, canon: C) -> Self {
        Self { inner, canon }
    }

    /// The cache-namespace token for this wrapper's strategy: `"<id>/v<version>"`.
    /// Pass to [`EmbedCache::open_namespaced`](crate::embed_cache::EmbedCache::open_namespaced).
    #[must_use]
    pub fn cache_namespace(&self) -> String {
        format!("{}/v{}", self.canon.id(), self.canon.version())
    }

    /// Borrow the inner embedder.
    pub fn inner(&self) -> &E {
        &self.inner
    }
}

impl<E: Embedder, C: vrules_canon::Canonicalizer + Send + Sync> Embedder
    for CanonicalizingEmbedder<E, C>
{
    fn dim(&self) -> usize {
        self.inner.dim()
    }

    fn model_id(&self) -> ModelId {
        self.inner.model_id()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let canon = self.canon.canon(text);
        self.inner.embed(&canon.canonical)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let canon: Vec<String> = texts
            .iter()
            .map(|t| self.canon.canon(t).canonical)
            .collect();
        let refs: Vec<&str> = canon.iter().map(String::as_str).collect();
        self.inner.embed_batch(&refs)
    }
}

// ---------------------------------------------------------------------------
// Data-parallel pool — backend-agnostic, multi-device.
// ---------------------------------------------------------------------------

/// A data-parallel embedder pool: holds N inner embedders ("workers", normally
/// one per accelerator) and spreads work across them. [`Embedder::embed_batch`]
/// shards the batch across workers and runs them **concurrently**;
/// [`Embedder::embed`] round-robins. Backend-agnostic — a worker is any
/// `Arc<dyn Embedder>`, so the pool drives server-backed embedders, a null
/// embedder, or a future backend identically. Per-worker construction lives in
/// the caller (e.g. the embedding component host); this type is purely the parallel fan-out.
pub struct PooledEmbedder {
    workers: Vec<Arc<dyn Embedder>>,
    next: AtomicUsize,
}

impl PooledEmbedder {
    /// Build a pool from pre-constructed workers (one per device). All workers
    /// must report the same `dim()` — they embed into one shared vector space.
    ///
    /// # Errors
    /// `Invariant` if `workers` is empty or their dimensions disagree.
    pub fn new(workers: Vec<Arc<dyn Embedder>>) -> Result<Self> {
        let dim = match workers.first() {
            Some(w) => w.dim(),
            None => return Err(Error::Invariant("PooledEmbedder needs >= 1 worker")),
        };
        if workers.iter().any(|w| w.dim() != dim) {
            return Err(Error::Invariant("PooledEmbedder workers disagree on dim"));
        }
        Ok(Self {
            workers,
            next: AtomicUsize::new(0),
        })
    }

    /// Number of workers (≈ devices) in the pool.
    #[must_use]
    pub fn workers(&self) -> usize {
        self.workers.len()
    }
}

impl Embedder for PooledEmbedder {
    fn dim(&self) -> usize {
        self.workers[0].dim()
    }

    fn model_id(&self) -> ModelId {
        self.workers[0].model_id()
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let i = self.next.fetch_add(1, Ordering::Relaxed) % self.workers.len();
        self.workers[i].embed(text)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        let n = self.workers.len();
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if n == 1 {
            return self.workers[0].embed_batch(texts);
        }
        // Contiguous chunks → concatenating the parts preserves caller order.
        let chunk = texts.len().div_ceil(n);
        let mut parts: Vec<Result<Vec<Vec<f32>>>> = Vec::with_capacity(n);
        std::thread::scope(|s| {
            let mut handles = Vec::new();
            for (i, slice) in texts.chunks(chunk).enumerate() {
                let worker = Arc::clone(&self.workers[i]);
                handles.push(s.spawn(move || worker.embed_batch(slice)));
            }
            for h in handles {
                parts.push(
                    h.join()
                        .unwrap_or_else(|_| Err(Error::Embed("pool worker panicked".into()))),
                );
            }
        });
        let mut out = Vec::with_capacity(texts.len());
        for part in parts {
            out.extend(part?);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod pooled_tests {
    use super::*;

    /// Deterministic worker: encodes `len` and first byte so a reassembled batch
    /// can be checked for both correctness and order.
    struct LenEmbedder {
        dim: usize,
    }
    impl Embedder for LenEmbedder {
        fn dim(&self) -> usize {
            self.dim
        }
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            let mut v = vec![0.0; self.dim];
            v[0] = text.len() as f32;
            if let Some(&b) = text.as_bytes().first() {
                v[1] = b as f32;
            }
            Ok(v)
        }
    }

    fn pool(n: usize) -> PooledEmbedder {
        let workers: Vec<Arc<dyn Embedder>> = (0..n)
            .map(|_| Arc::new(LenEmbedder { dim: 4 }) as Arc<dyn Embedder>)
            .collect();
        PooledEmbedder::new(workers).unwrap()
    }

    #[test]
    fn batch_shards_across_workers_preserving_order() {
        let p = pool(3);
        assert_eq!(p.workers(), 3);
        let texts: Vec<String> = (0..10).map(|i| "x".repeat(i + 1)).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let out = p.embed_batch(&refs).unwrap();
        assert_eq!(out.len(), 10);
        for (i, v) in out.iter().enumerate() {
            assert_eq!(v[0] as usize, texts[i].len(), "order/sharding wrong at {i}");
            assert_eq!(v[1] as u8, b'x');
        }
    }

    #[test]
    fn single_worker_and_empty_batch() {
        let p = pool(1);
        assert!(p.embed_batch(&[]).unwrap().is_empty());
        assert_eq!(p.embed_batch(&["abc"]).unwrap()[0][0] as usize, 3);
    }

    #[test]
    fn rejects_empty_and_dim_mismatch() {
        assert!(PooledEmbedder::new(vec![]).is_err());
        let mixed: Vec<Arc<dyn Embedder>> = vec![
            Arc::new(LenEmbedder { dim: 4 }),
            Arc::new(LenEmbedder { dim: 8 }),
        ];
        assert!(PooledEmbedder::new(mixed).is_err());
    }

    #[test]
    fn round_robin_embed_works() {
        let p = pool(3);
        for _ in 0..7 {
            assert_eq!(p.embed("hello").unwrap()[0] as usize, 5);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_embedder_returns_zero_vec_of_dim() {
        let e = NullEmbedder { dim: 4 };
        let v = e.embed("hi").unwrap();
        assert_eq!(v, vec![0.0; 4]);
    }

    /// Embedder that records every text it actually embeds.
    struct RecordingEmbedder {
        dim: usize,
        seen: std::sync::Mutex<Vec<String>>,
    }
    impl Embedder for RecordingEmbedder {
        fn dim(&self) -> usize {
            self.dim
        }
        fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.seen.lock().unwrap().push(text.to_owned());
            Ok(vec![0.0; self.dim])
        }
    }

    #[test]
    fn canonicalizing_embedder_embeds_canonical_form() {
        let inner = RecordingEmbedder {
            dim: 4,
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let wrap = CanonicalizingEmbedder::new(inner, vrules_canon::LogMask);
        wrap.embed("User 42 login from 10.0.0.1").unwrap();
        let seen = wrap.inner().seen.lock().unwrap();
        assert_eq!(seen[0], "User <*> login from <*>");
    }

    #[test]
    fn canonicalizing_embedder_collapses_variants() {
        let inner = RecordingEmbedder {
            dim: 4,
            seen: std::sync::Mutex::new(Vec::new()),
        };
        let wrap = CanonicalizingEmbedder::new(inner, vrules_canon::LogMask);
        let _ = wrap
            .embed_batch(&["User 1 login", "User 9999 login"])
            .unwrap();
        let seen = wrap.inner().seen.lock().unwrap();
        assert_eq!(seen[0], seen[1], "variants collapse to one canonical form");
    }

    #[test]
    fn cache_namespace_encodes_strategy_and_version() {
        let wrap = CanonicalizingEmbedder::new(NullEmbedder { dim: 4 }, vrules_canon::LogMask);
        assert_eq!(wrap.cache_namespace(), "log-mask/v1");
    }

    #[test]
    fn model_id_parses_sha256_revision() {
        let id = ModelId::from_sha256(
            "model",
            "1cb1b7e0f8b96cee3445e317b8064d8805bf35c7dc7de82cddcb9f78d4c95e0e",
            768,
        )
        .unwrap();
        assert_eq!(id.name, "model");
        assert_eq!(id.dim, 768);
        assert_eq!(id.digest[0], 0x1c);
        assert!(ModelId::from_sha256("model", "invalid", 768).is_err());
    }
}
