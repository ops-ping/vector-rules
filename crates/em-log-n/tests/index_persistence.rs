//! The usearch ANN index persists across shard reopen (save on write, load on
//! open) — so vectors survive a restart without re-embedding.

#![cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]

use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};

fn spec(dim: usize) -> ShardSpec {
    ShardSpec {
        domain: DomainId::new("persist-test").unwrap(),
        indexes: vec![IndexSpec {
            name: "v".into(),
            dim,
            metric: Metric::Cosine,
        }],
    }
}

#[test]
fn usearch_index_persists_across_reopen() {
    let tmp = tempfile::tempdir().unwrap();
    let v1 = vec![1.0f32, 0.0, 0.0, 0.0];
    let v2 = vec![0.0f32, 1.0, 0.0, 0.0];

    // Write two distinct vectors, then drop the shard (simulating a restart).
    {
        let s = Shard::open(spec(4), tmp.path()).unwrap();
        s.put(
            &KeyBuilder::new(1, b"one").build(),
            b"one",
            &[("v", v1.as_slice())],
        )
        .unwrap();
        s.put(
            &KeyBuilder::new(2, b"two").build(),
            b"two",
            &[("v", v2.as_slice())],
        )
        .unwrap();
        assert_eq!(
            s.index_len("v").unwrap(),
            2,
            "two vectors indexed before drop"
        );
        assert_eq!(
            s.ann("v", &v2, 2).unwrap().len(),
            2,
            "both searchable before reopen"
        );
    }

    // Reopen the SAME path. The index must reload from its persisted file — no
    // re-put, no re-embedding.
    let s = Shard::open(spec(4), tmp.path()).unwrap();
    assert_eq!(
        s.index_len("v").unwrap(),
        2,
        "index reloaded with BOTH vectors"
    );
    let hits = s.ann("v", &v2, 2).unwrap();
    assert_eq!(
        hits.len(),
        2,
        "both vectors searchable after reopen (persisted)"
    );
}

/// The live daemon's path: write, restart (load), write ANOTHER, restart (load).
/// A vector added AFTER a load must coexist with the loaded ones across the next
/// reopen.
#[test]
fn vector_added_after_load_persists() {
    let tmp = tempfile::tempdir().unwrap();
    let v1 = vec![1.0f32, 0.0, 0.0, 0.0];
    let v2 = vec![0.0f32, 1.0, 0.0, 0.0];

    // Session 1: write v1, drop.
    {
        let s = Shard::open(spec(4), tmp.path()).unwrap();
        s.put(
            &KeyBuilder::new(1, b"one").build(),
            b"one",
            &[("v", v1.as_slice())],
        )
        .unwrap();
    }
    // Session 2: LOAD v1, then add v2, drop.
    {
        let s = Shard::open(spec(4), tmp.path()).unwrap();
        assert_eq!(s.index_len("v").unwrap(), 1, "v1 loaded");
        s.put(
            &KeyBuilder::new(2, b"two").build(),
            b"two",
            &[("v", v2.as_slice())],
        )
        .unwrap();
        assert_eq!(
            s.index_len("v").unwrap(),
            2,
            "v1 (loaded) + v2 (added) both present"
        );
    }
    // Session 3: LOAD both — v2 (added after a load) must have survived.
    let s = Shard::open(spec(4), tmp.path()).unwrap();
    assert_eq!(
        s.index_len("v").unwrap(),
        2,
        "both vectors reload after add-after-load"
    );
    let near_v2 = s.ann("v", &v2, 2).unwrap();
    assert_eq!(near_v2.len(), 2, "both searchable");
    // v2's own row must be retrievable as the nearest to v2.
    let near_v1 = s.ann("v", &v1, 2).unwrap();
    assert_eq!(near_v1.len(), 2, "both searchable from v1's side too");
}
