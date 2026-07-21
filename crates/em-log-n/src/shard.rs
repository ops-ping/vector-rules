//! Per-domain shard.
//!
//! A shard owns:
//! - a fjall [`Database`](fjall::Database) directory dedicated to it,
//! - one fjall keyspace per logical role (KV rows; reverse lookup table for
//!   ANN → row-key resolution),
//! - N named usearch indexes (text, image, geo, fused, …) with their own
//!   dimensions and metrics (built-in or caller-supplied),
//! - a [`RetentionPolicy`](crate::retention::RetentionPolicy) (compaction-time;
//!   wiring lands with the cold-tier work).
//!
//! Implementation status by feature:
//!
//! - Scaffold types ([`DomainId`], [`Metric`], [`IndexSpec`], [`ShardSpec`])
//!   are always available.
//! - The functional [`Shard`] (open / put / scan / ann / ann_in_window /
//!   multi-index) is gated behind `--features fjall-backend,usearch-backend`.
//!   This is `phase2-spike`.

use crate::error::{Error, Result};

/// Per-domain identifier. Short, ASCII, kebab-case is the convention
/// (e.g. `system-core`, `ui`, `llm-traces`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DomainId(pub String);

impl DomainId {
    /// Construct a domain id, validating the kebab-case convention.
    ///
    /// # Errors
    /// Returns `Invariant` if the id is empty or contains characters outside
    /// `[a-z0-9-]`.
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        if s.is_empty() {
            return Err(Error::Invariant("domain id must be non-empty"));
        }
        if !s
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-'))
        {
            return Err(Error::Invariant(
                "domain id must be ASCII kebab-case [a-z0-9-]",
            ));
        }
        Ok(Self(s))
    }
}

/// Distance metric for a usearch index.
///
/// `Custom` carries a caller-supplied metric closure registered via
/// [`Shard::register_custom_metric`] (see `phase2-custom-metric`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Metric {
    /// Cosine similarity (usearch `Cos`).
    Cosine,
    /// Squared L2 distance (usearch `L2sq`).
    L2sq,
    /// Inner product (usearch `IP`).
    Ip,
    /// Haversine great-circle distance for `(lat, lon)` pairs.
    Haversine,
    /// Caller-supplied metric.
    Custom,
}

/// Configuration for a single named usearch index inside a shard.
#[derive(Debug, Clone)]
pub struct IndexSpec {
    /// Index name (unique within a shard). Used at query time.
    pub name: String,
    /// Vector dimension.
    pub dim: usize,
    /// Distance metric.
    pub metric: Metric,
}

/// Configuration for a domain shard.
#[derive(Debug, Clone)]
pub struct ShardSpec {
    /// Domain id.
    pub domain: DomainId,
    /// Indexes the shard will host. Empty is legal (KV-only domain).
    pub indexes: Vec<IndexSpec>,
}

// ---------------------------------------------------------------------------
// Functional shard — feature-gated end-to-end implementation.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
pub use engine::{AnnHit, Combine, Shard};

#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
mod engine {
    use std::collections::HashMap;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    use fjall::{Database, Keyspace, KeyspaceCreateOptions, PersistMode};
    use usearch::{Index, IndexOptions, MetricKind, ScalarKind};

    use super::{DomainId, IndexSpec, Metric, ShardSpec};
    use crate::error::{Error, Result};
    use crate::key::{time_range_bytes, KeyParts, RowKey, DEFAULT_HASH_LEN};

    /// Capacity reserved for each usearch index on creation. Real usage is
    /// well below this — usearch grows dynamically — but reserving avoids
    /// the "must reserve before add" gotcha.
    const INITIAL_INDEX_CAPACITY: usize = 16_384;

    /// Default oversample factor for [`Shard::ann_in_window`]. Search asks
    /// usearch for `k * oversample` candidates, then filters by time. Tuned
    /// empirically in `phase3-bench`; this default is a deliberately
    /// conservative starting point.
    const DEFAULT_WINDOW_OVERSAMPLE: usize = 4;

