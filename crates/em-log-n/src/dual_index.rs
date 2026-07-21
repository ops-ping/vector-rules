//! Dual coarse/detail index pairing for one logical field.
//!
//! The pattern that lets canonicalization maximize cache hits **without**
//! compromising recall: store two usearch indexes per field and combine them in
//! a single [`Shard::ann_multi`] scan.
//!
//! - `<field>__canon` — the **coarse** index: embedding of the *canonical* form
//!   (see [`vrules_canon`] / [`crate::embed::CanonicalizingEmbedder`]). High
//!   cache-hit rate; matches recurring patterns.
//! - `<field>` — the **detail** index: embedding of the *raw* text. Precise
//!   matches. Populated off the hot path via [`Shard::add_vector`]
//!   (see [`crate::detail_backfill`]).
//!
//! A query embeds the request both ways and asks both indexes at once;
//! [`Combine::UnionMin`] keeps, for each row, its best distance under *either*
//! index — so a hit in the coarse index is available immediately while the
//! detail index is still warming up, and precise hits take over once backfilled.

use crate::error::Result;
use crate::key::RowKey;
use crate::shard::{AnnHit, Combine, IndexSpec, Metric, Shard};

/// Suffix appended to a field name to form its coarse/canonical index.
pub const CANON_SUFFIX: &str = "__canon";

/// A coarse/detail index pair for one logical field.
#[derive(Debug, Clone)]
pub struct DualIndex {
    /// Detail (raw-text) index name — equals the logical field name.
    pub detail: String,
    /// Coarse (canonical) index name — `<field>__canon`.
    pub canon: String,
}

impl DualIndex {
    /// Build the pair for a logical `field`.
    #[must_use]
    pub fn new(field: impl Into<String>) -> Self {
        let detail = field.into();
        let canon = format!("{detail}{CANON_SUFFIX}");
        Self { detail, canon }
    }

    /// The two [`IndexSpec`]s to register in a [`ShardSpec`](crate::shard::ShardSpec):
    /// detail first, then canon. Both share `dim` and `metric`.
    #[must_use]
    pub fn index_specs(&self, dim: usize, metric: Metric) -> [IndexSpec; 2] {
        [
            IndexSpec {
                name: self.detail.clone(),
                dim,
                metric,
            },
            IndexSpec {
                name: self.canon.clone(),
                dim,
                metric,
            },
        ]
    }

    /// Write a row plus its **canonical** vector into the coarse index only.
    /// The detail vector is expected to arrive later via
    /// [`backfill_detail`](Self::backfill_detail).
    ///
    /// # Errors
    /// Surfaces [`Shard::put`] errors.
    pub fn put_canon(
        &self,
        shard: &Shard,
        row_key: &RowKey,
        value_bytes: &[u8],
        canon_vec: &[f32],
    ) -> Result<()> {
        shard.put(row_key, value_bytes, &[(self.canon.as_str(), canon_vec)])
    }

    /// Add the **detail** (raw) vector for an already-written row into the fine
    /// index. Idempotent at the index level (re-adding overwrites the key).
    ///
    /// # Errors
    /// Surfaces [`Shard::add_vector`] errors (including "row does not exist").
    pub fn backfill_detail(
        &self,
        shard: &Shard,
        row_key: &RowKey,
        detail_vec: &[f32],
    ) -> Result<()> {
        shard.add_vector(row_key, &self.detail, detail_vec)
    }

    /// Query both indexes in one scan and union by best distance.
    ///
    /// `q_canon` = embedding of the canonicalized query; `q_detail` = embedding
    /// of the raw query. Each index is probed for `k` candidates; the merged set
    /// is truncated to `final_k`.
    ///
    /// # Errors
    /// Surfaces [`Shard::ann_multi`] errors.
    pub fn query(
        &self,
        shard: &Shard,
        q_canon: &[f32],
        q_detail: &[f32],
        k: usize,
        final_k: usize,
    ) -> Result<Vec<AnnHit>> {
        shard.ann_multi(
            &[
                (self.canon.as_str(), q_canon, k),
                (self.detail.as_str(), q_detail, k),
            ],
            Combine::UnionMin,
            final_k,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_convention() {
        let d = DualIndex::new("text");
        assert_eq!(d.detail, "text");
        assert_eq!(d.canon, "text__canon");
    }

    #[test]
    fn index_specs_share_dim_and_metric() {
        let d = DualIndex::new("text");
        let [detail, canon] = d.index_specs(8, Metric::Cosine);
        assert_eq!(detail.name, "text");
        assert_eq!(canon.name, "text__canon");
        assert_eq!(detail.dim, 8);
        assert_eq!(canon.dim, 8);
    }
}
