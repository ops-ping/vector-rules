//! Cloud object-store backend for em-log-n's cold tier.
//!
//! Wraps [Apache Arrow's `object_store`
//! crate](https://docs.rs/object_store/) (pure Rust: S3, Azure, GCS,
//! local, and in-memory under one trait) and bridges its async API to
//! em-log-n's sync [`ObjectStore`] trait, so cold-tier publish /
//! restore / GC work unchanged against cloud buckets.
//!
//! ## Why this design
//!
//! - **Public API stays sync.** em-log-n's callers (`Shard`,
//!   `CachingEmbedder`, ...) are sync and shouldn't be forced to wire
//!   a tokio runtime to use the cloud cold tier.
//! - **Bridge owns its runtime.** A `CloudStore` allocates a small
//!   `tokio::runtime::Runtime` (current-thread, default features off)
//!   and `block_on`s the underlying async operations. No global
//!   runtime, no leaked tokio handle, no risk of deadlocking a
//!   runtime the caller already owns.
//! - **Absent-vs-error contract.** `object_store::Error::NotFound` is
//!   mapped to `Ok(None)` (mirrors the
//!   [docs/cold-tier.md](https://github.com/ops-ping/em-log-n/blob/main/docs/cold-tier.md)
//!   contract every backend must implement).
//!
//! ## Quick start
//!
//! ```no_run
//! # #[cfg(all(feature = "fjall-backend", feature = "usearch-backend", feature = "coldtier-cloud"))]
//! # fn main() -> em_log_n::Result<()> {
//! use em_log_n::coldtier::{publish_generation, restore_into, CloudStore};
//! // ... open your Shard as usual ...
//! # let shard: em_log_n::shard::Shard = unimplemented!();
//! # let fresh: em_log_n::shard::Shard = unimplemented!();
//!
//! let store = CloudStore::s3("my-bucket", "us-east-1", "em-log-n/prod")?;
//! publish_generation(&shard, &store, "ui")?;
//!
//! // Later, on a different host:
//! restore_into(&fresh, &store, "ui")?;
//! # Ok(())
//! # }
//! # #[cfg(not(all(feature = "fjall-backend", feature = "usearch-backend", feature = "coldtier-cloud")))]
//! # fn main() {}
//! ```
//!
//! For non-standard providers (LocalStack, MinIO, custom endpoints,
//! Cloudflare R2), build the underlying `object_store::ObjectStore`
//! impl yourself and hand it to [`CloudStore::from_object_store`].
//!
//! The crate-level `pub mod coldtier_cloud` is already `#[cfg]`-gated on
//! `coldtier-cloud`, so this file needs no inner `cfg`.

use std::sync::Arc;

use futures::StreamExt;
use object_store::path::Path as OsPath;
use object_store::{
    aws::AmazonS3Builder, azure::MicrosoftAzureBuilder, gcp::GoogleCloudStorageBuilder,
};
use object_store::{Error as OsError, ObjectStore as ArrowObjectStore, ObjectStoreExt, PutPayload};
use tokio::runtime::{Builder as RuntimeBuilder, Runtime};

use crate::coldtier::ObjectStore;
use crate::error::{Error, Result};

/// Cloud-backed [`ObjectStore`] over Arrow's `object_store` crate.
///
/// Construct via [`CloudStore::s3`], [`CloudStore::azure`],
/// [`CloudStore::gcs`], [`CloudStore::from_url`], or
/// [`CloudStore::from_object_store`].
pub struct CloudStore {
    inner: Arc<dyn ArrowObjectStore>,
    /// Optional prefix prepended to every key (so a single bucket can
    /// host multiple em-log-n deployments).
    root: OsPath,
    /// Dedicated current-thread tokio runtime used for `block_on`. The
    /// runtime is single-threaded by choice — em-log-n's cold-tier
    /// publish/restore are not contended; the runtime exists only to
    /// drive `object_store`'s async API.
    rt: Runtime,
}

