//! Shale-style object-store cold tier.
//!
//! See [`docs/cold-tier.md`](https://github.com/ops-ping/em-log-n/blob/main/docs/cold-tier.md)
//! for the full semantics (publish, restore, GC, absent-vs-error mapping,
//! validate-after-GET + retry transient).
//!
//! Model (mirrors shale's whole-blob, no-rename, no-mmap-of-remote design):
//!
//! - Each shard publishes **immutable generation directories**:
//!   `<root>/<domain>/gen/NNNNNNNN/{kv.snapshot, idx.<name>.usearch}`.
//! - A single **manifest blob** `<root>/<domain>/manifest` names the live
//!   generations as a comma-separated ASCII list of u64s (e.g. `40,41,42`).
//!   Object-store atomic PUT semantics mean readers see a whole-old or
//!   whole-new manifest, never torn.
//! - Old generations leaving the live window are deleted (GC). Readers
//!   tolerate the race by **re-checking the manifest** after a missing
//!   segment: if the gen has left the manifest, the failure is benign.
//!
//! This module currently provides:
//!
//! - [`ObjectStore`] trait with [`InMemoryStore`] (tests) and [`LocalDirStore`]
//!   (production substrate that wraps a local directory; the same trait is
//!   what a future cloud `ObjectStore` impl will implement).
//! - [`publish_generation`] for a shard → returns the new generation number.
//! - [`restore_into`] for a freshly-opened shard → loads the latest generation.
//! - GC helper that keeps the most recent N generations.
//! - GC-race tolerance built into [`restore_into`].
//!
//! ## TODO: in-process cloud backend
//!
//! See [`docs/roadmap.md`](https://github.com/ops-ping/em-log-n/blob/main/docs/roadmap.md).
//! Target: a new `coldtier-cloud` cargo feature implementing this
//! [`ObjectStore`] trait via Apache Arrow's
//! [`object_store`](https://crates.io/crates/object_store) crate
//! (pure-Rust S3 + GCS + Azure under one API). The
//! [`docs/cold-tier.md`](https://github.com/ops-ping/em-log-n/blob/main/docs/cold-tier.md)
//! document captures the absent-vs-error mapping and retry semantics that
//! impl must respect.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::error::{Error, Result};

#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
const MANIFEST_KEY: &str = "manifest";

/// Whole-blob object-store interface.
///
/// All implementations must guarantee that [`Self::put`] is atomic at the
/// **whole-blob** level — readers see either the previous blob in full or
/// the new blob in full, never a torn or partial body. This is the same
/// guarantee S3/Azure/GCS provide for whole-object PUTs and what shale relies
/// on.
pub trait ObjectStore: Send + Sync {
    /// Write `bytes` at `key`, atomically replacing any prior value.
    ///
    /// # Errors
    /// Returns an `ObjectStore` error on I/O failure.
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()>;

    /// Read the blob at `key`. Returns `None` if absent.
    ///
    /// # Errors
    /// Returns an `ObjectStore` error on I/O failure other than absence.
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;

    /// List keys beginning with `prefix`. Order is implementation-defined.
    ///
    /// # Errors
    /// Returns an `ObjectStore` error on I/O failure.
    fn list(&self, prefix: &str) -> Result<Vec<String>>;

    /// Delete the blob at `key`. Missing keys are not errors.
    ///
    /// # Errors
    /// Returns an `ObjectStore` error on I/O failure.
    fn delete(&self, key: &str) -> Result<()>;
}

// ---------------------------------------------------------------------------
// InMemoryStore
// ---------------------------------------------------------------------------

/// Thread-safe in-memory [`ObjectStore`]. Intended for tests and harnesses.
#[derive(Default)]
pub struct InMemoryStore {
    inner: Mutex<std::collections::BTreeMap<String, Vec<u8>>>,
}

impl InMemoryStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl ObjectStore for InMemoryStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| Error::ObjectStore("in-memory mutex poisoned".into()))?;
        g.insert(key.to_owned(), bytes.to_vec());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let g = self
            .inner
            .lock()
            .map_err(|_| Error::ObjectStore("in-memory mutex poisoned".into()))?;
        Ok(g.get(key).cloned())
    }
    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let g = self
            .inner
            .lock()
            .map_err(|_| Error::ObjectStore("in-memory mutex poisoned".into()))?;
        Ok(g.keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect())
    }
    fn delete(&self, key: &str) -> Result<()> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| Error::ObjectStore("in-memory mutex poisoned".into()))?;
        g.remove(key);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// LocalDirStore
// ---------------------------------------------------------------------------

/// Local-filesystem [`ObjectStore`] backed by a directory tree.
///
/// Atomic [`Self::put`] is implemented as **write-to-tempfile + rename** —
/// POSIX rename within a single filesystem is atomic, which mirrors the
/// whole-blob atomicity of S3/Azure PUT.
pub struct LocalDirStore {
    root: PathBuf,
}

