//! Cold-tier round-trip + GC-race tests.
//!
//! Phase-3 deliverables:
//! 1. publish → restore round-trip preserves KV rows AND vector index hits.
//! 2. publish n times → GC keeping k yields the most-recent k live.
//! 3. GC-race tolerance: a missing segment whose generation has left the
//!    manifest is benign; a missing segment whose generation IS still listed
//!    is a hard error.

#![cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]

use em_log_n::coldtier::{
    gc, publish_generation, restore_into, InMemoryStore, Manifest, ObjectStore,
};
use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};

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

#[test]
fn publish_restore_round_trip_preserves_kv_and_ann() {
    let dim = 8;
    let (_tmp_a, shard_a) = open_shard(dim);

    // Populate.
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

    // Publish to cold tier.
    let store = InMemoryStore::new();
    let gen = publish_generation(&shard_a, &store, "logs").unwrap();
    assert_eq!(gen, 0);

    // Open a fresh shard in a fresh directory, restore from cold tier.
    let (_tmp_b, shard_b) = open_shard(dim);
    restore_into(&shard_b, &store, "logs").unwrap();

    // KV: point gets + newest-first scan.
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
    assert_eq!(scanned[0].0, k2); // newer first

    // ANN: query for v1 should land on k1.
    let hits = shard_b.ann("text", &v1, 1).unwrap();
    assert_eq!(hits[0].row_key, k1);
}

#[test]
fn publish_then_gc_keeps_most_recent_n() {
    let dim = 4;
    let (_tmp, shard) = open_shard(dim);
    let store = InMemoryStore::new();

    // Publish 5 generations, mutating between each.
    for i in 0..5u64 {
        let k = KeyBuilder::new(i + 1, format!("e{i}").as_bytes()).build();
        let v = norm(vec![i as f32, 1.0, 0.0, 0.0]);
        shard
            .put(&k, format!("v{i}").as_bytes(), &[("text", v.as_slice())])
            .unwrap();
        let gen = publish_generation(&shard, &store, "logs").unwrap();
        assert_eq!(gen, i);
    }

    // GC keep=2 should leave gens [3, 4] live and delete 0..=2.
    gc(&shard, &store, "logs", 2).unwrap();
    let manifest =
        Manifest::parse(&store.get("logs/manifest").unwrap().unwrap_or_default()).unwrap();
    assert_eq!(manifest.generations, vec![3, 4]);
    // Segments for dropped generations are gone.
    for g in 0..=2 {
        let kv_key = format!("logs/gen/{g:08}/kv.snapshot");
        assert!(
            store.get(&kv_key).unwrap().is_none(),
            "gen {g} kv survived gc"
        );
    }
}

#[test]
fn restore_tolerates_gc_race_on_old_generation() {
    let dim = 4;
    let (_tmp, shard) = open_shard(dim);
    let store = InMemoryStore::new();

    // Publish 3 generations.
    for i in 0..3u64 {
        let k = KeyBuilder::new(i + 1, format!("e{i}").as_bytes()).build();
        let v = norm(vec![i as f32, 1.0, 0.0, 0.0]);
        shard
            .put(&k, format!("v{i}").as_bytes(), &[("text", v.as_slice())])
            .unwrap();
        publish_generation(&shard, &store, "logs").unwrap();
    }

    // Simulate a GC-race: reader grabs the manifest just before gen 2's KV
    // blob is deleted. We model this by:
    //   1. Reading the manifest (records [0, 1, 2]).
    //   2. Deleting gen 2's kv.snapshot.
    //   3. Updating the manifest to [0, 1] (gen 2 left the live window).
    //   4. Calling restore_into — the loader will try gen 2 first
    //      (from a re-fetched manifest, [0,1]) and fall back to gen 1.
    // The shale-style tolerance check is: missing-segment whose gen has left
    // the manifest is benign. We assert that restore_into succeeds.
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

    // After restore we should see gens 0 and 1's entries.
    assert!(shard_b.scan(10).unwrap().len() >= 2);
}

#[test]
fn restore_errors_on_missing_segment_still_listed() {
    let dim = 4;
    let (_tmp, shard) = open_shard(dim);
    let store = InMemoryStore::new();
    let k = KeyBuilder::new(1, b"e").build();
    let v = norm(vec![1.0, 0.0, 0.0, 0.0]);
    shard.put(&k, b"v", &[("text", v.as_slice())]).unwrap();
    publish_generation(&shard, &store, "logs").unwrap();

    // Delete the KV blob but leave the manifest intact (a real hard error).
    store.delete("logs/gen/00000000/kv.snapshot").unwrap();
    let (_tmp_b, shard_b) = open_shard(dim);
    let err = restore_into(&shard_b, &store, "logs");
    assert!(
        err.is_err(),
        "missing segment still listed must be hard error"
    );
}

#[test]
fn restore_no_manifest_is_no_op() {
    let dim = 4;
    let (_tmp_a, shard) = open_shard(dim);
    let store = InMemoryStore::new();
    // Nothing published — restore should be a no-op.
    restore_into(&shard, &store, "logs").unwrap();
    assert_eq!(shard.scan(10).unwrap().len(), 0);
}