impl CloudStore {
    /// Construct from any [`ArrowObjectStore`] impl. `prefix` is
    /// appended to the store's root for every key (use `""` for none).
    ///
    /// # Errors
    /// Returns `ObjectStore` if `prefix` is not a valid object-store path.
    pub fn from_object_store(inner: Arc<dyn ArrowObjectStore>, prefix: &str) -> Result<Self> {
        let root = parse_prefix(prefix)?;
        let rt = build_runtime()?;
        Ok(Self { inner, root, rt })
    }

    /// AWS S3 with default credential resolution. `prefix` is the path
    /// inside the bucket (use `""` for the bucket root).
    ///
    /// # Errors
    /// Returns `ObjectStore` if the builder fails or `prefix` is invalid.
    pub fn s3(bucket: &str, region: &str, prefix: &str) -> Result<Self> {
        let store = AmazonS3Builder::from_env()
            .with_bucket_name(bucket)
            .with_region(region)
            .build()
            .map_err(|e| Error::ObjectStore(format!("AmazonS3Builder: {e}")))?;
        Self::from_object_store(Arc::new(store), prefix)
    }

    /// Azure Blob Storage.
    ///
    /// # Errors
    /// Returns `ObjectStore` if the builder fails or `prefix` is invalid.
    pub fn azure(account: &str, container: &str, prefix: &str) -> Result<Self> {
        let store = MicrosoftAzureBuilder::from_env()
            .with_account(account)
            .with_container_name(container)
            .build()
            .map_err(|e| Error::ObjectStore(format!("MicrosoftAzureBuilder: {e}")))?;
        Self::from_object_store(Arc::new(store), prefix)
    }

    /// Google Cloud Storage.
    ///
    /// # Errors
    /// Returns `ObjectStore` if the builder fails or `prefix` is invalid.
    pub fn gcs(bucket: &str, prefix: &str) -> Result<Self> {
        let store = GoogleCloudStorageBuilder::from_env()
            .with_bucket_name(bucket)
            .build()
            .map_err(|e| Error::ObjectStore(format!("GoogleCloudStorageBuilder: {e}")))?;
        Self::from_object_store(Arc::new(store), prefix)
    }

    /// Build from a provider URL such as `s3://bucket/path`,
    /// `az://account/container/path`, `gs://bucket/path`,
    /// `file:///some/dir`, or `memory:///`. The path component becomes
    /// the prefix.
    ///
    /// # Errors
    /// Returns `ObjectStore` if URL parsing fails.
    pub fn from_url(url: &str) -> Result<Self> {
        let parsed =
            url::Url::parse(url).map_err(|e| Error::ObjectStore(format!("url parse: {e}")))?;
        let (store, path) = object_store::parse_url(&parsed)
            .map_err(|e| Error::ObjectStore(format!("object_store::parse_url: {e}")))?;
        let rt = build_runtime()?;
        Ok(Self {
            inner: Arc::from(store),
            root: path,
            rt,
        })
    }

    fn full_path(&self, key: &str) -> Result<OsPath> {
        if self.root.as_ref().is_empty() {
            OsPath::parse(key).map_err(|e| Error::ObjectStore(format!("bad key {key:?}: {e}")))
        } else {
            let joined = format!("{}/{}", self.root.as_ref(), key);
            OsPath::parse(&joined)
                .map_err(|e| Error::ObjectStore(format!("bad key {joined:?}: {e}")))
        }
    }
}