impl LocalDirStore {
    /// Use `root` as the storage directory; created if missing.
    ///
    /// # Errors
    /// Returns I/O errors from directory creation.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    fn full_path(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }
}

impl ObjectStore for LocalDirStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.full_path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.full_path(key)) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::Io(e)),
        }
    }
    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        // Walk the tree; em-log-n's manifest+generation prefixes are short and
        // shallow so a recursive walk is fine.
        let mut out = Vec::new();
        let prefix_path = self.root.join(prefix);
        let walk_root = if prefix_path.is_dir() {
            prefix_path
        } else {
            self.root.clone()
        };
        fn walk(dir: &Path, root: &Path, prefix: &str, out: &mut Vec<String>) -> Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, root, prefix, out)?;
                } else {
                    let rel = p
                        .strip_prefix(root)
                        .map_err(|_| Error::ObjectStore("strip_prefix".into()))?
                        .to_string_lossy()
                        .replace('\\', "/");
                    if rel.starts_with(prefix) {
                        out.push(rel);
                    }
                }
            }
            Ok(())
        }
        if walk_root.exists() {
            walk(&walk_root, &self.root, prefix, &mut out)?;
        }
        Ok(out)
    }
    fn delete(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.full_path(key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(Error::Io(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Manifest
// ---------------------------------------------------------------------------

/// Comma-separated ASCII list of live generation numbers (oldest → newest).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Manifest {
    /// Generation numbers currently live, in monotonic order.
    pub generations: Vec<u64>,
}

impl Manifest {
    /// Parse the wire format. Empty bytes ⇒ empty manifest.
    ///
    /// # Errors
    /// Returns `ObjectStore` if any token is not a valid u64.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let s = std::str::from_utf8(bytes)
            .map_err(|e| Error::ObjectStore(format!("manifest utf8: {e}")))?
            .trim();
        if s.is_empty() {
            return Ok(Self::default());
        }
        let mut gens = Vec::new();
        for tok in s.split(',') {
            let g: u64 = tok
                .trim()
                .parse()
                .map_err(|e| Error::ObjectStore(format!("manifest token {tok:?}: {e}")))?;
            gens.push(g);
        }
        Ok(Self { generations: gens })
    }
    /// Encode to the wire format.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut s = String::new();
        for (i, g) in self.generations.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&g.to_string());
        }
        s.into_bytes()
    }
    /// Greatest live generation, if any.
    #[must_use]
    pub fn latest(&self) -> Option<u64> {
        self.generations.iter().copied().max()
    }
}

// ---------------------------------------------------------------------------
// Key helpers
// ---------------------------------------------------------------------------

#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
fn manifest_key(domain: &str) -> String {
    format!("{domain}/{MANIFEST_KEY}")
}
#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
fn gen_dir(domain: &str, gen: u64) -> String {
    format!("{domain}/gen/{gen:08}")
}
#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
fn kv_key(domain: &str, gen: u64) -> String {
    format!("{}/kv.snapshot", gen_dir(domain, gen))
}
#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
fn idx_key(domain: &str, gen: u64, name: &str) -> String {
    format!("{}/idx.{name}.usearch", gen_dir(domain, gen))
}

// ---------------------------------------------------------------------------
// Snapshot helpers
// ---------------------------------------------------------------------------

/// Wire format for the KV snapshot: a sequence of length-prefixed
/// `(key_len: u32 BE, key_bytes, val_len: u32 BE, val_bytes)` records, in
/// **forward** (insertion) iterator order. Newest-first scan ordering is a
/// property of the key encoding, not the snapshot file — order at rest
/// doesn't matter; what matters is that the loader re-inserts every row.
#[cfg(any(test, all(feature = "fjall-backend", feature = "usearch-backend")))]
fn encode_kv_record(out: &mut Vec<u8>, key: &[u8], val: &[u8]) -> Result<()> {
    let kl =
        u32::try_from(key.len()).map_err(|_| Error::ObjectStore("snapshot key >4GiB".into()))?;
    let vl =
        u32::try_from(val.len()).map_err(|_| Error::ObjectStore("snapshot val >4GiB".into()))?;
    out.extend_from_slice(&kl.to_be_bytes());
    out.extend_from_slice(key);
    out.extend_from_slice(&vl.to_be_bytes());
    out.extend_from_slice(val);
    Ok(())
}

/// Iterate (key, val) pairs from a buffer produced by [`encode_kv_record`].
pub struct KvSnapshotReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> KvSnapshotReader<'a> {
    /// New reader over a serialized snapshot buffer.
    #[must_use]
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }
}

