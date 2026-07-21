//! Top-level [`Store`] = a collection of per-domain shards sharing one
//! base directory and one shale-style cold-tier bucket.
//!
//! Phase-2 scaffold: registers and tracks [`ShardSpec`]s. The functional
//! [`Store::open_shard`] (which actually opens a backed [`Shard`]) is gated
//! behind `--features fjall-backend,usearch-backend`. The cold-tier mirror
//! arrives in `phase3-object-store`.

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::shard::{DomainId, ShardSpec};

/// Top-level store. Holds one [`ShardSpec`] per registered domain; opens
/// backed shards on demand.
#[derive(Debug, Default)]
pub struct Store {
    specs: HashMap<DomainId, ShardSpec>,
}

impl Store {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a domain. Returns the registered [`ShardSpec`] for chaining.
    ///
    /// # Errors
    /// Returns `Invariant` if the domain is already registered.
    pub fn register(&mut self, spec: ShardSpec) -> Result<&ShardSpec> {
        let id = spec.domain.clone();
        if self.specs.contains_key(&id) {
            return Err(Error::Invariant("domain already registered"));
        }
        self.specs.insert(id.clone(), spec);
        Ok(self.specs.get(&id).expect("just inserted"))
    }

    /// Look up a registered spec.
    #[must_use]
    pub fn spec(&self, domain: &DomainId) -> Option<&ShardSpec> {
        self.specs.get(domain)
    }

    /// Number of registered domains.
    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// Whether any domain is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// Open the backed [`Shard`](crate::shard::Shard) for a registered
    /// domain under `base_dir/<domain>/`. Available only with the
    /// `fjall-backend` and `usearch-backend` features.
    ///
    /// # Errors
    /// `Invariant` if the domain isn't registered; otherwise propagates the
    /// shard's open error.
    #[cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]
    pub fn open_shard(
        &self,
        domain: &DomainId,
        base_dir: impl AsRef<std::path::Path>,
    ) -> Result<crate::shard::Shard> {
        let spec = self
            .specs
            .get(domain)
            .ok_or(Error::Invariant("domain not registered"))?
            .clone();
        let path = base_dir.as_ref().join(&domain.0);
        crate::shard::Shard::open(spec, path)
    }

    /// Open (or create) a content-addressed embedding cache under
    /// `base_dir/_embed_cache/`. The cache is shared across all
    /// domains in this Store; namespace isolation by `model_id` is
    /// enforced inside [`crate::embed_cache::EmbedCache`].
    ///
    /// # Errors
    /// Surfaces fjall errors.
    #[cfg(feature = "fjall-backend")]
    pub fn open_embed_cache(
        &self,
        base_dir: impl AsRef<std::path::Path>,
        model_id: impl Into<String>,
        dim: usize,
    ) -> Result<crate::embed_cache::EmbedCache> {
        let path = base_dir.as_ref().join("_embed_cache");
        crate::embed_cache::EmbedCache::open(path, model_id, dim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shard::{IndexSpec, Metric};

    fn spec(name: &str) -> ShardSpec {
        ShardSpec {
            domain: DomainId::new(name).unwrap(),
            indexes: vec![IndexSpec {
                name: "text".into(),
                dim: 768,
                metric: Metric::Cosine,
            }],
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut s = Store::new();
        s.register(spec("ui")).unwrap();
        s.register(spec("system-core")).unwrap();
        assert_eq!(s.len(), 2);
        assert!(s.spec(&DomainId::new("ui").unwrap()).is_some());
        assert!(s.spec(&DomainId::new("missing").unwrap()).is_none());
    }

    #[test]
    fn duplicate_registration_rejected() {
        let mut s = Store::new();
        s.register(spec("ui")).unwrap();
        assert!(s.register(spec("ui")).is_err());
    }

    #[cfg(feature = "fjall-backend")]
    #[test]
    fn open_embed_cache_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new();
        let cache = store.open_embed_cache(tmp.path(), "test-model", 4).unwrap();
        cache.put("hello", &[0.5, 0.25, 0.125, 0.0625]).unwrap();
        let v = cache.get("hello").unwrap().unwrap();
        assert_eq!(v.len(), 4);
    }
}