impl ObjectStore for CloudStore {
    fn put(&self, key: &str, bytes: &[u8]) -> Result<()> {
        let path = self.full_path(key)?;
        let payload = PutPayload::from(bytes.to_vec());
        self.rt
            .block_on(async { self.inner.put(&path, payload).await })
            .map_err(map_err)?;
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let path = self.full_path(key)?;
        let result = self.rt.block_on(async { self.inner.get(&path).await });
        match result {
            Ok(get_result) => {
                let bytes = self
                    .rt
                    .block_on(async { get_result.bytes().await })
                    .map_err(map_err)?;
                Ok(Some(bytes.to_vec()))
            }
            Err(OsError::NotFound { .. }) => Ok(None),
            Err(e) => Err(map_err(e)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        // Build the full prefix the way put/get do, but as a Path; an
        // empty prefix means "everything under root".
        let path = if prefix.is_empty() {
            self.root.clone()
        } else {
            self.full_path(prefix)?
        };
        let inner = Arc::clone(&self.inner);
        let entries: Vec<object_store::ObjectMeta> = self
            .rt
            .block_on(async move {
                let mut stream = inner.list(Some(&path));
                let mut out = Vec::new();
                while let Some(item) = stream.next().await {
                    match item {
                        Ok(meta) => out.push(meta),
                        Err(e) => return Err(e),
                    }
                }
                Ok::<_, OsError>(out)
            })
            .map_err(map_err)?;

        // Strip the configured root so callers see keys relative to
        // their put/get/delete namespace.
        let strip = if self.root.as_ref().is_empty() {
            String::new()
        } else {
            format!("{}/", self.root.as_ref())
        };
        Ok(entries
            .into_iter()
            .map(|m| {
                let s = m.location.as_ref().to_string();
                if strip.is_empty() {
                    s
                } else {
                    s.strip_prefix(&strip).unwrap_or(&s).to_string()
                }
            })
            .collect())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let path = self.full_path(key)?;
        let result = self.rt.block_on(async { self.inner.delete(&path).await });
        match result {
            Ok(()) => Ok(()),
            Err(OsError::NotFound { .. }) => Ok(()), // idempotent
            Err(e) => Err(map_err(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn parse_prefix(prefix: &str) -> Result<OsPath> {
    if prefix.is_empty() {
        return Ok(OsPath::from(""));
    }
    OsPath::parse(prefix).map_err(|e| Error::ObjectStore(format!("bad prefix {prefix:?}: {e}")))
}

fn build_runtime() -> Result<Runtime> {
    RuntimeBuilder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::ObjectStore(format!("build tokio runtime: {e}")))
}

fn map_err(e: OsError) -> Error {
    Error::ObjectStore(format!("object_store: {e}"))
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use object_store::memory::InMemory;

    fn open_inmemory(prefix: &str) -> CloudStore {
        CloudStore::from_object_store(Arc::new(InMemory::new()), prefix).unwrap()
    }

    #[test]
    fn put_get_round_trip() {
        let s = open_inmemory("");
        assert!(s.get("k").unwrap().is_none());
        s.put("k", b"hello").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some(b"hello".as_slice()));
    }

    #[test]
    fn put_get_with_prefix() {
        let s = open_inmemory("em-log-n/test");
        s.put("a/b", b"x").unwrap();
        assert_eq!(s.get("a/b").unwrap().as_deref(), Some(b"x".as_slice()));
        // A bare InMemory get without the prefix path should NOT see it
        // through `s`, but if we ask via `s.list("")` we should find it.
        let mut keys = s.list("").unwrap();
        keys.sort();
        assert_eq!(keys, vec!["a/b"]);
    }

    #[test]
    fn missing_key_returns_none_not_error() {
        let s = open_inmemory("p");
        assert!(s.get("never-written").unwrap().is_none());
    }

    #[test]
    fn delete_is_idempotent() {
        let s = open_inmemory("p");
        s.put("k", b"v").unwrap();
        s.delete("k").unwrap();
        s.delete("k").unwrap(); // second delete must not fail
        assert!(s.get("k").unwrap().is_none());
    }

    #[test]
    fn list_only_returns_matching_prefix() {
        let s = open_inmemory("p");
        s.put("a/1", b"x").unwrap();
        s.put("a/2", b"y").unwrap();
        s.put("b/1", b"z").unwrap();
        let mut a = s.list("a/").unwrap();
        a.sort();
        assert_eq!(a, vec!["a/1", "a/2"]);
        let mut b = s.list("b/").unwrap();
        b.sort();
        assert_eq!(b, vec!["b/1"]);
    }

    #[test]
    fn put_overwrites() {
        let s = open_inmemory("");
        s.put("k", b"one").unwrap();
        s.put("k", b"two").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some(b"two".as_slice()));
    }

    #[test]
    fn from_url_memory() {
        let s = CloudStore::from_url("memory:///").unwrap();
        s.put("a", b"hi").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some(b"hi".as_slice()));
    }
}