impl<'a> Iterator for KvSnapshotReader<'a> {
    type Item = Result<(&'a [u8], &'a [u8])>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let mut try_read = || -> Result<(&'a [u8], &'a [u8])> {
            let bytes = self.bytes;
            let need = self.pos + 4;
            if bytes.len() < need {
                return Err(Error::ObjectStore("snapshot truncated at klen".into()));
            }
            let mut klen = [0u8; 4];
            klen.copy_from_slice(&bytes[self.pos..self.pos + 4]);
            self.pos += 4;
            let kl = u32::from_be_bytes(klen) as usize;
            if bytes.len() < self.pos + kl + 4 {
                return Err(Error::ObjectStore("snapshot truncated at key".into()));
            }
            let key = &bytes[self.pos..self.pos + kl];
            self.pos += kl;
            let mut vlen = [0u8; 4];
            vlen.copy_from_slice(&bytes[self.pos..self.pos + 4]);
            self.pos += 4;
            let vl = u32::from_be_bytes(vlen) as usize;
            if bytes.len() < self.pos + vl {
                return Err(Error::ObjectStore("snapshot truncated at val".into()));
            }
            let val = &bytes[self.pos..self.pos + vl];
            self.pos += vl;
            Ok((key, val))
        };
        Some(try_read())
    }
}

// ---------------------------------------------------------------------------
// Publish + restore (feature-gated; needs the live engine to read from)
// ---------------------------------------------------------------------------

#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
mod publish {
    use super::{
        encode_kv_record, idx_key, kv_key, manifest_key, KvSnapshotReader, Manifest, ObjectStore,
    };
    use crate::error::{Error, Result};
    use crate::shard::Shard;

    /// Publish the current state of `shard` as the next generation under the
    /// given `domain` prefix in `store`. Returns the new generation number.
    ///
    /// The new generation is added to the live manifest after every segment
    /// blob is successfully PUT, so a reader that sees the manifest is
    /// guaranteed to find every named segment present.
    ///
    /// # Errors
    /// Surfaces I/O and snapshot errors.
    pub fn publish_generation(shard: &Shard, store: &dyn ObjectStore, domain: &str) -> Result<u64> {
        // Decide gen number.
        let manifest_bytes = store.get(&manifest_key(domain))?.unwrap_or_default();
        let mut manifest = Manifest::parse(&manifest_bytes)?;
        let new_gen = manifest.latest().map_or(0, |m| m + 1);

        // KV snapshot.
        let mut kv_buf = Vec::new();
        for guard in shard.iter_raw() {
            let (k, v) = guard?;
            encode_kv_record(&mut kv_buf, &k, &v)?;
        }
        store.put(&kv_key(domain, new_gen), &kv_buf)?;

        // Per-index serialization.
        for ispec in &shard.spec().indexes {
            let buf = shard.snapshot_index_bytes(&ispec.name)?;
            store.put(&idx_key(domain, new_gen, &ispec.name), &buf)?;
        }

        // Manifest update LAST — only now is the generation observable.
        manifest.generations.push(new_gen);
        store.put(&manifest_key(domain), &manifest.encode())?;
        Ok(new_gen)
    }

    /// Restore the latest live generation from `store` into a freshly-opened
    /// shard. Tolerates GC-race failures: a missing segment is benign iff
    /// its generation has left the manifest by the time we re-check.
    ///
    /// # Errors
    /// Returns `ObjectStore` for hard failures (missing segment still listed
    /// in the manifest after re-check, or torn bytes).
    pub fn restore_into(shard: &Shard, store: &dyn ObjectStore, domain: &str) -> Result<()> {
        let load = |gen: u64| -> Result<()> {
            // KV snapshot.
            let kv = match store.get(&kv_key(domain, gen))? {
                Some(b) => b,
                None => return Err(Error::ObjectStore("kv snapshot missing".into())),
            };
            for rec in KvSnapshotReader::new(&kv) {
                let (k, v) = rec?;
                shard.put_raw_kv(k, v)?;
            }
            // Per-index.
            for ispec in &shard.spec().indexes {
                let buf = match store.get(&idx_key(domain, gen, &ispec.name))? {
                    Some(b) => b,
                    None => {
                        return Err(Error::ObjectStore(format!(
                            "idx snapshot missing for {}",
                            ispec.name
                        )))
                    }
                };
                shard.load_index_bytes(&ispec.name, &buf)?;
            }
            Ok(())
        };

        let manifest_bytes = match store.get(&manifest_key(domain))? {
            Some(b) => b,
            None => return Ok(()), // nothing published yet
        };
        let manifest = Manifest::parse(&manifest_bytes)?;
        let Some(gen) = manifest.latest() else {
            return Ok(());
        };

        match load(gen) {
            Ok(()) => Ok(()),
            Err(_e) => {
                // GC-race re-check: if the gen has left the (newly re-read)
                // manifest, the failure was a delete race and is tolerated.
                let manifest_now =
                    Manifest::parse(&store.get(&manifest_key(domain))?.unwrap_or_default())?;
                if !manifest_now.generations.contains(&gen) {
                    // Try latest of the new manifest (recurse one level).
                    if let Some(next_gen) = manifest_now.latest() {
                        return load(next_gen);
                    }
                    return Ok(());
                }
                Err(Error::ObjectStore(format!(
                    "hard restore failure on gen {gen} (still listed after re-check)"
                )))
            }
        }
    }