    /// How [`Shard::ann_multi`] combines per-index hit sets.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Combine {
        /// Result set is the **intersection** of per-index top-k sets,
        /// ordered by **sum of distances ascending** (smaller = closer).
        /// Use when a row must match ALL query intents (e.g. visually
        /// similar AND textually similar AND geographically close).
        IntersectSum,
        /// Result set is the **union** of per-index top-k sets, ordered by
        /// **min distance across hits ascending**. Use when a row that
        /// matches ANY query intent should appear, with strongest match
        /// determining rank.
        UnionMin,
    }

    /// Resolved nearest-neighbour hit.
    #[derive(Debug, Clone)]
    pub struct AnnHit {
        /// Original row key (the same bytes fjall sees).
        pub row_key: RowKey,
        /// Distance under the index's metric (smaller = closer for
        /// distance-style metrics; usearch follows its `MetricKind` semantics).
        pub distance: f32,
    }

    /// Functional per-domain shard.
    ///
    /// One [`Shard`] owns one fjall [`Database`] directory plus one usearch
    /// [`Index`] per spec entry. Writes are immediately visible to both KV
    /// scans and ANN searches — no microbatch, no flush wait.
    pub struct Shard {
        spec: ShardSpec,
        /// Shard root directory — also where each index's usearch file lives.
        root: PathBuf,
        db: Database,
        kv: Keyspace,
        rev: Keyspace,
        /// One usearch index per [`IndexSpec`]. `Mutex` because the cxx
        /// binding's `change_metric` requires `&mut`, and we want a single
        /// place to serialize writes per index (usearch's own concurrency
        /// model is otherwise read-mostly).
        indexes: HashMap<String, Mutex<Index>>,
    }

    impl Shard {
        /// Open (or create) the shard at `path`. The directory is created if
        /// missing; opening an existing shard reuses its keyspaces and **restores
        /// each usearch index from its persisted file** (`usearch-<name>.idx` in
        /// the shard directory) if present — so vectors survive restarts. Indexes
        /// are saved on every [`put`](Self::put) / [`add_vector`](Self::add_vector).
        ///
        /// # Errors
        /// Surfaces fjall and usearch errors.
        pub fn open(spec: ShardSpec, path: impl AsRef<Path>) -> Result<Self> {
            let db = Database::builder(path.as_ref())
                .open()
                .map_err(|e| Error::KvBackend(format!("open db: {e}")))?;
            let kv = db
                .keyspace("kv", KeyspaceCreateOptions::default)
                .map_err(|e| Error::KvBackend(format!("open kv keyspace: {e}")))?;
            let rev = db
                .keyspace("rev", KeyspaceCreateOptions::default)
                .map_err(|e| Error::KvBackend(format!("open rev keyspace: {e}")))?;

            let root = path.as_ref().to_path_buf();
            let mut indexes = HashMap::with_capacity(spec.indexes.len());
            for ispec in &spec.indexes {
                let idx = open_index(ispec)?;
                // Restore a previously persisted index, if one exists on disk.
                let file = index_file_path(&root, &ispec.name);
                if file.exists() {
                    let p = file
                        .to_str()
                        .ok_or(Error::Invariant("non-utf8 index path"))?;
                    idx.load(p).map_err(|e| {
                        Error::VectorIndex(format!("load index {}: {e}", ispec.name))
                    })?;
                    // `load` resets capacity to the saved size, so without this the
                    // next `add` after a restart fails ("Reserve capacity ahead of
                    // insertions!"). Re-reserve headroom above the loaded count.
                    idx.reserve(idx.size() + INITIAL_INDEX_CAPACITY)
                        .map_err(|e| {
                            Error::VectorIndex(format!("reserve after load {}: {e}", ispec.name))
                        })?;
                }
                indexes.insert(ispec.name.clone(), Mutex::new(idx));
            }

            Ok(Self {
                spec,
                root,
                db,
                kv,
                rev,
                indexes,
            })
        }

        /// Read-only view of this shard's spec.
        #[must_use]
        pub fn spec(&self) -> &ShardSpec {
            &self.spec
        }

        /// Read-only view of this shard's domain id.
        #[must_use]
        pub fn domain(&self) -> &DomainId {
            &self.spec.domain
        }

        /// Register a caller-supplied distance metric for an index declared
        /// with [`Metric::Custom`].
        ///
        /// The `metric` closure receives `(a, b)` as `&[f32]` slices of
        /// length equal to the index's `dim` and returns a distance — the
        /// usearch convention is "smaller = closer". Used for fused
        /// embeddings, partial-similarity composites (e.g. image+text
        /// concatenated with per-half cosine summed), or any caller-defined
        /// distance.
        ///
        /// Must be called **before** the first `put` to an index that uses
        /// `Metric::Custom`; until registered, distance reduces to the
        /// placeholder (cosine).
        ///
        /// # Errors
        /// Returns `Invariant` for unknown `index_name`.
        pub fn register_custom_metric<F>(&self, index_name: &str, metric: F) -> Result<()>
        where
            F: Fn(&[f32], &[f32]) -> f32 + Send + Sync + 'static,
        {
            let ispec = self
                .spec
                .indexes
                .iter()
                .find(|s| s.name == index_name)
                .ok_or(Error::Invariant(
                    "unknown index name in register_custom_metric",
                ))?;
            let dim = ispec.dim;
            let metric = std::sync::Arc::new(metric);

            let mut guard = self
                .indexes
                .get(index_name)
                .ok_or(Error::Invariant("missing index handle"))?
                .lock()
                .map_err(|_| Error::Invariant("index mutex poisoned"))?;

            // Bridge usearch's raw-pointer ABI to our safe Rust closure.
            // Safety: usearch invokes this with two pointers to `dim` f32s
            // packed contiguously, allocated by usearch itself (the same
            // memory it stores vectors in). We never let the slices outlive
            // the call.
            #[allow(unsafe_code)]
            let bridge: Box<dyn Fn(*const f32, *const f32) -> f32 + Send + Sync> = {
                let metric = std::sync::Arc::clone(&metric);
                Box::new(move |a: *const f32, b: *const f32| -> f32 {
                    let a_slice = unsafe { std::slice::from_raw_parts(a, dim) };
                    let b_slice = unsafe { std::slice::from_raw_parts(b, dim) };
                    metric(a_slice, b_slice)
                })
            };
            guard.change_metric::<f32>(bridge);
            Ok(())
        }

        /// Persist a single event.
        ///
        /// - `row_key` is the typed reverse-ts key (see [`crate::key`]).
        /// - `value_bytes` is the codec-encoded value bytes.
        /// - `vectors` carries `(index_name, vector)` pairs. Each name must
        ///   match an [`IndexSpec`] in this shard's [`ShardSpec`]; vector
        ///   length must equal that spec's `dim`. Unknown names or wrong
        ///   dimensions are rejected before any write happens (no partial
        ///   commits).
        ///
        /// Writes are durable on return (`PersistMode::SyncAll` on the
        /// journal). Vectors are added to in-memory usearch indexes; the
        /// cold-tier handoff lands in `phase3-object-store`.
        ///
        /// # Errors
        /// Returns `Invariant` for spec mismatches, `KvBackend` for fjall
        /// failures, `VectorIndex` for usearch failures.
        pub fn put(
            &self,
            row_key: &RowKey,
            value_bytes: &[u8],
            vectors: &[(&str, &[f32])],
        ) -> Result<()> {
            // Validate first; never write a partial row.
            for (name, vec) in vectors {
                let ispec = self
                    .spec
                    .indexes
                    .iter()
                    .find(|s| s.name == *name)
                    .ok_or(Error::Invariant("unknown index name in put"))?;
                if vec.len() != ispec.dim {
                    return Err(Error::Invariant("vector dim mismatch"));
                }
            }

            let u64_key = usearch_key_for(row_key);

            // KV write + reverse-table write (both keyspaces).
            self.kv
                .insert(row_key.as_bytes(), value_bytes)
                .map_err(|e| Error::KvBackend(format!("kv insert: {e}")))?;
            self.rev
                .insert(u64_key.to_be_bytes(), row_key.as_bytes())
                .map_err(|e| Error::KvBackend(format!("rev insert: {e}")))?;

            // Vector index writes.
            for (name, vec) in vectors {
                let idx = self
                    .indexes
                    .get(*name)
                    .ok_or(Error::Invariant("missing index handle"))?
                    .lock()
                    .map_err(|_| Error::Invariant("index mutex poisoned"))?;
                upsert(&idx, u64_key, vec)?;
                save_index(&self.root, name, &idx)?;
            } // `name` here is `&&str`; save_index takes `&str` (deref coerces).

            // Durable on return.
            self.db
                .persist(PersistMode::SyncAll)
                .map_err(|e| Error::KvBackend(format!("persist: {e}")))?;
            Ok(())
        }

        /// Add a single vector to one named index for a row that **already
        /// exists** in the KV store, without rewriting the value or the reverse
        /// table.
        ///
        /// This is the backfill primitive behind the dual coarse/detail index
        /// design: the hot path writes the row plus its cheap canonical vector
        /// via [`put`](Self::put); the expensive raw/detail vector is computed
        /// off the hot path and added here later (see
        /// [`crate::detail_backfill`]).
        ///
        /// # Errors
        /// - `Invariant` if `index_name` is unknown, the dim mismatches, or the
        ///   row does not exist (we refuse to create an index entry with no
        ///   backing KV row).
        /// - Surfaces vector-index / fjall errors.
        pub fn add_vector(&self, row_key: &RowKey, index_name: &str, vector: &[f32]) -> Result<()> {
            let ispec = self
                .spec
                .indexes
                .iter()
                .find(|s| s.name == index_name)
                .ok_or(Error::Invariant("unknown index name in add_vector"))?;
            if vector.len() != ispec.dim {
                return Err(Error::Invariant("vector dim mismatch in add_vector"));
            }
            // Refuse to orphan a vector: the row must already exist.
            if self
                .kv
                .get(row_key.as_bytes())
                .map_err(|e| Error::KvBackend(format!("kv get: {e}")))?
                .is_none()
            {
                return Err(Error::Invariant("add_vector for nonexistent row"));
            }

            let u64_key = usearch_key_for(row_key);
            {
                let idx = self
                    .indexes
                    .get(index_name)
                    .ok_or(Error::Invariant("missing index handle"))?
                    .lock()
                    .map_err(|_| Error::Invariant("index mutex poisoned"))?;
                upsert(&idx, u64_key, vector)?;
                save_index(&self.root, index_name, &idx)?;
            }

            self.db
                .persist(PersistMode::SyncAll)
                .map_err(|e| Error::KvBackend(format!("persist: {e}")))?;
            Ok(())
        }

        /// Delete a row and all of its vectors from this shard.
        ///
        /// Returns `true` when the KV row existed. The operation is idempotent:
        /// deleting an absent row still removes any matching vector/reverse-table
        /// entries derived from the same row key.
        ///
        /// # Errors
        /// Surfaces fjall/usearch persistence errors.
        pub fn delete(&self, row_key: &RowKey) -> Result<bool> {
            let existed = self
                .kv
                .get(row_key.as_bytes())
                .map_err(|e| Error::KvBackend(format!("kv get before delete: {e}")))?
                .is_some();
            let u64_key = usearch_key_for(row_key);

            self.kv
                .remove(row_key.as_bytes())
                .map_err(|e| Error::KvBackend(format!("kv delete: {e}")))?;
            self.rev
                .remove(u64_key.to_be_bytes())
                .map_err(|e| Error::KvBackend(format!("rev delete: {e}")))?;

            for (name, idx) in &self.indexes {
                let idx = idx
                    .lock()
                    .map_err(|_| Error::Invariant("index mutex poisoned"))?;
                if idx.contains(u64_key) {
                    idx.remove(u64_key)
                        .map_err(|e| Error::VectorIndex(format!("remove {name}: {e}")))?;
                    save_index(&self.root, name, &idx)?;
                }
            }

            self.db
                .persist(PersistMode::SyncAll)
                .map_err(|e| Error::KvBackend(format!("persist: {e}")))?;
            Ok(existed)
        }

        /// Number of vectors currently in a named index (usearch `size`). Lets a
        /// caller decide whether an index needs (re)building — e.g. an empty index
        /// over a non-empty KV log means the vectors were never persisted.
        ///
        /// # Errors
        /// `Invariant` for an unknown index name.
        pub fn index_len(&self, index_name: &str) -> Result<usize> {
            let idx = self
                .indexes
                .get(index_name)
                .ok_or(Error::Invariant("unknown index name in index_len"))?
                .lock()
                .map_err(|_| Error::Invariant("index mutex poisoned"))?;
            Ok(idx.size())
        }

        /// Point read.
        ///
        /// # Errors
        /// Surfaces fjall errors.
        pub fn get(&self, row_key: &RowKey) -> Result<Option<Vec<u8>>> {
            Ok(self
                .kv
                .get(row_key.as_bytes())
                .map_err(|e| Error::KvBackend(format!("kv get: {e}")))?
                .map(|v| v.as_ref().to_vec()))
        }

        /// Newest-first scan of the whole shard.
        ///
        /// # Errors
        /// Surfaces fjall errors mid-iteration.
        pub fn scan(&self, limit: usize) -> Result<Vec<(RowKey, Vec<u8>)>> {
            let mut out = Vec::with_capacity(limit);
            for guard in self.kv.iter().take(limit) {
                let (k, v) = guard
                    .into_inner()
                    .map_err(|e| Error::KvBackend(format!("scan iter: {e}")))?;
                out.push((
                    RowKey::from_bytes(k.as_ref().to_vec())?,
                    v.as_ref().to_vec(),
                ));
            }
            Ok(out)
        }

        /// Newest-first scan restricted to `[t_lo, t_hi)` (nanoseconds).
        ///
        /// Implemented as a byte-range scan over the reverse-ts prefix —
        /// fjall walks one contiguous slice of the LSM-tree, not the whole
        /// shard.
        ///
        /// # Errors
        /// Surfaces fjall errors mid-iteration.
        pub fn scan_window(
            &self,
            t_lo: u64,
            t_hi: u64,
            limit: usize,
        ) -> Result<Vec<(RowKey, Vec<u8>)>> {
            let (start, end) = time_range_bytes(t_lo, t_hi);
            // Range on the 8-byte prefix bounds *all* row keys whose ts prefix
            // sits in the window. We rely on fjall returning the full row
            // keys, then re-check ts to drop any boundary stragglers from
            // longer keys whose hash bytes land past `end`.
            let mut out = Vec::with_capacity(limit);
            let extended_end = {
                let mut e = end.to_vec();
                e.extend_from_slice(&[0xFFu8; DEFAULT_HASH_LEN + 8]);
                e
            };
            for guard in self.kv.range(start.as_slice()..=extended_end.as_slice()) {
                if out.len() >= limit {
                    break;
                }
                let (k, v) = guard
                    .into_inner()
                    .map_err(|e| Error::KvBackend(format!("scan_window iter: {e}")))?;
                let row_key = RowKey::from_bytes(k.as_ref().to_vec())?;
                let parts = KeyParts::parse(row_key.as_bytes(), DEFAULT_HASH_LEN)?;
                if parts.ts_nanos >= t_lo && parts.ts_nanos < t_hi {
                    out.push((row_key, v.as_ref().to_vec()));
                }
            }
            Ok(out)
        }

        /// Top-`k` ANN search on a named usearch index.
        ///
        /// # Errors
        /// `Invariant` for unknown `index_name`; `VectorIndex` for usearch
        /// failures; `KvBackend` for reverse-table lookups.
        pub fn ann(&self, index_name: &str, qvec: &[f32], k: usize) -> Result<Vec<AnnHit>> {
            let idx_lock = self
                .indexes
                .get(index_name)
                .ok_or(Error::Invariant("unknown index name in ann"))?;
            let matches = {
                let idx = idx_lock
                    .lock()
                    .map_err(|_| Error::Invariant("index mutex poisoned"))?;
                idx.search(qvec, k)
                    .map_err(|e| Error::VectorIndex(format!("search: {e}")))?
            };
            self.resolve(&matches.keys, &matches.distances)
        }

        /// Top-`k` ANN search restricted to `[t_lo, t_hi)`.
        ///
        /// Strategy: ask usearch for `k * oversample` candidates, then filter
        /// post-hoc by parsing each candidate row key's ts. Correct but
        /// scan-bound; the NEXT-style per-segment shards in
        /// `phase3-object-store` (and the time-aware u64 key encoding in
        /// `phase3-bench`) replace this with a true predicate filter.
        ///
        /// # Errors
        /// Same as [`Self::ann`].
        pub fn ann_in_window(
            &self,
            index_name: &str,
            qvec: &[f32],
            k: usize,
            t_lo: u64,
            t_hi: u64,
        ) -> Result<Vec<AnnHit>> {
            let oversample = DEFAULT_WINDOW_OVERSAMPLE.max(1);
            let raw = self.ann(index_name, qvec, k.saturating_mul(oversample))?;
            let mut out = Vec::with_capacity(k);
            for hit in raw {
                if out.len() >= k {
                    break;
                }
                let parts = KeyParts::parse(hit.row_key.as_bytes(), DEFAULT_HASH_LEN)?;
                if parts.ts_nanos >= t_lo && parts.ts_nanos < t_hi {
                    out.push(hit);
                }
            }
            Ok(out)
        }

        /// Multi-index ANN: search several indexes independently, then
        /// combine. Implements the "marketplace" pattern (similar-images ∩
        /// similar-text ∩ geographically-close) and its union counterpart.
        ///
        /// Each `queries` entry is `(index_name, query_vec, per_index_k)`.
        /// `per_index_k` is how many candidates to ask each index for; the
        /// returned `Vec` is **sorted by combined score ascending and
        /// truncated to `final_k`**.
        ///
        /// # Errors
        /// `Invariant` for unknown index names; otherwise as [`Self::ann`].
        pub fn ann_multi(
            &self,
            queries: &[(&str, &[f32], usize)],
            combine: Combine,
            final_k: usize,
        ) -> Result<Vec<AnnHit>> {
            if queries.is_empty() {
                return Ok(Vec::new());
            }

            // Collect per-query results as (RowKey, distance) maps.
            let mut per_query: Vec<std::collections::HashMap<Vec<u8>, f32>> =
                Vec::with_capacity(queries.len());
            for (name, qvec, k) in queries {
                let hits = self.ann(name, qvec, *k)?;
                let mut m = std::collections::HashMap::with_capacity(hits.len());
                for h in hits {
                    m.insert(h.row_key.as_bytes().to_vec(), h.distance);
                }
                per_query.push(m);
            }

            let mut combined: Vec<(Vec<u8>, f32)> = match combine {
                Combine::IntersectSum => {
                    let first = per_query.first().expect("checked non-empty");
                    first
                        .iter()
                        .filter_map(|(key, &d0)| {
                            let mut sum = d0;
                            for other in &per_query[1..] {
                                let d = *other.get(key)?;
                                sum += d;
                            }
                            Some((key.clone(), sum))
                        })
                        .collect()
                }
                Combine::UnionMin => {
                    let mut all: std::collections::HashMap<Vec<u8>, f32> =
                        std::collections::HashMap::new();
                    for m in &per_query {
                        for (k, &d) in m {
                            all.entry(k.clone())
                                .and_modify(|cur| {
                                    if d < *cur {
                                        *cur = d;
                                    }
                                })
                                .or_insert(d);
                        }
                    }
                    all.into_iter().collect()
                }
            };

            combined.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            combined.truncate(final_k);

            combined
                .into_iter()
                .map(|(k, d)| {
                    Ok(AnnHit {
                        row_key: RowKey::from_bytes(k)?,
                        distance: d,
                    })
                })
                .collect()
        }

        // ----- cold-tier helpers (called by em_log_n::coldtier) -----

        /// Forward iteration over `(key_bytes, value_bytes)` pairs, for
        /// snapshotting. Order of emission doesn't carry semantic meaning
        /// (newest-first is a property of the key encoding, not the
        /// snapshot file).
        pub(crate) fn iter_raw(&self) -> RawIter<'_> {
            RawIter {
                inner: self.kv.iter(),
                _phantom: std::marker::PhantomData,
            }
        }

        /// Direct write of `(row_key_bytes, value_bytes)` plus reverse-table
        /// entry. Used by [`crate::coldtier::restore_into`]; does **not**
        /// re-insert into vector indexes (those are loaded by
        /// `load_index_bytes`).
        ///
        /// # Errors
        /// Surfaces fjall errors.
        pub(crate) fn put_raw_kv(&self, key: &[u8], value: &[u8]) -> Result<()> {
            self.kv
                .insert(key, value)
                .map_err(|e| Error::KvBackend(format!("kv insert: {e}")))?;
            // Reverse table maps usearch-key → row-key. Recompute exactly
            // the same usearch-key the original put would have used.
            let row_key = RowKey::from_bytes(key.to_vec())?;
            let uk = usearch_key_for(&row_key);
            self.rev
                .insert(uk.to_be_bytes(), key)
                .map_err(|e| Error::KvBackend(format!("rev insert: {e}")))?;
            Ok(())
        }

        /// Serialize a named usearch index to a buffer.
        ///
        /// # Errors
        /// `Invariant` for unknown `index_name`; `VectorIndex` for usearch
        /// errors.
        pub fn snapshot_index_bytes(&self, index_name: &str) -> Result<Vec<u8>> {
            let guard = self
                .indexes
                .get(index_name)
                .ok_or(Error::Invariant("unknown index name in snapshot"))?
                .lock()
                .map_err(|_| Error::Invariant("index mutex poisoned"))?;
            let len = guard.serialized_length();
            let mut buf = vec![0u8; len];
            guard
                .save_to_buffer(&mut buf)
                .map_err(|e| Error::VectorIndex(format!("save_to_buffer: {e}")))?;
            Ok(buf)
        }

        /// Load a named usearch index from a buffer (in place).
        ///
        /// # Errors
        /// `Invariant` for unknown `index_name`; `VectorIndex` for usearch
        /// errors.
        pub fn load_index_bytes(&self, index_name: &str, buf: &[u8]) -> Result<()> {
            let guard = self
                .indexes
                .get(index_name)
                .ok_or(Error::Invariant("unknown index name in load"))?
                .lock()
                .map_err(|_| Error::Invariant("index mutex poisoned"))?;
            guard
                .load_from_buffer(buf)
                .map_err(|e| Error::VectorIndex(format!("load_from_buffer: {e}")))?;
            Ok(())
        }

        // ----- internal helpers -----

        fn resolve(&self, keys: &[u64], distances: &[f32]) -> Result<Vec<AnnHit>> {
            let mut out = Vec::with_capacity(keys.len());
            for (i, k) in keys.iter().enumerate() {
                let row_key_bytes = self
                    .rev
                    .get(k.to_be_bytes())
                    .map_err(|e| Error::KvBackend(format!("rev get: {e}")))?
                    .ok_or(Error::Invariant("rev table missing key"))?;
                let dist = *distances
                    .get(i)
                    .ok_or(Error::Invariant("distance vector shorter than keys"))?;
                out.push(AnnHit {
                    row_key: RowKey::from_bytes(row_key_bytes.as_ref().to_vec())?,
                    distance: dist,
                });
            }
            Ok(out)
        }
    }

    /// Iterator over raw `(key_bytes, value_bytes)` pairs for cold-tier
    /// snapshotting. Each `next()` materializes the guard into owned
    /// buffers.
    pub(crate) struct RawIter<'a> {
        inner: fjall::Iter,
        _phantom: std::marker::PhantomData<&'a ()>,
    }

    impl<'a> Iterator for RawIter<'a> {
        type Item = Result<(Vec<u8>, Vec<u8>)>;
        fn next(&mut self) -> Option<Self::Item> {
            let guard = self.inner.next()?;
            Some(
                guard
                    .into_inner()
                    .map(|(k, v)| (k.as_ref().to_vec(), v.as_ref().to_vec()))
                    .map_err(|e| Error::KvBackend(format!("raw iter: {e}"))),
            )
        }
    }

    /// Map a [`RowKey`] to the u64 key used in usearch. We hash the entire
    /// row key (not just the ts prefix) so collisions are content-derived
    /// and infinitesimally rare; the reverse-lookup keyspace makes round-
    /// trip from u64 → full row key cheap.
    fn usearch_key_for(row_key: &RowKey) -> u64 {
        let h = blake3::hash(row_key.as_bytes());
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&h.as_bytes()[..8]);
        u64::from_be_bytes(buf)
    }

    /// On-disk path for a named index's persisted usearch file, in the shard dir.
    fn index_file_path(root: &Path, name: &str) -> PathBuf {
        root.join(format!("usearch-{name}.idx"))
    }

    /// Persist a usearch index to its file. Called after every add so vectors
    /// survive a restart. NOTE: usearch serializes the **whole** index per save
    /// (no incremental append), so this is O(index size) per write — fine for
    /// low-write logs; batched/segment persistence is future work.
    fn save_index(root: &Path, name: &str, idx: &Index) -> Result<()> {
        let path = index_file_path(root, name);
        let p = path
            .to_str()
            .ok_or(Error::Invariant("non-utf8 index path"))?;
        idx.save(p)
            .map_err(|e| Error::VectorIndex(format!("save index {name}: {e}")))
    }

    /// Add a vector under `key`, replacing any existing entry. usearch (opened
    /// `multi: false`) errors on a duplicate key rather than replacing, so re-adding
    /// the same row — e.g. an index rebuild — would fail; remove-then-add makes it
    /// idempotent (an upsert).
    fn upsert(idx: &Index, key: u64, vector: &[f32]) -> Result<()> {
        if idx.contains(key) {
            idx.remove(key)
                .map_err(|e| Error::VectorIndex(format!("remove: {e}")))?;
        }
        idx.add(key, vector)
            .map_err(|e| Error::VectorIndex(format!("add: {e}")))
    }

    fn open_index(spec: &IndexSpec) -> Result<Index> {
        let metric = match spec.metric {
            Metric::Cosine => MetricKind::Cos,
            Metric::L2sq => MetricKind::L2sq,
            Metric::Ip => MetricKind::IP,
            Metric::Haversine => MetricKind::Haversine,
            // For Custom we open with a placeholder MetricKind; the actual
            // metric function is registered via change_metric (wired in
            // phase2-custom-metric). Cosine is a safe placeholder.
            Metric::Custom => MetricKind::Cos,
        };
        let options = IndexOptions {
            dimensions: spec.dim,
            metric,
            quantization: ScalarKind::F32,
            connectivity: 0,
            expansion_add: 0,
            expansion_search: 0,
            multi: false,
        };
        let idx =
            Index::new(&options).map_err(|e| Error::VectorIndex(format!("Index::new: {e}")))?;
        idx.reserve(INITIAL_INDEX_CAPACITY)
            .map_err(|e| Error::VectorIndex(format!("reserve: {e}")))?;
        Ok(idx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_id_accepts_kebab() {
        assert!(DomainId::new("system-core").is_ok());
        assert!(DomainId::new("ui").is_ok());
        assert!(DomainId::new("llm-traces-v2").is_ok());
    }

    #[test]
    fn domain_id_rejects_bad_input() {
        assert!(DomainId::new("").is_err());
        assert!(DomainId::new("Has-Caps").is_err());
        assert!(DomainId::new("under_score").is_err());
        assert!(DomainId::new("dots.bad").is_err());
    }
}
