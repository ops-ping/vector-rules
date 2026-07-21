//! Custom-metric round-trip. Proves the usearch `change_metric` Box<dyn Fn>
//! path is reachable from em-log-n with a caller-supplied Rust closure — the
//! "JIT-UDF" pattern usearch advertises (we don't need a JIT; a regular Rust
//! function compiled into the binary is fine).
//!
//! Two cases:
//! 1. A trivial custom metric returns a constant — `ann` must surface that
//!    same constant as the distance, proving the closure is actually called.
//! 2. A "fused" image+text metric (concatenated 4+4 vector, cosine-of-image
//!    + cosine-of-text) — proves a non-trivial caller composite works.

#![cfg(all(feature = "fjall-backend", feature = "usearch-backend"))]

use em_log_n::key::KeyBuilder;
use em_log_n::shard::{DomainId, IndexSpec, Metric, Shard, ShardSpec};

fn open(dim: usize) -> (tempfile::TempDir, Shard) {
    let tmp = tempfile::tempdir().unwrap();
    let spec = ShardSpec {
        domain: DomainId::new("custom-metric").unwrap(),
        indexes: vec![IndexSpec {
            name: "fused".into(),
            dim,
            metric: Metric::Custom,
        }],
    };
    let s = Shard::open(spec, tmp.path()).unwrap();
    (tmp, s)
}

#[test]
fn custom_metric_constant_proves_closure_is_invoked() {
    let (_tmp, shard) = open(4);

    // Register a metric that always returns 42.0. If usearch ever returns a
    // hit, its distance MUST be 42.0 — proves our closure ran.
    shard
        .register_custom_metric("fused", |_a: &[f32], _b: &[f32]| 42.0)
        .unwrap();

    // Need at least two vectors so search returns a non-self match.
    let v1 = vec![1.0f32, 0.0, 0.0, 0.0];
    let v2 = vec![0.0f32, 1.0, 0.0, 0.0];
    shard
        .put(
            &KeyBuilder::new(1, b"a").build(),
            b"a",
            &[("fused", v1.as_slice())],
        )
        .unwrap();
    shard
        .put(
            &KeyBuilder::new(2, b"b").build(),
            b"b",
            &[("fused", v2.as_slice())],
        )
        .unwrap();

    let q = vec![1.0f32, 0.0, 0.0, 0.0];
    let hits = shard.ann("fused", &q, 2).unwrap();
    assert!(!hits.is_empty());
    for h in hits {
        // usearch may report self-hit at 0 then any other hit at our 42.0;
        // we only require that at least one non-self hit reports 42.0.
        if h.distance > 0.0 {
            assert!((h.distance - 42.0).abs() < 1e-3, "got {}", h.distance);
            return;
        }
    }
    panic!("no non-self hit observed; custom metric never invoked");
}

#[test]
fn fused_image_text_metric() {
    // 8-dim vector: bytes 0..4 = image embedding, 4..8 = text embedding.
    // Custom metric = (1 - cos(image)) + (1 - cos(text)).
    let (_tmp, shard) = open(8);

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
    shard
        .register_custom_metric("fused", |a, b| {
            let image_sim = dot(&a[..4], &b[..4]);
            let text_sim = dot(&a[4..], &b[4..]);
            (1.0 - image_sim) + (1.0 - text_sim)
        })
        .unwrap();

    // Helper: synthesize an 8-dim fused vector from two normalized halves.
    fn fuse(img: [f32; 4], txt: [f32; 4]) -> Vec<f32> {
        let mut v = Vec::with_capacity(8);
        v.extend_from_slice(&img);
        v.extend_from_slice(&txt);
        v
    }

    let strong = fuse([1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]);
    let half = fuse([1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]);
    let weak = fuse([0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]);

    let strong_key = KeyBuilder::new(10, b"strong").build();
    let half_key = KeyBuilder::new(20, b"half").build();
    let weak_key = KeyBuilder::new(30, b"weak").build();
    shard
        .put(&strong_key, b"s", &[("fused", strong.as_slice())])
        .unwrap();
    shard
        .put(&half_key, b"h", &[("fused", half.as_slice())])
        .unwrap();
    shard
        .put(&weak_key, b"w", &[("fused", weak.as_slice())])
        .unwrap();

    let q = fuse([1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]);
    let hits = shard.ann("fused", &q, 3).unwrap();
    assert!(hits.len() >= 2);
    // Closest must be strong (distance ~0); second must be half (~1); weak (~2)
    // last. Verify ordering, not exact distances (HNSW is approximate).
    let order: Vec<_> = hits.iter().map(|h| h.row_key.clone()).collect();
    let strong_pos = order.iter().position(|k| k == &strong_key);
    let weak_pos = order.iter().position(|k| k == &weak_key);
    assert!(strong_pos.is_some(), "fused-strong must appear");
    if let (Some(s), Some(w)) = (strong_pos, weak_pos) {
        assert!(s < w, "strong must rank before weak under the fused metric");
    }
}

#[test]
fn register_custom_metric_unknown_index_errors() {
    let (_tmp, shard) = open(4);
    assert!(shard.register_custom_metric("nope", |_, _| 0.0).is_err());
}