    /// Garbage-collect generations not in the most-recent `keep`. The
    /// manifest is rewritten FIRST so the generations become unreachable
    /// before any segment blob is deleted (mirrors shale).
    ///
    /// # Errors
    /// Surfaces I/O errors. Deletes are idempotent; a missing blob isn't an
    /// error.
    pub fn gc(shard: &Shard, store: &dyn ObjectStore, domain: &str, keep: usize) -> Result<()> {
        let manifest_bytes = store.get(&manifest_key(domain))?.unwrap_or_default();
        let mut manifest = Manifest::parse(&manifest_bytes)?;
        if manifest.generations.len() <= keep {
            return Ok(());
        }
        let cutoff = manifest.generations.len() - keep;
        let to_drop: Vec<u64> = manifest.generations.drain(..cutoff).collect();
        store.put(&manifest_key(domain), &manifest.encode())?;
        for gen in to_drop {
            store.delete(&kv_key(domain, gen))?;
            for ispec in &shard.spec().indexes {
                store.delete(&idx_key(domain, gen, &ispec.name))?;
            }
        }
        Ok(())
    }
}

#[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
pub use publish::{gc, publish_generation, restore_into};

#[cfg(feature = "coldtier-cloud")]
pub use crate::coldtier_cloud::CloudStore;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip() {
        let m = Manifest {
            generations: vec![40, 41, 42],
        };
        assert_eq!(Manifest::parse(&m.encode()).unwrap(), m);
        assert_eq!(Manifest::parse(b"").unwrap().generations, Vec::<u64>::new());
        assert_eq!(
            Manifest::parse(b" 1, 2,3 ").unwrap().generations,
            vec![1, 2, 3]
        );
        assert!(Manifest::parse(b"1,oops,3").is_err());
    }

    #[test]
    fn in_memory_store_basic_ops() {
        let s = InMemoryStore::new();
        assert!(s.get("k").unwrap().is_none());
        s.put("k", b"v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some(b"v".as_slice()));
        s.put("a/b/c", b"x").unwrap();
        s.put("a/b/d", b"y").unwrap();
        let mut keys = s.list("a/b/").unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a/b/c", "a/b/d"]);
        s.delete("k").unwrap();
        assert!(s.get("k").unwrap().is_none());
        s.delete("k").unwrap(); // idempotent
    }

    #[test]
    fn local_dir_store_atomic_put_no_torn_reads() {
        let tmp = tempfile::tempdir().unwrap();
        let s = LocalDirStore::open(tmp.path()).unwrap();
        s.put("a/b", b"hello").unwrap();
        s.put("a/b", b"world").unwrap();
        assert_eq!(s.get("a/b").unwrap().as_deref(), Some(b"world".as_slice()));
        assert!(s.list("a/").unwrap().contains(&"a/b".to_string()));
        s.delete("a/b").unwrap();
        assert!(s.get("a/b").unwrap().is_none());
    }

    #[test]
    fn kv_snapshot_round_trip() {
        let mut buf = Vec::new();
        encode_kv_record(&mut buf, b"k1", b"v1").unwrap();
        encode_kv_record(&mut buf, b"k2", b"v2-longer").unwrap();
        encode_kv_record(&mut buf, &[0u8; 0], &[0u8; 0]).unwrap();
        let recs: Vec<_> = KvSnapshotReader::new(&buf)
            .map(|r| r.unwrap())
            .map(|(k, v)| (k.to_vec(), v.to_vec()))
            .collect();
        assert_eq!(recs.len(), 3);
        assert_eq!(recs[0], (b"k1".to_vec(), b"v1".to_vec()));
        assert_eq!(recs[1], (b"k2".to_vec(), b"v2-longer".to_vec()));
        assert_eq!(recs[2], (vec![], vec![]));
    }

    #[test]
    fn kv_snapshot_truncation_is_error() {
        let mut buf = Vec::new();
        encode_kv_record(&mut buf, b"k1", b"v1").unwrap();
        // Drop the last 2 bytes.
        buf.truncate(buf.len() - 2);
        let err = KvSnapshotReader::new(&buf).find_map(|r| r.err());
        assert!(err.is_some());
    }
}
