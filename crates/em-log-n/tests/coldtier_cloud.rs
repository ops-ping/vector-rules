//! Re-runs the cold-tier publish/restore/gc/race scenarios from
//! `tests/coldtier.rs` against the `CloudStore` cloud bridge backed by
//! `object_store::memory::InMemory`. Proves the bridge is
//! semantics-equivalent to em-log-n's native `InMemoryStore`.

#![cfg(all(
    feature = "fjall-backend",
    feature = "usearch-backend",
    feature = "coldtier-cloud",
))]

use std::sync::Arc;

use em_log_n::coldtier::{gc, publish_generation, restore_into, CloudStore, Manifest, ObjectStore};
use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};
use object_store::memory::InMemory;

fn norm(mut v: Vec<f32>) -> Vec<f32> {
    let m = v.iter().fold(0.0f32, |a, &x| x.mul_add(x, a)).sqrt();
    if m > 0.0 {
        for x in &mut v {
            *x /= m;
        }
    }
    v
}

fn spec(dim: usize) -> ShardSpec {
    ShardSpec {
        domain: DomainId::new("logs").unwrap(),
        indexes: vec![IndexSpec {
            name: "text".into(),
            dim,
            metric: Metric::Cosine,
        }],
    }
}

fn open_shard(dim: usize) -> (tempfile::TempDir, Shard) {
    let tmp = tempfile::tempdir().unwrap();
    let shard = Shard::open(spec(dim), tmp.path()).unwrap();
    (tmp, shard)
}

fn cloud_store() -> CloudStore {
    CloudStore::from_object_store(Arc::new(InMemory::new()), "em-log-n").unwrap()
}

#[test]
fn publish_restore_round_trip_via_cloudstore() {
    let dim = 8;
    let (_tmp_a, shard_a) = open_shard(dim);

    let v1 = norm(vec![1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let v2 = norm(vec![0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
    let k1 = KeyBuilder::new(100, b"alpha").build();
    let k2 = KeyBuilder::new(200, b"beta").build();
    shard_a
        .put(&k1, b"payload-1", &[("text", v1.as_slice())])
        .unwrap();
    shard_a
        .put(&k2, b"payload-2", &[("text", v2.as_slice())])
        .unwrap();

    let store = cloud_store();
    let gen = publish_generation(&shard_a, &store, "logs").unwrap();
    assert_eq!(gen, 0);

    let (_tmp_b, shard_b) = open_shard(dim);
    restore_into(&shard_b, &store, "logs").unwrap();

    assert_eq!(
        shard_b.get(&k1).unwrap().as_deref(),
        Some(b"payload-1".as_slice())
    );
    assert_eq!(
        shard_b.get(&k2).unwrap().as_deref(),
        Some(b"payload-2".as_slice())
    );
    let scanned = shard_b.scan(10).unwrap();
    assert_eq!(scanned.len(), 2);
    assert_eq!(scanned[0].0, k2);
    let hits = shard_b.ann("text", &v1, 1).unwrap();
    assert_eq!(hits[0].row_key, k1);
}

#[test]
fn publish_then_gc_via_cloudstore() {
    let dim = 4;
    let (_tmp, shard) = open_shard(dim);
    let store = cloud_store();

    for i in 0..5u64 {
        let k = KeyBuilder::new(i + 1, format!("e{i}").as_bytes()).build();
        let v = norm(vec![i as f32, 1.0, 0.0, 0.0]);
        shard
            .put(&k, format!("v{i}").as_bytes(), &[("text", v.as_slice())])
            .unwrap();
        let gen = publish_generation(&shard, &store, "logs").unwrap();
        assert_eq!(gen, i);
    }

    gc(&shard, &store, "logs", 2).unwrap();
    let manifest_bytes = store.get("logs/manifest").unwrap().unwrap_or_default();
    let manifest = Manifest::parse(&manifest_bytes).unwrap();
    assert_eq!(manifest.generations, vec![3, 4]);
    for g in 0..=2u64 {
        let kv_key = format!("logs/gen/{g:08}/kv.snapshot");
        assert!(
            store.get(&kv_key).unwrap().is_none(),
            "gen {g} kv survived gc"
        );
    }
}

#[test]
fn restore_tolerates_gc_race_via_cloudstore() {
    let dim = 4;
    let (_tmp, shard) = open_shard(dim);
    let store = cloud_store();

    for i in 0..3u64 {
        let k = KeyBuilder::new(i + 1, format!("e{i}").as_bytes()).build();
        let v = norm(vec![i as f32, 1.0, 0.0, 0.0]);
        shard
            .put(&k, format!("v{i}").as_bytes(), &[("text", v.as_slice())])
            .unwrap();
        publish_generation(&shard, &store, "logs").unwrap();
    }

    // Drop gen 2's KV blob and rewind the manifest. Mirrors
    // tests/coldtier.rs::restore_tolerates_gc_race_on_old_generation.
    store.delete("logs/gen/00000002/kv.snapshot").unwrap();
    store
        .put(
            "logs/manifest",
            &Manifest {
                generations: vec![0, 1],
            }
            .encode(),
        )
        .unwrap();

    let (_tmp_b, shard_b) = open_shard(dim);
    restore_into(&shard_b, &store, "logs").unwrap();
    assert!(shard_b.scan(10).unwrap().len() >= 2);
}

#[test]
fn restore_errors_on_missing_segment_still_listed_via_cloudstore() {
    let dim = 4;
    let (_tmp, shard) = open_shard(dim);
    let store = cloud_store();
    let k = KeyBuilder::new(1, b"e").build();
    let v = norm(vec![1.0, 0.0, 0.0, 0.0]);
    shard.put(&k, b"v", &[("text", v.as_slice())]).unwrap();
    publish_generation(&shard, &store, "logs").unwrap();

    store.delete("logs/gen/00000000/kv.snapshot").unwrap();
    let (_tmp_b, shard_b) = open_shard(dim);
    let err = restore_into(&shard_b, &store, "logs");
    assert!(
        err.is_err(),
        "missing segment still listed must be hard error"
    );
}

#[test]
fn restore_no_manifest_is_no_op_via_cloudstore() {
    let dim = 4;
    let (_tmp_a, shard) = open_shard(dim);
    let store = cloud_store();
    restore_into(&shard, &store, "logs").unwrap();
    assert_eq!(shard.scan(10).unwrap().len(), 0);
}
